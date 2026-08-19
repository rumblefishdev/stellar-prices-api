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
import { readFileSync } from 'node:fs';
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
 * Mirrors `PORTAL_API_PREFIX` in that module and `PORTAL_BACKEND` in
 * `infra/src/lib/stacks/portal-hosting-stack.ts` — and, unlike when this skip
 * was written, that agreement is now asserted rather than asked for in a
 * comment. See the three-way check below the gateway walk.
 *
 * The skip is symmetric, unlike the OPTIONS one above: a portal path appearing
 * in the document would be checked by neither direction, so the spec side
 * rejects it outright a few dozen lines down rather than passing over it.
 */
const PORTAL_API_PREFIX = '/api-tokens/api/';

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
// The portal prefix agrees across all three places that declare it.
// ---------------------------------------------------------------------------
// The skip above is the reason this section has to exist. Until now the prefix
// was three independent literals — Rust `PORTAL_API_PREFIX`, CDK
// `PORTAL_BACKEND`, and the one in this file — tied together by comments saying
// "the three must agree", which is not a check.
//
// The failure that buys: task 0186 moves the Rust prefix, the CloudFront table
// still routes the NEW prefix to S3, and the OAuth callback is answered with
// the SPA bundle as `200 text/html` — the exact "reads as a JSON parse error,
// not as a routing bug" outcome the portal stack's own docblock warns about.
// CI stays green throughout, because this script skips the OLD prefix and the
// portal is absent from the document either way.
//
// This script already derives both of its sides from synthesized artifacts, so
// it is the natural place: same technique, one more artifact.
const portalTemplate = readJson(
  join(
    repoRoot,
    'infra',
    'cdk.out',
    'Prices-production-PortalHosting.template.json',
  ),
  'run `npm run infra:synth:production` first',
);
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
    '  → the routing table in infra/src/lib/stacks/portal-hosting-stack.ts ' +
      'almost certainly needs the same edit; all three must agree.',
  );
}

// --- 2. The CloudFront behaviour that must win for a portal backend path. ---
const distributions = Object.values(portalTemplate.Resources ?? {}).filter(
  (r) => r.Type === 'AWS::CloudFront::Distribution',
);
if (distributions.length !== 1) {
  fail(
    `error: expected exactly 1 AWS::CloudFront::Distribution in the ` +
      `PortalHosting template, found ${distributions.length}. This check ` +
      `assumes one distribution owns the portal's routing table.`,
  );
}
const distConfig = distributions[0].Properties?.DistributionConfig ?? {};
const behaviours = distConfig.CacheBehaviors ?? [];
const originsById = new Map((distConfig.Origins ?? []).map((o) => [o.Id, o]));

/**
 * CloudFront matches a path pattern with `*` (any run of characters) and `?`
 * (exactly one), first behaviour wins. Everything else is a literal — escape it
 * so a `.` or `+` in a pattern cannot widen the match.
 */
const patternToRegExp = (pattern) =>
  new RegExp(
    `^${pattern.replace(/[.*+?^${}()|[\]\\]/g, (c) =>
      c === '*' ? '.*' : c === '?' ? '.' : `\\${c}`,
    )}$`,
  );

// Ordering is the property under test, so probe it the way CloudFront does:
// take the FIRST pattern that matches a representative backend path.
const probe = `${PORTAL_API_PREFIX}probe`;
const winner = behaviours.find((b) =>
  patternToRegExp(String(b.PathPattern)).test(probe),
);
const expectedPattern = `${PORTAL_API_PREFIX}*`;

