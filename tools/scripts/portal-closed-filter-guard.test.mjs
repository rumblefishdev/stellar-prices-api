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

// The portal-closed filter's own options object — from its construct id to
// the `);` that closes `new logs.MetricFilter(`. Searching inside that block
// only: an unbounded search would run on to the FIRST `stringValue(...)`
// anywhere later in the file, so a second filter added after this one could
// stand in for it and the guard would pass on the wrong filter.
const portalClosedFilterBlock = (source) => {
  const block = source.match(
    /new logs\.MetricFilter\(\s*this,\s*'ApiHandlerPortalClosedFilter',([\s\S]*?)\n\s*\);/,
  );
  return block ? block[1] : undefined;
};

// `FilterPattern.stringValue('<key>', '=', '<prefix>*')` inside that block →
// { key, prefix }, or undefined when the filter is not a prefix match on a key.
const portalClosedFilter = (source) => {
  const filter = portalClosedFilterBlock(source)?.match(
    /FilterPattern\.stringValue\(\s*'([^']+)',\s*'='\s*,\s*'([^']+)\*',?\s*\)/,
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

test('a second filter later in the file cannot stand in for the portal-closed one', () => {
  // The portal-closed filter moves off `stringValue`; another filter with the
  // right pattern is added after it. The alarm now matches nothing.
  const swapped = stackSource
    .replace(
      /filterPattern: logs\.FilterPattern\.stringValue\(\s*'\$\.fields\.message',\s*'=',\s*'portal closed at cold start\*',\s*\)/,
      'filterPattern: logs.FilterPattern.literal(\'{ $.message = "portal closed at cold start*" }\')',
    )
    .replace(
      'this.apiHandlerPortalClosedAlarm = new cloudwatch.Alarm(',
      "new logs.MetricFilter(this, 'OtherFilter', { logGroup: apiHandlerLogGroup, filterPattern: logs.FilterPattern.stringValue('$.fields.message', '=', 'portal closed at cold start*'), metricNamespace: 'X', metricName: 'Y' });\nthis.apiHandlerPortalClosedAlarm = new cloudwatch.Alarm(",
    );

  assert.notEqual(swapped, stackSource);
  assert.equal(portalClosedFilter(swapped), undefined);
});
