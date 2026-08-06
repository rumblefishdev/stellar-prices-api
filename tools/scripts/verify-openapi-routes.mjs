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

/** logicalId -> { pathPart, parentLogicalId | null (null = the API root) } */
const nodes = new Map();
for (const [id, res] of resources) {
  if (res.Type !== 'AWS::ApiGateway::Resource') continue;
  const parent = res.Properties?.ParentId;
  // Root children reference the API's RootResourceId via Fn::GetAtt; nested
  // resources reference their parent resource via Ref.
  nodes.set(id, {
    pathPart: res.Properties?.PathPart,
    parent: parent && parent.Ref ? parent.Ref : null,
  });
}

function fullPath(id) {
  const segments = [];
  let cursor = id;
  // The template is a DAG rooted at the RestApi; bound the walk anyway so a
  // malformed template fails loudly instead of hanging CI. Both exits below
  // throw rather than returning what was resolved so far: a truncated path
  // still *looks* like a route, so it would surface as drift on a path that is
  // almost right, which is harder to read than the parse failure it actually
  // is. Same stance as the ANY and root-method cases below.
  for (let hops = 0; hops <= nodes.size; hops++) {
    const node = nodes.get(cursor);
    if (!node) {
      throw new Error(
        `resource ${id}: parent ${cursor} is not an ` +
          `AWS::ApiGateway::Resource in this template — cannot resolve its path`,
      );
    }
    segments.unshift(node.pathPart);
    cursor = node.parent;
    // `parent: null` is the API root: the path is complete.
    if (cursor === null) return '/' + segments.join('/');
  }
  throw new Error(
    `resource ${id}: ParentId walk exceeded ${nodes.size} hops without ` +
      `reaching the API root — the template has a resource cycle`,
  );
}

// CORS preflight and HEAD are not conventionally described in an OpenAPI
// document, so they are excluded from BOTH sides rather than compared. Task
// 0126 adds `addCorsPreflight`, which emits an OPTIONS method on every
// resource; without this the gate would fail that PR with the misleading
// remedy "add a #[utoipa::path] for each". The tradeoff is that a documented
// OPTIONS/HEAD path is not checked against the gateway either — acceptable
// only because preflight is out of scope for the document by convention.
const UNCOMPARED_METHODS = new Set(['options', 'head']);

const gatewayRoutes = new Set();
for (const [, res] of resources) {
  if (res.Type !== 'AWS::ApiGateway::Method') continue;
  const method = String(res.Properties?.HttpMethod ?? '').toLowerCase();
  if (UNCOMPARED_METHODS.has(method)) continue;
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
// Spec side.
// ---------------------------------------------------------------------------
// `head` and `options` are absent deliberately — see UNCOMPARED_METHODS above.
// Both sides must exclude the same set or the exclusion creates false drift.
const HTTP_METHODS = new Set([
  'get',
  'put',
  'post',
  'delete',
  'patch',
  'trace',
]);
const specRoutes = new Set();
for (const [path, item] of Object.entries(spec.paths ?? {})) {
  for (const key of Object.keys(item)) {
    if (HTTP_METHODS.has(key)) specRoutes.add(`${key} ${path}`);
  }
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
