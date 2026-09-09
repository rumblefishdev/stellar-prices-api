#!/usr/bin/env node
/**
 * Assert the published OpenAPI document and the deployed API Gateway describe
 * the SAME set of routes, in both directions.
 *
 * WHY THIS EXISTS
 * ---------------
 * Task 0124 exists because `/api-docs-json` sat in the axum router for months
 * while API Gateway never mapped it — the spec documented a route no caller
 * could reach, and nothing failed. The unit test in
 * `packages/prices-api/tests/openapi.rs` guards the spec against a hand-written
 * list, which catches most drift but is itself a copy of the CDK source, and
 * this repo has already been bitten three times by hand-maintained mirrors
 * (see `tools/scripts/lambda-assets.sh` and task 0077).
 *
 * So this derives BOTH sides from the artifacts that actually decide them:
 *
 *   - routes: the synthesized CloudFormation template (what CDK will deploy)
 *   - paths:  the extracted OpenAPI document (what the handler will serve)
 *
 * Add a route to the gateway and forget to document it — or document one the
 * gateway never maps — and this fails, in CI, before either reaches a reader.
 *
 * Usage:
 *   npm run infra:synth:production   # → infra/cdk.out/*.template.json
 *   npm run openapi:verify-routes    # re-extracts the document, then compares
 *
 * `npm run openapi:verify-routes` re-runs the extraction itself, so the
 * document compared is always the current one. Invoking this file directly
 * skips that and compares whatever target/openapi.json happens to hold — a
 * stale file from another branch reads as a pass, or as drift that cannot be
 * reproduced. The synthesized template is still the caller's responsibility.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const templatePath = join(
  repoRoot,
  'infra',
  'cdk.out',
  'Prices-production-ApiGateway.template.json',
);
const specPath = join(repoRoot, 'target', 'openapi.json');

function readJson(path, hint) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (err) {
    console.error(`error: cannot read ${path}\n  ${err.message}\n  ${hint}`);
    process.exit(1);
  }
}

const template = readJson(
  templatePath,
  'run `npm run infra:synth:production` first',
);
const spec = readJson(specPath, 'run `npm run openapi:extract` first');

// ---------------------------------------------------------------------------
// Gateway side: rebuild each resource's full path by walking ParentId up to the
// RestApi root, then attach every method declared on it.
// ---------------------------------------------------------------------------
const resources = Object.entries(template.Resources ?? {});

/**
 * Classify a `ParentId` property into one of three cases. The distinction
 * matters: an earlier revision collapsed "root" and "anything I don't
 * understand" into the same `null`, and `null` is the walk's "path is
 * complete" signal — so an unrecognized parent silently produced a *truncated*
 * path (`/status` for `/v1/backfill/status`) instead of failing. That is the
 * one outcome this script must never produce, because a truncated path still
 * looks like a route: it reports as drift on a path that is almost right, and
 * if it happens to collide with a documented one, genuine drift passes.
 *
 * Today only the first two cases occur. The third exists for when they stop
 * being the only ones — an imported `RestApi` or a gateway split across stacks
 * (task 0126) emits `Fn::ImportValue`, and a second `RestApi` in one template
 * emits a `Fn::GetAtt` this does not recognize.
 */
function classifyParent(parentId) {
  // Nested resources reference their parent resource via Ref.
  if (parentId?.Ref) return { kind: 'ref', id: parentId.Ref };
  // Root children reference the API's RootResourceId via Fn::GetAtt.
  const att = parentId?.['Fn::GetAtt'];
  if (Array.isArray(att) && att[1] === 'RootResourceId')
    return { kind: 'root' };
  return { kind: 'unresolved', raw: parentId };
}

/** logicalId -> { pathPart, parent: {kind: 'ref'|'root'|'unresolved'} } */
const nodes = new Map();
for (const [id, res] of resources) {
  if (res.Type !== 'AWS::ApiGateway::Resource') continue;
  nodes.set(id, {
    pathPart: res.Properties?.PathPart,
    parent: classifyParent(res.Properties?.ParentId),
  });
}

