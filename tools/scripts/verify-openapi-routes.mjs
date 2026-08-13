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
 * `packages/prices-api/src/portal/mod.rs`. They are also mapped as a single
 * greedy `ANY /api-tokens/api/{proxy+}`, which has no OpenAPI equivalent to
 * compare against even if we wanted one.
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
  // Checked BEFORE the ANY case: the portal is mapped as `ANY {proxy+}`, so
  // the order decides whether this is a skip or a hard failure.
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
      'Restore `ANY /api-tokens/api/{proxy+}` in ' +
      'infra/src/lib/stacks/api-gateway-stack.ts.',
  );
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
