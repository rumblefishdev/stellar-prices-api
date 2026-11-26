// The sign-up metric (task 0355) hangs on one log line. ObservabilityStack's
// `ApiHandlerNewApiKeyFilter` matches `portal issued an API key` with
// `created: true`, logged by `keys::issue_for`. The API hub issues through
// that same function, which is why the hub's own line is not counted (it
// would count each hub key twice). Reword the line, rename the field, or give
// the hub its own issue path, and the metric undercounts with every check
// green. The JSON shape (`$.fields.*`) is guarded by
// portal-load-failed-filter-guard.
//
// Run: npx nx test @rumblefish/stellar-prices-api-aws-cdk

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const read = (...p) => readFileSync(join(root, ...p), 'utf8');

const stack = read('infra', 'src', 'lib', 'stacks', 'observability-stack.ts');
const portal = read('packages', 'prices-api', 'src', 'portal', 'keys', 'mod.rs');
const hub = read('packages', 'prices-api', 'src', 'portal', 'hub', 'key.rs');

const filter = stack.match(
  /new logs\.MetricFilter\(\s*this,\s*'ApiHandlerNewApiKeyFilter',([\s\S]*?)\n\s*\);/,
)?.[1];

test('the filter matches the portal line and its created field', () => {
  assert.ok(filter, 'ApiHandlerNewApiKeyFilter not found');
  assert.match(filter, /'\$\.fields\.message',\s*'=',\s*'portal issued an API key'/);
  assert.match(filter, /booleanValue\('\$\.fields\.created', true\)/);
});

test('issue_for logs that line with created', () => {
  assert.match(
    portal,
    /tracing::info!\([^;]*created = outcome\.created,\s*"portal issued an API key"\s*\)/,
  );
});

test('the hub issues through issue_for, so the portal line covers it', () => {
  assert.match(hub, /keys::issue_for\(/);
  assert.doesNotMatch(filter, /hub issued an API key/);
});