function fullPath(id) {
  const segments = [];
  let cursor = id;
  // The template is a DAG rooted at the RestApi; bound the walk anyway so a
  // malformed template fails loudly instead of hanging CI. Every exit below
  // throws rather than returning what was resolved so far — see classifyParent
  // for why a partial path is the worst possible outcome. Same stance as the
  // ANY and root-method cases below.
  for (let hops = 0; hops <= nodes.size; hops++) {
    const node = nodes.get(cursor);
    if (!node) {
      throw new Error(
        `resource ${id}: parent ${cursor} is not an ` +
          `AWS::ApiGateway::Resource in this template — cannot resolve its path`,
      );
    }
    // PathPart is a literal in every resource CDK emits for this app. If one
    // is ever an intrinsic or absent, `unshift`ing it would put `[object
    // Object]` or `undefined` in the middle of a path and compare that.
    if (typeof node.pathPart !== 'string' || !node.pathPart) {
      throw new Error(
        `resource ${cursor}: PathPart is not a non-empty literal string ` +
          `(${JSON.stringify(node.pathPart)}) — cannot compare a path built ` +
          `from it`,
      );
    }
    segments.unshift(node.pathPart);
    if (node.parent.kind === 'root') return '/' + segments.join('/');
    if (node.parent.kind === 'unresolved') {
      throw new Error(
        `resource ${cursor}: ParentId ${JSON.stringify(node.parent.raw)} is ` +
          `neither a Ref to another resource nor the API's RootResourceId, so ` +
          `its path cannot be resolved. Teach classifyParent() about it — do ` +
          `not let it read as the API root, which would truncate the path.`,
      );
    }
    cursor = node.parent.id;
  }
  throw new Error(
    `resource ${id}: ParentId walk exceeded ${nodes.size} hops without ` +
      `reaching the API root — the template has a resource cycle`,
  );
}

// CORS preflight is not conventionally described in an OpenAPI document, so
// OPTIONS is skipped on the GATEWAY side: task 0126 adds `addCorsPreflight`,
// which emits an OPTIONS method on every resource, and without this the gate
// would fail that PR with the misleading remedy "add a #[utoipa::path] for
// each".
//
// The exclusion is deliberately one-sided. An earlier revision dropped OPTIONS
// *and* HEAD from both sides to keep the two guards' method sets identical,
// which left a hole: a documented HEAD or OPTIONS operation was then checked by
// neither guard in either direction, so documenting `head /v1/assets/{id}` and
// never mapping it reproduced the exact unroutable-documented-route defect task
// 0124 exists to fix. HEAD is therefore compared normally — nothing generates
// it automatically on either side — and a documented OPTIONS fails loudly on
// the spec side below rather than passing unnoticed.
const GATEWAY_SKIPPED_METHODS = new Set(['options']);

/**
 * The onboarding portal's backend prefix (task 0184), which this check skips on
 * BOTH sides.
 *
 * These routes are deliberately absent from the OpenAPI document. The document
 * describes the public data API to integrators; the portal's endpoints belong
 * to the portal's own bundle and publishing them would advertise a half-built
 * portal to every reader of the spec — see the module docs on
 * `packages/prices-api/src/portal/mod.rs`. They are also mapped as a greedy
 * `{proxy+}` carrying one method per verb (see `PORTAL_API_RESOURCE_PATH` and
 * `PORTAL_API_METHODS` in `infra/src/lib/stacks/api-gateway-stack.ts`), so
 * there is no OpenAPI equivalent to compare against even if we wanted one: the
 * segment is a placeholder for whatever routes the axum router owns, and the
 * verbs say nothing about which paths answer on them.
 *
 * Mirrors `PORTAL_API_PREFIX` in that module — and, unlike when this skip was
 * written, that agreement is asserted rather than asked for in a comment. See
 * the check below the gateway walk. (A third copy, `PORTAL_BACKEND` in the
 * CloudFront routing table of `portal-hosting-stack.ts`, went with that stack
 * — task 0195.)
 *
 * The skip is symmetric, unlike the OPTIONS one above: a portal path appearing
 * in the document would be checked by neither direction, so the spec side
 * rejects it outright a few dozen lines down rather than passing over it.
 */
const PORTAL_API_PREFIX = '/api/';

const gatewayRoutes = new Set();
/** Routes the prefix skip above swallowed — asserted non-empty further down. */
const portalGatewayRoutes = [];
for (const [, res] of resources) {
  if (res.Type !== 'AWS::ApiGateway::Method') continue;
  // Every method CDK emits declares HttpMethod as a literal. A non-string would
  // stringify to `[object Object]`, which matches no OpenAPI operation key and
  // would report as permanent, unexplainable drift.
  const rawMethod = res.Properties?.HttpMethod;
  if (typeof rawMethod !== 'string' || !rawMethod) {
    console.error(
      `error: an AWS::ApiGateway::Method declares HttpMethod ` +
        `${JSON.stringify(rawMethod)}, which is not a literal verb this check ` +
        `can compare.`,
    );
    process.exit(1);
  }
  const method = rawMethod.toLowerCase();
  if (GATEWAY_SKIPPED_METHODS.has(method)) continue;
  const resourceId = res.Properties?.ResourceId?.Ref;
  // A method on the API root itself has no Ref'd resource; none exist today,
  // and silently skipping one would be a hole in the check. Checked before the
  // ANY case so the message below always has a path to name.
  if (!resourceId) {
    console.error(
      `error: method ${method} is attached to the API root, which this check ` +
        `does not model. Extend fullPath() before adding root-level methods.`,
    );
    process.exit(1);
  }
  let path;
  try {
    path = fullPath(resourceId);
  } catch (err) {
    console.error(`error: ${err.message}`);
    process.exit(1);
  }
  // Checked BEFORE the ANY case. The portal's verbs are enumerated today, so
  // the order does not currently decide anything — but it did when they were
  // `ANY`, and it would again if a slice ever collapsed them back, so the skip
  // stays first rather than depending on that.
  if (path.startsWith(PORTAL_API_PREFIX)) {
    portalGatewayRoutes.push(`${method} ${path}`);
    continue;
  }
  // `ANY` maps every verb at once and can never equal an OpenAPI operation
  // key, so it would report as undocumented forever. Skipping it silently
  // would instead hide a mapped route from the check — fail loudly and make
  // the next person decide, which is the same stance as the root-method case
  // above.
  if (method === 'any') {
    console.error(
      `error: method ANY on ${path} has no OpenAPI equivalent. Expand it into ` +
        `explicit verbs, or teach this check how to compare it.`,
    );
    process.exit(1);
  }
  gatewayRoutes.add(`${method} ${path}`);
}

