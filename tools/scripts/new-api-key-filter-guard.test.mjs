// The sign-up metric and the Slack message (task 0355) hang on one log line.
// ObservabilityStack's `newApiKeyPattern` matches `portal issued an API key`
// with `created: true`, logged by `keys::issue_for`; the metric filter and the
// notifier's subscription filter both use it. The API hub issues through
// that same function, which is why the hub's own line is not counted (it
// would count each hub key twice). Reword the line, rename the field, or give
// the hub its own issue path, and both undercount with every check green.
// The JSON shape (`$.fields.*`) is guarded by portal-load-failed-filter-guard.
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
const portal = read(
  'packages',
  'prices-api',
  'src',
  'portal',
  'keys',
  'mod.rs',
);
const hub = read('packages', 'prices-api', 'src', 'portal', 'hub', 'key.rs');

const pattern = stack.match(
  /const newApiKeyPattern = logs\.FilterPattern\.all\(([\s\S]*?)\n {4}\);/,
)?.[1];
const block = (kind, id) =>
  stack.match(
    new RegExp(`new ${kind}\\(\\s*this,\\s*'${id}',([\\s\\S]*?)\\n\\s*\\}\\);`),
  )?.[1];

test('the pattern matches the portal line and its created field', () => {
  assert.ok(pattern, 'newApiKeyPattern not found');
  assert.match(
    pattern,
    /'\$\.fields\.message',\s*'=',\s*'portal issued an API key'/,
  );
  assert.match(pattern, /booleanValue\('\$\.fields\.created', true\)/);
});

test('the metric filter and the subscription filter share the pattern', () => {
  const metric = block('logs\\.MetricFilter', 'ApiHandlerNewApiKeyFilter');
  const subscription = block(
    'logs\\.SubscriptionFilter',
    'ApiHandlerNewApiKeySubscription',
  );
  assert.ok(metric, 'ApiHandlerNewApiKeyFilter not found');
  assert.ok(subscription, 'ApiHandlerNewApiKeySubscription not found');
  assert.match(metric, /filterPattern: newApiKeyPattern,/);
  assert.match(subscription, /filterPattern: newApiKeyPattern,/);
  assert.match(subscription, /LambdaDestination\(signupNotifier\)/);
});

test('issue_for logs that line with created', () => {
  assert.match(
    portal,
    /tracing::info!\([^;]*created = outcome\.created,\s*"portal issued an API key"\s*\)/,
  );
});

test('the hub issues through issue_for, so the portal line covers it', () => {
  assert.match(hub, /keys::issue_for\(/);
  assert.doesNotMatch(pattern, /hub issued an API key/);
});

// The message goes to the sign-ups topic, never ops-alarms, and the notifier
// reads the key the line names.
test('the notifier publishes to the sign-ups topic', () => {
  assert.match(stack, /TOPIC_ARN: signupsTopic\.topicArn,/);
  assert.match(stack, /signupsTopic\.grantPublish\(signupNotifier\);/);
  assert.doesNotMatch(stack, /opsAlarmsTopic\.grantPublish\(signupNotifier\)/);
});