if (!winner) {
  fail(
    `error: no cache behaviour matches ${probe}, so it falls to the ` +
      `distribution's DefaultCacheBehavior — the bundle bucket.`,
    `  → add \`${expectedPattern}\` to the routing table in ` +
      'infra/src/lib/stacks/portal-hosting-stack.ts.',
  );
}
if (winner.PathPattern !== expectedPattern) {
  fail(
    `error: ${probe} is matched first by \`${winner.PathPattern}\`, not by ` +
      `\`${expectedPattern}\`.`,
    '  → CloudFront takes the first match, so a broader pattern listed above ' +
      'the portal backend swallows every portal API call and answers it with ' +
      'whatever that behaviour serves. If that is the bundle, the caller gets ' +
      'a 200 full of HTML, which surfaces as a JSON parse error rather than ' +
      'as a routing bug. Reorder `additionalBehaviors` in ' +
      'infra/src/lib/stacks/portal-hosting-stack.ts.',
  );
}
// Matching first is not enough — it has to point at the API. An S3 target here
// is the same 200-full-of-HTML failure arriving by a different route.
const winnerOrigin = originsById.get(winner.TargetOriginId);
if (!winnerOrigin?.CustomOriginConfig) {
  fail(
    `error: behaviour \`${winner.PathPattern}\` targets origin ` +
      `\`${winner.TargetOriginId}\`, which is not an HTTP (execute-api) ` +
      `origin — the portal's backend calls would be served from the bundle ` +
      `bucket.`,
  );
}
// Two more settings on the same objects, each load-bearing and each failing
// silently. Cheap to assert now that `winner` and `winnerOrigin` are in hand.
//
// The methods: CloudFront's default allowance is GET/HEAD, and it answers
// anything else with a 403 of its own that never reaches the API — which would
// take out task 0186's token exchange and task 0187's key issue, both POSTs,
// while every GET kept working.
const WRITE_METHODS = ['POST', 'PUT', 'PATCH', 'DELETE'];
const allowed = new Set(winner.AllowedMethods ?? []);
const missingMethods = WRITE_METHODS.filter((m) => !allowed.has(m));
if (missingMethods.length > 0) {
  fail(
    `error: behaviour \`${winner.PathPattern}\` does not allow ` +
      `${missingMethods.join(', ')}.`,
    '  → CloudFront rejects a disallowed method with its own 403 before the ' +
      "origin is reached, so the portal's writes would fail while its reads " +
      'kept working. Set `allowedMethods: ALLOW_ALL` on the API behaviour in ' +
      'infra/src/lib/stacks/portal-hosting-stack.ts.',
  );
}
// The origin path: API Gateway serves a REST API only under `/{stage}`, so
// without it every proxied request arrives one segment short and 403s.
if (!/^\/[^/]+$/.test(String(winnerOrigin.OriginPath ?? ''))) {
  fail(
    `error: the API origin behind \`${winner.PathPattern}\` has OriginPath ` +
      `${JSON.stringify(winnerOrigin.OriginPath ?? null)}, which is not a ` +
      `single stage segment.`,
    '  → an execute-api origin serves the REST API under `/{stage}` only. ' +
      'Without it CloudFront forwards `/api-tokens/api/x` as `/api-tokens/' +
      'api/x`, the gateway maps nothing, and every portal call 403s. Set ' +
      '`originPath` on the HttpOrigin in ' +
      'infra/src/lib/stacks/portal-hosting-stack.ts.',
  );
}

// The session cookie has to reach the origin, and the responses that carry it
// must not be cached at the edge (task 0186). Both are properties of the two
// managed policies attached to this behaviour, both fail ONLY in a browser —
// `curl` sends whatever headers you tell it to and reads whatever comes back —
// and both are the kind of setting a later "tidy-up" reaches for.
//
// Asserted by managed-policy ID rather than by name, because the ID is what the
// template actually contains. AWS documents these as fixed and global:
//   Managed-AllViewerExceptHostHeader  b689b0a8-53d0-40ab-baf2-68738e2966ac
//   Managed-CachingDisabled            4135ea2d-6df8-44a3-9df3-4b5a84be39ad
//
// The narrow reading of the first is "forward every header except Host", which
// is how `portal-hosting-stack.ts` originally justified it. The load-bearing
// half for this task is that it also forwards **all cookies** — which
// `Managed-CORS-S3Origin`, `Managed-UserAgentRefererHeaders` and every other
// managed origin-request policy except `Managed-AllViewer` do not. Swapping to
// one of those would leave `Host` correct, every `curl` passing, and every
// signed-in visitor reading as signed out.
const ALL_VIEWER_EXCEPT_HOST_HEADER = 'b689b0a8-53d0-40ab-baf2-68738e2966ac';
const CACHING_DISABLED = '4135ea2d-6df8-44a3-9df3-4b5a84be39ad';