// ---------------------------------------------------------------------------
// The portal prefix agrees between the handler and this check.
// ---------------------------------------------------------------------------
// The skip above is the reason this section has to exist. The prefix is two
// independent literals — Rust `PORTAL_API_PREFIX` and the one in this file —
// and a comment saying "the two must agree" is not a check: move the Rust
// prefix alone and this script keeps skipping the OLD one while every route
// under the new one is compared against a document that never describes it,
// with the wrong remedy printed.
//
// There used to be a third copy, `PORTAL_BACKEND` in the CloudFront routing
// table of `portal-hosting-stack.ts`, and a whole section here probing that
// table the way CloudFront does — first match wins, the API row ahead of the
// bundle rows, cookies forwarded, caching off, access logs without cookies.
// The distribution was retired by task 0195 (the page is on the explorer's
// host and calls the API on its own hostname, task 0194), and the section
// went with it: the properties it guarded are now the explorer repo's, or do
// not exist — there is no CDN between the browser and the portal's routes.
const portalModPath = join(
  repoRoot,
  'packages',
  'prices-api',
  'src',
  'portal',
  'mod.rs',
);

function fail(...lines) {
  for (const line of lines) console.error(line);
  process.exit(1);
}

// --- 1. The handler's literal. ---
let portalMod;
try {
  portalMod = readFileSync(portalModPath, 'utf8');
} catch (err) {
  fail(`error: cannot read ${portalModPath}\n  ${err.message}`);
}
const rustPrefix = /pub const PORTAL_API_PREFIX: &str = "([^"]*)";/.exec(
  portalMod,
)?.[1];
if (rustPrefix === undefined) {
  fail(
    `error: could not find \`pub const PORTAL_API_PREFIX\` in ${portalModPath}.`,
    '  → this check reads it to prove the handler, the CDK routing table and ' +
      'this script still mean the same prefix. Restore the constant, or ' +
      'update the pattern here.',
  );
}
if (rustPrefix !== PORTAL_API_PREFIX) {
  fail(
    `error: the portal prefix disagrees between the handler and this check:`,
    `  ${portalModPath}: ${rustPrefix}`,
    `  ${fileURLToPath(import.meta.url)}: ${PORTAL_API_PREFIX}`,
    '  → the gateway resource (`PORTAL_API_RESOURCE_PATH` in ' +
      'infra/src/lib/stacks/api-gateway-stack.ts), the bundle (`BASE_PATH` ' +
      'in web/portal) and the Discord redirect URI all carry it too; a move ' +
      'is a coordinated change, see task 0235.',
  );
}

// --- 2. (Retired.) The CloudFront routing table — see the section comment. ---

// The anonymous routes keep their throttle in BOTH arms of `if (cacheEnabled)`.
//
// This is the trap task 0186 states as an acceptance criterion and task 0194
// audits, and until now nothing checked it. It cannot be checked from the
// synthesized template: `infra/envs/production.json` has
// `apiGatewayCacheEnabled: true`, so a synth only ever exercises ONE arm, and
// moving `...portalSettings` into that arm alone produces a template that is
// byte-identical to a correct one. The regression would be invisible until
// somebody turned the stage cache off — which is precisely the configuration
// where an unthrottled anonymous route costs the most, because every request is
// then also a billed Lambda invocation.
//
// So this reads the SOURCE. A source scan is a blunter instrument than the
// template checks above and it is the right one here: the property is about
// where a line sits in a conditional, which is a fact about the source and is
// erased by synthesis.
const gatewayStackPath = join(
  repoRoot,
  'infra',
  'src',
  'lib',
  'stacks',
  'api-gateway-stack.ts',
);
let gatewaySource;
try {
  gatewaySource = readFileSync(gatewayStackPath, 'utf8');
} catch (err) {
  fail(`error: cannot read ${gatewayStackPath}\n  ${err.message}`);
}

