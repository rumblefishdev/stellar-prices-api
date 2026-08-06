#!/usr/bin/env node
/**
 * Assert the `servers` URL in the published OpenAPI document is the URL the
 * deployed api-handler is actually configured with.
 *
 * WHY THIS EXISTS
 * ---------------
 * `tools/scripts/extract-openapi.sh` stamps `servers` by reading `apiBaseUrl`
 * out of `infra/envs/production.json` and exporting `API_BASE_URL` itself. That
 * is the right source for the *value*, but it means the extraction never
 * observes the thing that puts that value on the Lambda at runtime —
 * `ComputeStack`'s `API_BASE_URL: config.apiBaseUrl`. The two are independent.
 *
 * So the branch that added the lint gate could still ship this: rename the env
 * var to `API_BASE_URI` during an unrelated ComputeStack refactor, or drop the
 * line entirely. Synth succeeds, `openapi:lint` succeeds — because CI's copy of
 * the document was stamped from the JSON file, not from the deployment — and
 * production's `GET /api-docs-json` comes back with **no `servers` block at
 * all**, because the handler reads `API_BASE_URL` from its environment and
 * finds nothing. That is exactly the "the thing CI blesses and the thing
 * production serves are two different files" drift `stamp_servers()` was
 * written to prevent, reintroduced one layer down.
 *
 * This closes it by comparing artifacts, the same way `verify-openapi-routes`
 * does for paths:
 *
 *   - the synthesized Compute template (what the Lambda's environment will be)
 *   - the extracted OpenAPI document (what the handler will advertise)
 *
 * The api-handler is identified by *carrying* `API_BASE_URL`, not by name, so
 * renaming the variable fails as "no function declares it" rather than passing
 * against a function this script could no longer find.
 *
 * Usage:
 *   npm run infra:synth:production   # → infra/cdk.out/*.template.json
 *   npm run openapi:verify-servers   # re-extracts the document, then compares
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const cdkOut = join(repoRoot, 'infra', 'cdk.out');
const computePath = join(cdkOut, 'Prices-production-Compute.template.json');
const gatewayPath = join(cdkOut, 'Prices-production-ApiGateway.template.json');
const specPath = join(repoRoot, 'target', 'openapi.json');

function readJson(path, hint) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (err) {
    console.error(`error: cannot read ${path}\n  ${err.message}\n  ${hint}`);
    process.exit(1);
  }
}

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

const synthHint = 'run `npm run infra:synth:production` first';
const compute = readJson(computePath, synthHint);
const gateway = readJson(gatewayPath, synthHint);
const spec = readJson(specPath, 'run `npm run openapi:extract` first');

// ---------------------------------------------------------------------------
// Deployment side: the api-handler's API_BASE_URL.
// ---------------------------------------------------------------------------
const declaring = Object.entries(compute.Resources ?? {}).filter(
  ([, res]) =>
    res.Type === 'AWS::Lambda::Function' &&
    res.Properties?.Environment?.Variables?.API_BASE_URL !== undefined,
);

if (declaring.length === 0) {
  fail(
    `no Lambda function in ${computePath} declares API_BASE_URL. The ` +
      `api-handler builds the OpenAPI document's \`servers\` block from that ` +
      `variable, so the deployed GET /api-docs-json would advertise no base ` +
      `URL at all.\n  → restore \`API_BASE_URL: config.apiBaseUrl\` in ` +
      `infra/src/lib/stacks/compute-stack.ts`,
  );
}
if (declaring.length > 1) {
  fail(
    `${declaring.length} Lambda functions declare API_BASE_URL ` +
      `(${declaring.map(([id]) => id).join(', ')}); this check cannot tell ` +
      `which one serves /api-docs-json. Teach it how to pick, or stop ` +
      `setting the variable on the others.`,
  );
}

const [handlerId, handler] = declaring[0];
const deployedBase = handler.Properties.Environment.Variables.API_BASE_URL;

// A CloudFormation intrinsic (`Ref`, `Fn::GetAtt`, …) resolves only at deploy
// time, so this check could not compare it against anything. Today the value is
// a literal from config; refuse rather than silently comparing an object to a
// string and reporting nonsense drift.
if (typeof deployedBase !== 'string') {
  fail(
    `${handlerId}'s API_BASE_URL is not a literal string ` +
      `(${JSON.stringify(deployedBase)}) — it resolves at deploy time, so this ` +
      `check cannot compare it against the document. Keep it config-supplied, ` +
      `or teach this check how to resolve it.`,
  );
}

// ---------------------------------------------------------------------------
// Document side.
// ---------------------------------------------------------------------------
const servers = spec.servers;
if (!Array.isArray(servers) || servers.length === 0) {
  fail(
    `the extracted document has no \`servers\` block. A reader is told ` +
      `nothing about where to send requests.\n  → see stamp_servers() in ` +
      `packages/prices-api/src/openapi/mod.rs`,
  );
}
if (servers.length !== 1) {
  fail(
    `the document advertises ${servers.length} servers; this check models one ` +
      `(the deployed stage). Extend it before advertising more.`,
  );
}

const advertised = servers[0]?.url;
if (typeof advertised !== 'string' || !advertised) {
  fail(
    `servers[0].url is not a non-empty string: ${JSON.stringify(advertised)}`,
  );
}

// ---------------------------------------------------------------------------
// Compare.
// ---------------------------------------------------------------------------
if (advertised !== deployedBase) {
  fail(
    `the document advertises a base URL the deployed handler is not ` +
      `configured with:\n` +
      `  document (servers[0].url):        ${advertised}\n` +
      `  ${handlerId} (API_BASE_URL): ${deployedBase}\n` +
      `  → every partner client generated from this document would send ` +
      `traffic somewhere the API is not.`,
  );
}

// The stage-prefix trap (task 0089/0124): an execute-api host serves the API
// only under /{stage}, so a base without it advertises a URL where every route
// 403s. `validateConfig` in infra/src/lib/types.ts asserts this at synth time
// against the configured env name; assert it here too, against the stage the
// template actually deploys, so deleting that validation does not silently
// remove the guarantee. A custom domain (task 0126) has no such requirement.
const stages = Object.values(gateway.Resources ?? {}).filter(
  (res) => res.Type === 'AWS::ApiGateway::Stage',
);
if (stages.length !== 1) {
  fail(
    `expected exactly 1 AWS::ApiGateway::Stage in ${gatewayPath}, found ` +
      `${stages.length}; this check cannot pick the stage the base URL must ` +
      `carry.`,
  );
}
const stageName = stages[0].Properties?.StageName;
if (typeof stageName !== 'string' || !stageName) {
  fail(
    `the deployed stage has no literal StageName: ${JSON.stringify(stageName)}`,
  );
}

if (
  advertised.includes('.execute-api.') &&
  !advertised.endsWith(`/${stageName}`)
) {
  fail(
    `the advertised base URL is an execute-api host and must end with the ` +
      `stage path "/${stageName}", got: ${advertised}\n  → without it every ` +
      `documented route 403s (the stage-prefix trap).`,
  );
}

console.log(
  `OpenAPI \`servers\` matches the deployed api-handler (${handlerId}):\n` +
    `  ${advertised}`,
);
