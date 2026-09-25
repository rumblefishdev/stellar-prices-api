// The portal-load-failed alarm (tasks 0249, 0311) hangs on one string.
// ObservabilityStack's metric filter matches
// `{ $.fields.message = "portal sources failed to load*" }` on the api-handler
// log group; `packages/prices-api/src/portal/sources.rs` is what logs it, and
// `packages/prices-api/src/main.rs` owns the JSON subscriber that puts the
// message under `fields`. Nothing else ties the three together: reword the log
// line, flatten the subscriber, or point the alarm at another metric, and the
// alarm goes quiet for good with every check green. Asserted here:
//
//   - the filter reads `$.fields.message` and matches by the exact prefix;
//   - `sources.rs` has a `tracing::error!` whose message starts with it;
//   - the subscriber in `main.rs` is `fmt().json()` and not flattened;
//   - the alarm takes its metric from that filter, under its own name;
//   - the alarm's description fits CloudWatch's 1024-character limit.
//
// The pattern shape was proven against AWS's evaluator with
// `aws logs test-metric-filter` (task 0249's notes); this guards the drift.
//
// Run: npx nx test @rumblefish/stellar-prices-api-aws-cdk

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const PREFIX = 'portal sources failed to load';

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
const sourcesSource = readFileSync(
  join(repoRoot, 'packages', 'prices-api', 'src', 'portal', 'sources.rs'),
  'utf8',
);

// The filter's own construct: `const <var> = new logs.MetricFilter(` through
// the `);` that closes it. Searching inside that block only: an unbounded
// search would run on to the FIRST `stringValue(...)` anywhere later in the
// file, so a second filter added after this one could stand in for it and the
// guard would pass on the wrong filter.
const loadFailedFilterBlock = (source) => {
  const block = source.match(
    /const (\w+) = new logs\.MetricFilter\(\s*this,\s*'ApiHandlerPortalLoadFailedFilter',([\s\S]*?)\n\s*\);/,
  );
  return block ? { variable: block[1], body: block[2] } : undefined;
};

// `FilterPattern.stringValue('<key>', '=', '<prefix>*')` inside that block →
// { key, prefix }, or undefined when the filter is not a prefix match on a key.
const loadFailedFilter = (source) => {
  const filter = loadFailedFilterBlock(source)?.body.match(
    /FilterPattern\.stringValue\(\s*'([^']+)',\s*'='\s*,\s*'([^']+)\*',?\s*\)/,
  );
  return filter ? { key: filter[1], prefix: filter[2] } : undefined;
};

// The alarm's construct, from its id to the `);` that closes it.
const loadFailedAlarmBlock = (source) => {
  const block = source.match(
    /new cloudwatch\.Alarm\(\s*this,\s*'ApiHandlerPortalLoadFailedAlarm',([\s\S]*?)\n\s*\);/,
  );
  return block ? block[1] : undefined;
};

// The alarm's `alarmDescription`, backtick or quoted.
const alarmDescription = (block) => {
  const found = block?.match(
    /alarmDescription:\s*(?:`([^`]*)`|'([^']*)'|"([^"]*)")/,
  );
  return found ? (found[1] ?? found[2] ?? found[3]) : undefined;
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

test('the portal-load-failed filter matches $.fields.message by the exact prefix', () => {
  const filter = loadFailedFilter(stackSource);

  assert.ok(
    filter,
    'found no prefix FilterPattern on the portal-load-failed filter',
  );
  assert.equal(filter.key, '$.fields.message');
  assert.equal(filter.prefix, PREFIX);
});

test('sources.rs logs an error starting with the prefix the filter matches', () => {
  const { prefix } = loadFailedFilter(stackSource);

  assert.ok(
    logsErrorStartingWith(sourcesSource, prefix),
    `no tracing::error! in sources.rs starts with "${prefix}" — the portal-load-failed alarm would never fire`,
  );
});

test('the subscriber in main.rs keeps the message under `fields`', () => {
  assert.ok(
    subscriberPutsMessageUnderFields(mainSource),
    'main.rs must log through fmt().json() without flatten_event(true), or $.fields.message matches nothing',
  );
});

test("the alarm takes its metric from the filter, under the alarm's own name", () => {
  const { variable } = loadFailedFilterBlock(stackSource);
  const alarm = loadFailedAlarmBlock(stackSource);

  assert.ok(alarm, 'found no ApiHandlerPortalLoadFailedAlarm');
  assert.ok(
    alarm.includes(`${variable}.metric(`),
    `the alarm must read ${variable}.metric(…), or it watches another metric`,
  );
  assert.ok(alarm.includes('api-handler-portal-load-failed'));
});

test('the alarm description fits CloudWatch’s 1024-character limit', () => {
  const description = alarmDescription(loadFailedAlarmBlock(stackSource));

  assert.ok(description, 'found no alarmDescription on the alarm');
  assert.ok(
    description.length <= 1024,
    `alarmDescription is ${description.length} characters; CloudWatch refuses more than 1024`,
  );
});

test('a reworded log line is refused', () => {
  const reworded = sourcesSource.replace(
    PREFIX,
    'the portal sources failed to load',
  );

  assert.notEqual(reworded, sourcesSource);
  assert.equal(logsErrorStartingWith(reworded, PREFIX), false);
});

test('a flattened subscriber is refused', () => {
  const flattened = mainSource.replace(
    '.json()',
    '.json().flatten_event(true)',
  );

  assert.notEqual(flattened, mainSource);
  assert.equal(subscriberPutsMessageUnderFields(flattened), false);
});

test('a second filter later in the file cannot stand in for the portal-load-failed one', () => {
  // The filter moves off `stringValue`; another filter with the right
  // pattern is added after it. The alarm now matches nothing.
  const swapped = stackSource
    .replace(
      /filterPattern: logs\.FilterPattern\.stringValue\(\s*'\$\.fields\.message',\s*'=',\s*'portal sources failed to load\*',\s*\)/,
      'filterPattern: logs.FilterPattern.literal(\'{ $.message = "portal sources failed to load*" }\')',
    )
    .replace(
      'this.apiHandlerPortalLoadFailedAlarm = new cloudwatch.Alarm(',
      "new logs.MetricFilter(this, 'OtherFilter', { logGroup: apiHandlerLogGroup, filterPattern: logs.FilterPattern.stringValue('$.fields.message', '=', 'portal sources failed to load*'), metricNamespace: 'X', metricName: 'Y' });\nthis.apiHandlerPortalLoadFailedAlarm = new cloudwatch.Alarm(",
    );

  assert.notEqual(swapped, stackSource);
  assert.equal(loadFailedFilter(swapped), undefined);
});

test('an alarm pointed at another metric is refused', () => {
  const { variable } = loadFailedFilterBlock(stackSource);
  const repointed = stackSource.replace(
    `metric: ${variable}.metric(`,
    'metric: someOtherFilter.metric(',
  );

  assert.notEqual(repointed, stackSource);
  assert.equal(
    loadFailedAlarmBlock(repointed).includes(`${variable}.metric(`),
    false,
  );
});