const methodSettingsArms = [
  ...gatewaySource.matchAll(
    /cfnStage\.methodSettings\s*=\s*\[([\s\S]*?)\n\s*\];/g,
  ),
].map((m) => m[1]);

// Two: the `if (cacheEnabled)` arm and its `else`. If this stops being two, the
// checks below would silently cover fewer arms than exist — so the count is
// asserted rather than assumed.
if (methodSettingsArms.length !== 2) {
  fail(
    `error: expected exactly 2 \`cfnStage.methodSettings = [...]\` assignments ` +
      `in ${gatewayStackPath}, found ${methodSettingsArms.length}.`,
    '  → this check proves the anonymous-route throttles appear in every arm ' +
      'of the cache conditional. If the assignment was restructured, teach ' +
      'this check the new shape rather than deleting it — the property it ' +
      'guards is a written acceptance criterion of task 0186.',
  );
}

for (const entry of ['...portalSettings', 'apiDocsSettings']) {
  const missing = methodSettingsArms.filter(
    (arm) => !arm.includes(entry),
  ).length;
  if (missing > 0) {
    fail(
      `error: \`${entry}\` is missing from ${missing} of the ` +
        `${methodSettingsArms.length} \`cfnStage.methodSettings\` assignments ` +
        `in ${gatewayStackPath}.`,
      '  → these are keyless, anonymous routes that sit outside the usage ' +
        'plan, so a method-level throttle is the only limit they carry. ' +
        'Declared in one arm only, it vanishes wherever ' +
        '`apiGatewayCacheEnabled` is false. Spread the entry into BOTH arms.',
    );
  }
}

// --- 3. The skip is not covering an empty set. ---
// If the gateway ever stops mapping anything under the prefix, the two checks
// above still pass — they only prove the CDN would route it — while every
// portal call 403s at the gateway. Non-vacuous, same stance as the
// `gatewayRoutes.size === 0` guard further down.
if (portalGatewayRoutes.length === 0) {
  fail(
    `error: API Gateway maps no route under ${PORTAL_API_PREFIX}, so this ` +
      `check's portal skip covers nothing.`,
    '  → the portal backend is unreachable in production: the bundle calls ' +
      'the API host directly and the gateway answers 403 Missing ' +
      'Authentication Token. ' +
      'Restore the `/api/{proxy+}` methods in ' +
      'infra/src/lib/stacks/api-gateway-stack.ts.',
  );
}

// ---------------------------------------------------------------------------
// Task 0187: self-service key issuance — the three settings it cannot see.
// ---------------------------------------------------------------------------
// Every property below is real only in a deployed account, invisible to the
// Rust suite (which drives a mock control plane), and fails in a way that
// reads as somebody else's bug.
const computeTemplate = readJson(
  join(repoRoot, 'infra', 'cdk.out', 'Prices-production-Compute.template.json'),
  'run `npm run infra:synth:production` first',
);

/** Every resource of a type, across a template. */
const resourcesOfType = (tpl, type) =>
  Object.entries(tpl.Resources ?? {}).filter(([, r]) => r.Type === type);

// --- 4. The two stacks agree on the SSM parameter name. ---
// `ApiGatewayStack` WRITES the usage-plan id to a parameter; `ComputeStack`
// tells the handler to READ that name. They cannot reference each other — the
// dependency runs Compute -> Gateway, so importing the plan would be a cycle —
// so the two strings are typed out twice, in two files, and nothing but this
// keeps them equal.
//
// Drift is silent in the worst way: `PORTAL_FREE_PLAN_PARAM` names a parameter
// that does not exist, the extension answers 404, and `load_portal_keys` fails
// COLD START. One router serves every route group (ADR 0008), so that is `/v1`
// down — but only on the deploy that first opens the portal, long after the
// edit that broke it.
const apiHandler = resourcesOfType(
  computeTemplate,
  'AWS::Lambda::Function',
).find(([id]) => id.startsWith('ApiHandlerFunction'));
if (!apiHandler) {
  fail(
    'error: no ApiHandlerFunction in the Compute template.',
    '  → this check cannot verify the portal key wiring. If the construct id ' +
      'changed, teach this check the new one rather than deleting it.',
  );
}
const handlerEnv = apiHandler[1].Properties?.Environment?.Variables ?? {};
const planParamRead = handlerEnv['PORTAL_FREE_PLAN_PARAM'];

const planParamWritten = resourcesOfType(template, 'AWS::SSM::Parameter')
  .map(([, r]) => r.Properties?.Name)
  .filter(
    (n) => typeof n === 'string' && n.endsWith('/pricing-api-free-plan-id'),
  );

