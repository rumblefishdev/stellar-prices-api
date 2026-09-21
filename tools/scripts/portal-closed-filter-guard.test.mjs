// The portal-closed alarm (task 0249) hangs on one string. ObservabilityStack's
// metric filter matches `{ $.fields.message = "portal closed at cold start*" }`
// on the api-handler log group; `packages/prices-api/src/main.rs` is what logs
// it. Nothing else ties the two together: reword the log line, or flatten the
// JSON subscriber, and the alarm goes quiet for good with every check green.
// Asserted here:
//
//   - the filter reads `$.fields.message` and matches by prefix;
//   - `main.rs` has a `tracing::error!` whose message starts with that prefix;
//   - the subscriber is `fmt().json()` and not flattened, which is what puts
//     the message under `fields`.
//
// The pattern itself was proven against AWS's evaluator with
// `aws logs test-metric-filter` (task 0249's notes); this guards the drift.
//
// Run: npx nx test @rumblefish/stellar-prices-api-aws-cdk

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..');
const stackSource = readFileSync(
  join(repoRoot, 'infra', 'src', 'lib', 'stacks', 'observability-stack.ts'),
  'utf8',
);
const mainSource = readFileSync(
  join(repoRoot, 'packages', 'prices-api', 'src', 'main.rs'),
  'utf8',
);

// `FilterPattern.stringValue('<key>', '=', '<prefix>*')` on the portal-closed
// filter → { key, prefix }, or undefined when it is not a prefix match.
const portalClosedFilter = (source) => {
  const filter = source.match(
    /'ApiHandlerPortalClosedFilter'[\s\S]*?FilterPattern\.stringValue\(\s*'([^']+)',\s*'='\s*,\s*'([^']+)\*',?\s*\)/,
  );
  return filter ? { key: filter[1], prefix: filter[2] } : undefined;
};

// Does some `tracing::error!(…)` carry a message literal starting with `prefix`?
const logsErrorStartingWith = (source, prefix) =>
  [...source.matchAll(/tracing::error!\(([\s\S]*?)\);/g)].some(([, body]) =>
    body.includes(`"${prefix}`),
  );

const subscriberPutsMessageUnderFields = (source) => {
  const subscriber = source.match(
    /tracing_subscriber::fmt\(\)([\s\S]*?)\.init\(\)/,
  );
  return (
    subscriber !== null &&
    /\.json\(\)/.test(subscriber[1]) &&
    !/\.flatten_event\(\s*true\s*\)/.test(subscriber[1])
  );
};

test('the portal-closed filter matches $.fields.message by prefix', () => {
  const filter = portalClosedFilter(stackSource);

  assert.ok(
    filter,
    'found no prefix FilterPattern on the portal-closed filter',
  );
  assert.equal(filter.key, '$.fields.message');
});

test('main.rs logs an error starting with the prefix the filter matches', () => {
  const { prefix } = portalClosedFilter(stackSource);

  assert.ok(
    logsErrorStartingWith(mainSource, prefix),
    `no tracing::error! in main.rs starts with "${prefix}" — the portal-closed alarm would never fire`,
  );
});

test('the subscriber keeps the message under `fields`', () => {
  assert.ok(
    subscriberPutsMessageUnderFields(mainSource),
    'main.rs must log through fmt().json() without flatten_event(true), or $.fields.message matches nothing',
  );
});

test('a reworded log line is refused', () => {
  const reworded = mainSource.replace(
    'portal closed at cold start',
    'the portal was closed at cold start',
  );

  assert.notEqual(reworded, mainSource);
  assert.equal(
    logsErrorStartingWith(reworded, 'portal closed at cold start'),
    false,
  );
});

test('a flattened subscriber is refused', () => {
  const flattened = mainSource.replace(
    '.json()',
    '.json().flatten_event(true)',
  );

  assert.notEqual(flattened, mainSource);
  assert.equal(subscriberPutsMessageUnderFields(flattened), false);
});