if (String(winner.OriginRequestPolicyId) !== ALL_VIEWER_EXCEPT_HOST_HEADER) {
  fail(
    `error: behaviour \`${winner.PathPattern}\` has OriginRequestPolicyId ` +
      `${JSON.stringify(winner.OriginRequestPolicyId ?? null)}, not ` +
      `Managed-AllViewerExceptHostHeader (${ALL_VIEWER_EXCEPT_HOST_HEADER}).`,
    '  → that policy is what forwards the portal session cookie to the origin ' +
      "and what withholds the viewer's Host from execute-api. Under any other " +
      'managed policy the cookie is stripped, `/api-tokens/api/auth/me` reads ' +
      'as signed-out for a visitor who just signed in, and nothing fails ' +
      'outside a browser. Set `originRequestPolicy: ' +
      'ALL_VIEWER_EXCEPT_HOST_HEADER` on the API behaviour in ' +
      'infra/src/lib/stacks/portal-hosting-stack.ts.',
  );
}
if (String(winner.CachePolicyId) !== CACHING_DISABLED) {
  fail(
    `error: behaviour \`${winner.PathPattern}\` has CachePolicyId ` +
      `${JSON.stringify(winner.CachePolicyId ?? null)}, not ` +
      `Managed-CachingDisabled (${CACHING_DISABLED}).`,
    '  → the portal backend carries a session cookie and, from task 0187, an ' +
      'API key. A cached response on this prefix is one visitor served ' +
      "another visitor's identity or credential. Set `cachePolicy: " +
      'CACHING_DISABLED` on the API behaviour in ' +
      'infra/src/lib/stacks/portal-hosting-stack.ts.',
  );
}

// Cookies must reach the origin; they must NOT be written to the access-log
// bucket. From task 0186 the portal's cookie IS the session, so logging it
// would put a usable credential in S3 in plaintext for the log bucket's whole
// 90-day retention, to answer a question the request line already answers.
if (distConfig.Logging && distConfig.Logging.IncludeCookies !== false) {
  fail(
    `error: the distribution's access logging has IncludeCookies=` +
      `${JSON.stringify(distConfig.Logging.IncludeCookies ?? null)}.`,
    '  → from task 0186 the portal session is a cookie, so this writes a live ' +
      'credential into the log bucket in plaintext. Set ' +
      '`logIncludesCookies: false` in ' +
      'infra/src/lib/stacks/portal-hosting-stack.ts.',
  );
}

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
    '  → the portal backend is unreachable in production: CloudFront forwards ' +
      'the request and the gateway answers 403 Missing Authentication Token. ' +
      'Restore the `/api-tokens/api/{proxy+}` methods in ' +
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
        'every caller shares one entry. A cached `GET /api-tokens/api/key` ' +
        "serves one visitor another visitor's API key. Set " +
        '`cachingEnabled: false` in `portalSettings` in api-gateway-stack.ts.',
    );
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
  `\nPortal prefix ${PORTAL_API_PREFIX} agrees across the handler, the ` +
    `CloudFront routing table and this check; ` +
    `${portalGatewayRoutes.length} gateway route(s) skipped as portal:`,
);
for (const r of portalGatewayRoutes.sort()) console.log(`  ${r}`);