if (planParamWritten.length !== 1) {
  fail(
    `error: expected exactly one SSM parameter ending in ` +
      `\`/pricing-api-free-plan-id\` in the ApiGateway template, found ` +
      `${planParamWritten.length}.`,
    '  → task 0187 reads the usage-plan id from it at cold start. Without it ' +
      'the portal cannot attach a key to a plan, and a key on no plan ' +
      'authenticates and is then refused.',
  );
}
if (planParamRead !== planParamWritten[0]) {
  fail(
    `error: the api-handler reads PORTAL_FREE_PLAN_PARAM=` +
      `${JSON.stringify(planParamRead ?? null)} but ApiGatewayStack publishes ` +
      `the plan id at ${JSON.stringify(planParamWritten[0])}.`,
    '  → the two stacks cannot reference each other (Compute -> Gateway ' +
      'would be a cycle), so this pair is two hand-typed strings. A mismatch ' +
      'fails Lambda INIT on the deploy that opens the portal, which takes ' +
      '/v1 down with it. Fix both in infra/src/lib/stacks/compute-stack.ts ' +
      'and api-gateway-stack.ts.',
  );
}

// --- 5. The control-plane grants are scoped. ---
// `CreateApiKey` has no ARN for "keys this function created", so the create
// grant is necessarily broad — that limit is accepted and documented in
// compute-stack.ts. What must NOT happen is it getting broader by accident: an
// `apigateway:*` action, or a `Resource: "*"`, hands the api-handler every API
// Gateway operation in the account, including changing the usage plan's limits
// and deleting the REST API.
const apigatewayStatements = [];
for (const tpl of [computeTemplate, template]) {
  for (const [, policy] of resourcesOfType(tpl, 'AWS::IAM::Policy')) {
    for (const st of policy.Properties?.PolicyDocument?.Statement ?? []) {
      const actions = [st.Action ?? []].flat();
      if (actions.some((a) => String(a).startsWith('apigateway:'))) {
        apigatewayStatements.push(st);
      }
    }
  }
}
if (apigatewayStatements.length === 0) {
  fail(
    'error: no IAM statement grants any `apigateway:` action.',
    '  → task 0187 issues keys through the API Gateway control plane; without ' +
      'these grants every issue and every reveal fails at runtime with ' +
      'AccessDenied. Restore them in compute-stack.ts and api-gateway-stack.ts.',
  );
}
for (const st of apigatewayStatements) {
  const actions = [st.Action ?? []].flat().map(String);
  const resources = [st.Resource ?? []].flat();
  if (actions.includes('apigateway:*')) {
    fail(
      `error: an IAM statement grants \`apigateway:*\` (sid ` +
        `${JSON.stringify(st.Sid ?? null)}).`,
      '  → task 0187 grants four verbs on two resources and nothing else. ' +
        '`apigateway:*` on the api-handler role includes deleting the REST ' +
        'API and rewriting the usage plan the whole service is metered by.',
    );
  }
  for (const resource of resources) {
    if (resource === '*') {
      fail(
        `error: an \`apigateway:\` statement has Resource "*" (sid ` +
          `${JSON.stringify(st.Sid ?? null)}).`,
        '  → the grants are meant to name `/apikeys`, `/apikeys/*` and the ' +
          'one usage plan. A bare "*" also covers every other REST API in ' +
          'the account.',
      );
    }
  }
}

// --- 5b. Task 0188: the GetUsage grant exists, and is the narrow form. ---
// The dashboard's `GetUsage` needs `apigateway:GET` on the free plan's
// `/usage` sub-resource — a statement in ApiGatewayStack's standalone policy,
// because only that stack knows the plan id. A missing statement fails at
// runtime with AccessDenied, only once the portal opens, and reads as a
// backend bug; a broadened one (`/usageplans/*`, or the plan root) hands the
// api-handler reads this feature never makes. The `apigateway:*` and
// `Resource: "*"` refusals above already bound the worst case; this pins the
// intended shape.
const usageGrants = apigatewayStatements.filter((st) => {
  const resources = [st.Resource ?? []].flat();
  // The resource is an Fn::Join carrying the plan id ref, so it is matched as
  // serialized JSON rather than as a string. `/usage"` — with the closing
  // quote — is the sub-resource as a path SUFFIX; a bare `/usage` would also
  // match every `/usageplans/…` ARN, 0187's `/keys` attach included.
  return resources.some((r) => JSON.stringify(r).includes('/usage"'));
});
if (usageGrants.length !== 1) {
  fail(
    `error: expected exactly one IAM statement on the usage plan's /usage ` +
      `sub-resource, found ${usageGrants.length}.`,
    '  → task 0188 reads per-key usage with GetUsage, granted as ' +
      '`apigateway:GET` on `/usageplans/{planId}/usage` in ' +
      'api-gateway-stack.ts (the standalone portal policy — the plan id ' +
      'lives in that stack). Without it every dashboard load fails with ' +
      'AccessDenied once the portal opens.',
  );
}
{
  const actions = [usageGrants[0].Action ?? []].flat().map(String);
  if (actions.length !== 1 || actions[0] !== 'apigateway:GET') {
    fail(
      `error: the /usage grant carries actions ${JSON.stringify(actions)}.`,
      '  → GetUsage needs `apigateway:GET` and nothing else on this ' +
        'resource. Anything more (PATCH is UpdateUsage — moving the quota ' +
        'counter) is a different feature and a different decision.',
    );
  }
  // The narrow form has two more properties the count and action cannot see:
  // the resource names THIS plan (a wildcard `/usageplans/*/usage` would read
  // every plan's usage and still count as one statement), and the statement
  // lives in the GATEWAY template — only that stack knows the plan id, so a
  // copy in ComputeStack would necessarily be hard-coded or wildcarded.
  const serialized = JSON.stringify([usageGrants[0].Resource ?? []].flat());
  if (serialized.includes('*')) {
    fail(
      `error: the /usage grant's resource contains a wildcard: ${serialized}.`,
      '  → the grant is meant to name the one pricing-api-free plan by ' +
        'reference (api-gateway-stack.ts). A wildcard reads usage for every ' +
        'plan in the account.',
    );
  }
  const inCompute = resourcesOfType(computeTemplate, 'AWS::IAM::Policy').some(
    ([, policy]) =>
      (policy.Properties?.PolicyDocument?.Statement ?? []).some((st) =>
        [st.Resource ?? []]
          .flat()
          .some((r) => JSON.stringify(r).includes('/usage"')),
      ),
  );
  if (inCompute) {
    fail(
      'error: a /usage grant appears in the Compute template.',
      '  → the GetUsage statement belongs in ApiGatewayStack’s standalone ' +
        'portal policy, where the plan id is a reference rather than a ' +
        'hand-typed string. See the cycle argument on `apiHandlerRole` in ' +
        'api-gateway-stack.ts.',
    );
  }
}

// --- 6. The portal's methods are uncached AT THE GATEWAY. ---
// Not deferrable to task 0194, and not the same check as the CloudFront one
// above. `deployOptions.cachingEnabled` is ON in this stack and the gateway
// cache has NO cache-key parameters on these methods, so every caller would
// collapse onto ONE entry — and a cached reveal hands one visitor another
// visitor's API key. The handler also sets `Cache-Control: no-store`, which the
// gateway cache does not honour: it caches by configuration, not by response
// header.
const stages = resourcesOfType(template, 'AWS::ApiGateway::Stage');
if (stages.length !== 1) {
  fail(
    `error: expected exactly one AWS::ApiGateway::Stage, found ${stages.length}.`,
    '  → this check reads the portal method settings off it.',
  );
}
const methodSettings = stages[0][1].Properties?.MethodSettings ?? [];
for (const httpMethod of ['GET', 'POST']) {
  const entry = methodSettings.find(
    (m) =>
      String(m.ResourcePath ?? '').startsWith(PORTAL_API_PREFIX) &&
      m.HttpMethod === httpMethod,
  );
  if (!entry) {
    fail(
      `error: no method setting for ${httpMethod} under ${PORTAL_API_PREFIX} ` +
        `in the synthesized stage.`,
      '  → task 0187 requires the key routes to be uncached and throttled. ' +
        'Without an entry they fall back to the `/*` default, which is not ' +
        'the statement this task needs to be able to point at.',
    );
  }
  if (entry.CachingEnabled !== false) {
    fail(
      `error: ${httpMethod} ${entry.ResourcePath} has CachingEnabled=` +
        `${JSON.stringify(entry.CachingEnabled ?? null)}.`,
      '  → the gateway cache has no cache-key parameters on this method, so ' +
        'every caller shares one entry. A cached `GET /api/key` ' +
        "serves one visitor another visitor's API key. Set " +
        '`cachingEnabled: false` in `portalSettings` in api-gateway-stack.ts.',
    );
  }
}

// --- 7. Task 0189: the eligibility parameters are named, and never CDK-owned. ---
// The gate's two knobs — which guild membership is checked against, and the
// minimum account age — are OPERATOR-seeded SSM parameters, read at runtime per
// issuance. Two properties hold that design together, and neither is visible to
// the Rust suite:
//
// (a) the handler carries the two parameter NAMES, exactly. A typo'd name means
//     the cold-start probe fails on the deploy that opens the portal — `/v1`
//     down — long after the edit that broke it (the check-4 failure mode).
// (b) NO synthesized template creates a parameter with either name. A
//     CloudFormation-managed parameter is CDK-owned, so the next `cdk deploy`
//     silently restores the committed value — which, after task 0179 points
//     production at the real Stellar guild, would un-flip it back to the test
//     guild. That regression is invisible at runtime until a member is refused.
const ELIGIBILITY_PARAMS = {
  PORTAL_GUILD_ID_PARAM: '/prices/production/discord-guild-id',
  PORTAL_MIN_ACCOUNT_AGE_PARAM: '/prices/production/min-account-age-minutes',
};
for (const [envVar, expected] of Object.entries(ELIGIBILITY_PARAMS)) {
  if (handlerEnv[envVar] !== expected) {
    fail(
      `error: the api-handler carries ${envVar}=` +
        `${JSON.stringify(handlerEnv[envVar] ?? null)}, expected ` +
        `${JSON.stringify(expected)}.`,
      '  → task 0189 resolves the eligibility knobs from these names per ' +
        'issuance (compute-stack.ts). The operator seeds the VALUES at deploy ' +
        'prep (runbook §2a); a drifted name fails the cold-start probe on the ' +
        'deploy that opens the portal, taking /v1 down with it.',
    );
  }
}
{
  const cdkOut = join(repoRoot, 'infra', 'cdk.out');
  const templateFiles = readdirSync(cdkOut).filter((f) =>
    f.endsWith('.template.json'),
  );
  if (templateFiles.length === 0) {
    fail(
      'error: no synthesized templates found in infra/cdk.out.',
      '  → run `npm run infra:synth:production` first.',
    );
  }
  const forbiddenSuffixes = Object.values(ELIGIBILITY_PARAMS).map(
    (name) => name.slice(name.lastIndexOf('/')), // '/discord-guild-id', …
  );

  // A `Name` is rarely a bare string once anyone interpolates the environment
  // into it — `/prices/${env}/discord-guild-id` synthesizes to an `Fn::Join`
  // or an `Fn::Sub`, which is the natural way somebody would write the very
  // parameter this check forbids. Inspecting only literal strings therefore
  // left the guard evadable by the most likely spelling of the mistake.
  //
  // So intrinsics are RESOLVED as far as their literal text goes, with each
  // unresolvable piece (a `Ref`, a `${Var}`) standing in as one NUL — a byte
  // no parameter name may contain, which makes "literal tail" and "followed
  // by something we cannot read" distinguishable below. Anything this cannot
  // read at all is a failure, not a skip: see the message on that branch.
  const PLACEHOLDER = '\u0000';
  const resolveName = (value) => {
    if (typeof value === 'string') return value;
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
      return null;
    }
    const keys = Object.keys(value);
    if (keys.length !== 1) return null;
    const [fn] = keys;
    const arg = value[fn];
    if (fn === 'Ref' || fn === 'Fn::GetAtt' || fn === 'Fn::ImportValue') {
      return PLACEHOLDER;
    }
    if (fn === 'Fn::Sub') {
      const template = Array.isArray(arg) ? arg[0] : arg;
      if (typeof template !== 'string') return null;
      return (
        template
          // `${Foo}` substitutes; `${!Foo}` is an escaped literal `${Foo}`
          // and must NOT be masked, or a suffix spelled that way would hide.
          .replace(/\$\{(?!!)[^}]*\}/g, PLACEHOLDER)
          .replace(/\$\{!([^}]*)\}/g, '${$1}')
      );
    }
    if (fn === 'Fn::Join' && Array.isArray(arg) && arg.length === 2) {
      const [separator, parts] = arg;
      if (typeof separator !== 'string' || !Array.isArray(parts)) return null;
      const resolved = parts.map(resolveName);
      if (resolved.some((part) => part === null)) return null;
      return resolved.join(separator);
    }
    return null;
  };

  // The suffix must be the END of the name, or be followed by something this
  // check could not read — never merely contained in it. `endsWith` alone
  // misses `${prefix}/discord-guild-id${suffix}`; a bare `includes` would
  // false-fail on a genuinely different parameter such as
  // `…/discord-guild-id-backup`, which is the over-broad-matcher mistake task
  // 0188's check 5b already made once.
  const createsParameter = (resolved) =>
    forbiddenSuffixes.some((suffix) => {
      for (
        let at = resolved.indexOf(suffix);
        at !== -1;
        at = resolved.indexOf(suffix, at + 1)
      ) {
        const after = resolved[at + suffix.length];
        if (after === undefined || after === PLACEHOLDER) return true;
      }
      return false;
    });

  for (const file of templateFiles) {
    const tpl = readJson(join(cdkOut, file), 'synthesized template');
    for (const [id, resource] of resourcesOfType(tpl, 'AWS::SSM::Parameter')) {
      const name = resource.Properties?.Name;
      // Absent is fine: CloudFormation then generates a physical name of its
      // own, which cannot be one of ours.
      if (name === undefined) continue;
      const resolved = resolveName(name);
      // A name whose LAST segment is not literal is unreadable in the only
      // position that matters: the leaf is what the forbidden suffixes are.
      // `{"Ref": …}` for the whole name is the extreme case of this.
      if (resolved === null || resolved.endsWith(PLACEHOLDER)) {
        fail(
          `error: ${file} creates SSM parameter ${id} with a Name this check ` +
            `cannot read: ${JSON.stringify(name)}.`,
          '  → the guard below has to be able to tell whether a synthesized ' +
            'template creates the eligibility parameters, and it refuses to ' +
            'pass a name it cannot resolve rather than assume it is ' +
            'harmless. Give the parameter a literal name, or teach ' +
            '`resolveName` this intrinsic.',
        );
        continue;
      }
      if (createsParameter(resolved)) {
        fail(
          `error: ${file} creates SSM parameter ${JSON.stringify(name)} ` +
            `(resource ${id}).`,
          '  → the eligibility parameters are operator-seeded, never ' +
            'CloudFormation resources: a CDK-owned parameter is restored to ' +
            'the committed value by the next deploy, un-flipping production ' +
            'back to the test guild after task 0179. Delete the ' +
            '`ssm.StringParameter` and seed the value by hand (runbook §2a).',
        );
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Spec side.
// ---------------------------------------------------------------------------
// `head` IS compared — see GATEWAY_SKIPPED_METHODS above. `options` is absent
// because a documented OPTIONS is rejected outright a few lines down rather
// than silently ignored.
const HTTP_METHODS = new Set([
  'get',
  'put',
  'post',
  'delete',
  'patch',
  'head',
  'trace',
]);
const specRoutes = new Set();
const documentedOptions = [];
for (const [path, item] of Object.entries(spec.paths ?? {})) {
  for (const key of Object.keys(item)) {
    if (key === 'options') documentedOptions.push(path);
    if (HTTP_METHODS.has(key)) specRoutes.add(`${key} ${path}`);
  }
}

// The gateway side cannot distinguish a CDK-generated preflight from a
// deliberately mapped OPTIONS, so it skips all of them. That makes a documented
// OPTIONS uncheckable in either direction — the one thing this script must not
// pass over in silence.
if (documentedOptions.length) {
  console.error(
    'error: the OpenAPI document describes OPTIONS operations, which this ' +
      'check cannot compare against the gateway (preflight is skipped there):',
  );
  for (const p of documentedOptions.sort()) console.error(`  options ${p}`);
  console.error(
    '  → stop documenting them, or teach this check how to tell a mapped ' +
      'OPTIONS from a generated preflight',
  );
  process.exit(1);
}

// The gateway side skips the portal prefix (see PORTAL_API_PREFIX), so a
// documented portal path would be compared against nothing and read as a pass.
// Documenting one is also a decision, not a slip — it puts the portal's
// endpoints in front of every integrator reading the spec — so say so rather
// than ignoring it.
const documentedPortalPaths = [...specRoutes].filter((r) =>
  r.slice(r.indexOf(' ') + 1).startsWith(PORTAL_API_PREFIX),
);
if (documentedPortalPaths.length) {
  console.error(
    `error: the OpenAPI document describes routes under ${PORTAL_API_PREFIX}, ` +
      'which this check skips on the gateway side and therefore cannot compare:',
  );
  for (const r of documentedPortalPaths.sort()) console.error(`  ${r}`);
  console.error(
    '  → the portal describes itself to its own bundle, not to integrators ' +
      '(packages/prices-api/src/portal/mod.rs). Drop the #[utoipa::path], or ' +
      'map the routes explicitly and teach this check to compare them.',
  );
  process.exit(1);
}

// ---------------------------------------------------------------------------
// Compare.
// ---------------------------------------------------------------------------
const undocumented = [...gatewayRoutes].filter((r) => !specRoutes.has(r));
const unroutable = [...specRoutes].filter((r) => !gatewayRoutes.has(r));

// A pair of empty sets compares equal to a pair of empty sets. Assert we
// actually looked at something, or a broken parse reads as success.
if (gatewayRoutes.size === 0 || specRoutes.size === 0) {
  console.error(
    `error: parsed ${gatewayRoutes.size} gateway route(s) and ` +
      `${specRoutes.size} spec path(s); refusing to pass vacuously`,
  );
  process.exit(1);
}

if (undocumented.length || unroutable.length) {
  if (undocumented.length) {
    console.error(
      'error: API Gateway maps routes the OpenAPI document does not describe:',
    );
    for (const r of undocumented.sort()) console.error(`  ${r}`);
    console.error(
      '  → add a #[utoipa::path] for each, or register it in openapi/mod.rs',
    );
  }
  if (unroutable.length) {
    console.error(
      'error: the OpenAPI document describes routes API Gateway does not map ' +
        '(every reader following them gets a 403):',
    );
    for (const r of unroutable.sort()) console.error(`  ${r}`);
    console.error('  → map them in infra/src/lib/stacks/api-gateway-stack.ts');
  }
  process.exit(1);
}

console.log(
  `OpenAPI document and API Gateway agree on ${gatewayRoutes.size} route(s):`,
);
for (const r of [...gatewayRoutes].sort()) console.log(`  ${r}`);
console.log(
  `\nPortal prefix ${PORTAL_API_PREFIX} agrees between the handler and this ` +
    `check; ${portalGatewayRoutes.length} gateway route(s) skipped as portal:`,
);
for (const r of portalGatewayRoutes.sort()) console.log(`  ${r}`);
