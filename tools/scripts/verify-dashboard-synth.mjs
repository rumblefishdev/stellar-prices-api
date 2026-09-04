#!/usr/bin/env node
/**
 * Assert the synthesized `prices-<env>-overview` dashboard actually points at
 * metrics that exist, covers every alarm in the account, and ships no password.
 *
 * WHY THIS EXISTS
 * ---------------
 * A CloudWatch dashboard has no failure mode that is loud. A metric name that
 * does not exist, a dimension pair that has never published, an alarm dropped
 * from the strip, a per-widget period silently overridden by the dashboard's
 * default `periodOverride: auto` — none of these error at synth, at deploy, or
 * at render. They produce an empty panel, or a panel that quietly covers less
 * than its title claims. Task 0125's acceptance criterion 1 is "no empty
 * panels", and the only thing standing between that and a green deploy is a
 * human noticing. This script is that human, run by CI.
 *
 * The dashboard's whole audience is a Stellar reviewer checking Tranche 3
 * AC 8, so an empty panel is not a cosmetic defect — it reads as an
 * unmonitored system.
 *
 * WHAT IT CHECKS
 * --------------
 *   1. Every metric name, namespace and dimension pair the §9 topic list needs
 *      is present in the dashboard body.
 *   2. The two dimension pairs known to have NEVER published a datapoint are
 *      absent: `Stream=sdex_archive` (only `soroban_amm` has ever emitted —
 *      the probe defines both constants) and the `cleanup` worker's
 *      `FunctionName` (its EventBridge rule is disabled on purpose, so it has
 *      no Lambda metrics at all). Either would render as "No data".
 *   3. No widget asks for a percentile statistic (p50/p95/...) on a
 *      `Prices/Ingest` metric unless the ledger-processor publishes that metric
 *      as raw `Values`. CloudWatch cannot compute a percentile from a
 *      `StatisticSet`, so the panel renders "No data" with nothing failing —
 *      the exact defect this script exists to catch, invisible to check 1
 *      because the metric name IS in the body; only the statistic is unusable.
 *   4. The alarm strip's coverage, published as the `DashboardAlarmCount`
 *      output, equals the number of alarm RESOURCES in this template plus the
 *      nine per-worker `-errors` alarms owned by EventBridgeStack and imported
 *      by ARN. Deriving the expected number from the template's own resources
 *      rather than hard-coding it is what keeps this honest when the alarm set
 *      changes — an alarm added or removed does not need this file edited, but
 *      an alarm dropped from the strip fails here.
 *   5. `periodOverride` is `inherit`. Its default is `auto`, which silently
 *      overrides every per-widget period and makes the trend rows inert.
 *   6. The dashboard's physical name is unchanged — a rename replaces the CFN
 *      resource and changes the console URL handed to the reviewer.
 *   7. No `LoginProfile` appears anywhere in the template: the read-only viewer
 *      identity must carry no password in source control.
 *
 * USAGE
 *   npm run infra:synth:production && npm run infra:verify-dashboard
 *   node tools/scripts/verify-dashboard-synth.mjs --env production
 *
 * Reads only the synthesized template — no AWS credentials, no network.
 */

import { readFileSync } from 'node:fs';

const args = process.argv.slice(2);
const envIdx = args.indexOf('--env');
const envName = envIdx === -1 ? 'production' : args[envIdx + 1];
if (!envName || envName.startsWith('--')) {
  console.error(
    'usage: verify-dashboard-synth.mjs [--env <envName>]   (default: production)',
  );
  process.exit(2);
}

const TEMPLATE = `infra/cdk.out/Prices-${envName}-Observability.template.json`;

/**
 * Floor from the live account on 2026-09-03: 40 own + 9 imported. A
 * deliberately hand-maintained number — it exists to make an alarm REMOVAL a
 * decision someone edits here, not a synth detail.
 */
const MIN_ALARMS = 49;

let template;
try {
  template = JSON.parse(readFileSync(TEMPLATE, 'utf8'));
} catch (err) {
  console.error(`FAIL: could not read ${TEMPLATE}`);
  console.error(`      run \`npm run infra:synth:${envName}\` first.`);
  console.error(`      ${err.message}`);
  process.exit(1);
}

const dashboards = Object.entries(template.Resources ?? {}).filter(
  ([, r]) => r.Type === 'AWS::CloudWatch::Dashboard',
);
if (dashboards.length !== 1) {
  console.error(
    `FAIL: expected exactly one AWS::CloudWatch::Dashboard, found ${dashboards.length}`,
  );
  process.exit(1);
}
const dashboard = dashboards[0][1];

/**
 * The body is a plain string while it holds only literals, and an `Fn::Join`
 * object once alarm ARNs (which carry region/account tokens) are embedded.
 * Stringify the non-string case and strip backslashes so quoted fragments in
 * an escaped JSON string compare equal to the same fragments in a plain one.
 */
const rawBody = dashboard.Properties.DashboardBody;
const body = (
  typeof rawBody === 'string' ? rawBody : JSON.stringify(rawBody)
).replace(/\\/g, '');

const failures = [];

// -- 1. every §9 topic reaches a real metric --------------------------------
const required = [
  // Ingestion lag + the ledger-processor's own write path.
  ['AWS/SQS ingest queue age', '"ApproximateAgeOfOldestMessage"'],
  ['ingest DLQ depth', '"ApproximateNumberOfMessagesVisible"'],
  ['ClickHouse write latency', 'ClickHouseWriteLatencyMs'],
  ['ingest namespace', 'Prices/Ingest'],
  // API latency, error rate, cache hit ratio (the M2 acceptance criteria).
  ['API name dimension', `prices-${envName}-api`],
  ['API latency', '"Latency"'],
  ['API 5xx', '"5XXError"'],
  ['cache hits', 'CacheHitCount'],
  ['cache misses', 'CacheMissCount'],
  // ClickHouse host + rollups + backfill + mTLS.
  ['ClickHouse free disk', 'ClickHouseDiskFreePercent'],
  ['rollup lag', 'RollupLagSeconds'],
  ['backfill push age, AMM stream', '"Stream","soroban_amm"'],
  ['mTLS days to expiry', 'MinDaysToNotAfter'],
  // Enrichment: all six metrics (an acceptance criterion inherited from 0026).
  ['enrichment rows enriched', 'EnrichmentRowsEnriched'],
  ['enrichment oracle misses', 'EnrichmentOracleMiss'],
  ['enrichment recent backlog', 'EnrichmentRowsRemainingRecent'],
  ['enrichment volume-zero floor', 'EnrichmentRowsRemainingAtVolumeZero'],
  ['enrichment pass duration', 'EnrichmentPassDurationMs'],
  ['enrichment avg batch duration', 'EnrichmentAvgBatchDurationMs'],
  // The volume-zero floor must be labelled, not rendered as a queue depth.
  ['volume-zero caveat', 'by design and is not a backlog'],
  // Oracle.
  ['oracle rows written', 'OracleRowsWritten'],
  ['oracle timestamps rejected', 'OracleTimestampRejected'],
  // A quiet hour must draw zeros, not gaps, where absence means zero.
  ['5xx filled to zero', 'FILL(e5xx, 0)'],
  ['DLQ depth filled to zero', 'FILL(dlq, 0)'],
  // Trend rows only work with the override set.
  ['period override', '"periodOverride":"inherit"'],
];
for (const [label, fragment] of required) {
  if (!body.includes(fragment)) {
    failures.push(`missing ${label}: ${fragment}`);
  }
}

// -- 2. the two known empty-panel dimension pairs ---------------------------
const banned = [
  [
    'sdex_archive backfill stream (never published a datapoint)',
    '"Stream","sdex_archive"',
  ],
  [
    'cleanup worker (EventBridge rule disabled — no Lambda metrics at all)',
    `"FunctionName","prices-${envName}-cleanup"`,
  ],
];
for (const [label, fragment] of banned) {
  if (body.includes(fragment)) {
    failures.push(`known empty panel present — ${label}: ${fragment}`);
  }
}

// -- 3. percentile stats only on metrics published as raw values ------------
/**
 * `[["Prices/Ingest","MetricName","Environment","production",{...,"stat":"p95"}]]`
 * after the backslash strip above. `[^\]]*` keeps the match inside one metric
 * entry, so a percentile belonging to a neighbouring widget cannot be
 * attributed to this namespace.
 */
const INGEST_SOURCE = 'packages/prices-ledger-processor/src/metrics.rs';
const percentileOnIngest = [
  ...body.matchAll(
    /"Prices\/Ingest","([A-Za-z0-9_]+)"[^\]]*"stat":"(p\d+(?:\.\d+)?)"/g,
  ),
].map(([, metric, stat]) => `${metric} (${stat})`);

if (percentileOnIngest.length === 0) {
  failures.push(
    'no widget asks for a percentile statistic on a Prices/Ingest metric — the write-latency p50/p95 ' +
      'panels are gone, so the raw-values check below has nothing to protect (it must not pass vacuously)',
  );
}

if (percentileOnIngest.length > 0) {
  let ingestSource;
  try {
    ingestSource = readFileSync(INGEST_SOURCE, 'utf8');
  } catch (err) {
    failures.push(
      `widgets ask for a percentile on Prices/Ingest (${[...new Set(percentileOnIngest)].join(', ')}) ` +
        `but ${INGEST_SOURCE} could not be read to confirm how those metrics are published: ${err.message}`,
    );
    ingestSource = null;
  }
  if (ingestSource !== null) {
    const unique = [...new Set(percentileOnIngest)].join(', ');
    if (ingestSource.includes('statistic_values(')) {
      failures.push(
        `${INGEST_SOURCE} publishes via statistic_values(), but the dashboard asks for ${unique} — ` +
          'a percentile stat on a StatisticSet-published metric renders "No data" forever ' +
          '(CloudWatch keeps no distribution behind SampleCount/Sum/Minimum/Maximum). ' +
          'Publish raw values (set_values) or drop the percentile from the widget.',
      );
    }
    if (!ingestSource.includes('set_values(')) {
      failures.push(
        `the dashboard asks for ${unique} but ${INGEST_SOURCE} never calls set_values() — ` +
          'a percentile stat on a metric not published as raw values renders "No data" forever.',
      );
    }
  }
}

// -- 3b. the top-row tiles read a 24-hour window ---------------------------
/**
 * At the dashboard's 3-hour default a quiet night shows `--` on a single-value
 * tile (no requests, no datapoint). Every tile in the top row carries its own
 * 24-hour window so the number is always there. Split on widget boundaries —
 * a metrics array holds `{...}` option objects, so a `[^}]*` span cannot be
 * trusted to stay inside one widget.
 */
const widgets = body.split('{"type":"');
const singleValue24h = widgets.filter(
  (w) => w.includes('"view":"singleValue"') && w.includes('"start":"-P1D"'),
).length;
const TOP_ROW_TILES = 4;
if (singleValue24h < TOP_ROW_TILES) {
  failures.push(
    `expected ${TOP_ROW_TILES} single-value tiles with a 24-hour window ("start":"-P1D"), found ${singleValue24h} — ` +
      'a quiet 3-hour range renders the others as "--"',
  );
}

// -- 3c. threshold lines match the alarm config -----------------------------
/**
 * The annotation lines are built from the same `opsAlarms` keys as the alarms.
 * Read the env config and require each threshold to appear as an annotation
 * value in the body, so a threshold changed in config without a synth shows
 * up here, and a hard-coded line that drifted from its alarm fails.
 */
const CONFIG = `infra/envs/${envName}.json`;
let opsAlarms;
try {
  opsAlarms = JSON.parse(readFileSync(CONFIG, 'utf8')).opsAlarms;
} catch (err) {
  failures.push(
    `could not read ${CONFIG} to check annotation thresholds: ${err.message}`,
  );
}
if (opsAlarms) {
  // Each threshold must sit on ITS widget (matched by title), not merely
  // somewhere in the body — otherwise swapping the lag and disk lines would
  // pass (deep review WR-06).
  const annotationValuesOf = (titleFragment) => {
    const w = widgets.find((x) => x.includes(`"title":"${titleFragment}`));
    if (!w) return null;
    const m = w.match(/"annotations":\{"horizontal":\[(.*?)\]\}/);
    return m
      ? [...m[1].matchAll(/"value":(-?\d+(?:\.\d+)?)/g)].map(([, n]) =>
          Number(n),
        )
      : [];
  };
  for (const [key, titleFragment] of [
    ['ledgerProcessorLagSeconds', 'Ingest queue'],
    ['chDiskFreePercent', 'ClickHouse host'],
  ]) {
    const values = annotationValuesOf(titleFragment);
    if (values === null) {
      failures.push(
        `no widget titled "${titleFragment}…" found for the ${key} threshold line`,
      );
    } else if (!values.includes(Number(opsAlarms[key]))) {
      failures.push(
        `no threshold line at ${opsAlarms[key]} (opsAlarms.${key}) on the "${titleFragment}…" widget — ` +
          `annotation values there: [${values.join(', ')}]`,
      );
    }
  }
  const mtls = `alarm below ${opsAlarms.mtlsNotAfterDaysThreshold}`;
  if (!body.includes(mtls)) {
    failures.push(
      `mTLS tile title does not state the alarm threshold ("${mtls}")`,
    );
  }
}

// -- 4. the alarm strip is complete and derived -----------------------------
const ownAlarms = Object.values(template.Resources ?? {}).filter(
  (r) => r.Type === 'AWS::CloudWatch::Alarm',
).length;
// Per-worker `-errors` alarms are owned by EventBridgeStack and imported into
// the strip by ARN, so they are not resources in this template. Their count
// is read off the body (distinct `prices-<env>-<worker>-errors` names inside
// the alarm widgets) rather than kept as a constant — a constant was the
// second hand-maintained copy of the worker list (deep review WR-02).
const importedNames = new Set(
  [
    ...body.matchAll(new RegExp(`prices-${envName}-([a-z0-9-]+)-errors`, 'g')),
  ].map(([, w]) => w),
);
const IMPORTED_WORKER_ERROR_ALARMS = importedNames.size;
if (IMPORTED_WORKER_ERROR_ALARMS === 0) {
  failures.push(
    'no imported worker -errors alarm found in the dashboard body — the EventBridgeStack import is gone',
  );
}
const declared = Number(template.Outputs?.DashboardAlarmCount?.Value);
const expected = ownAlarms + IMPORTED_WORKER_ERROR_ALARMS;

if (!Number.isFinite(declared)) {
  failures.push('output DashboardAlarmCount is missing or not a number');
} else if (declared !== expected) {
  failures.push(
    `alarm strip covers ${declared}, expected ${expected} ` +
      `(${ownAlarms} alarm resources in this template + ${IMPORTED_WORKER_ERROR_ALARMS} imported worker -errors alarms). ` +
      'An alarm was constructed after the widget block, or the strip stopped being derived.',
  );
} else if (declared < MIN_ALARMS) {
  failures.push(
    `alarm strip covers ${declared}, below the ${MIN_ALARMS} known live on 2026-09-03 — alarms were removed, which is a decision, not a synth detail`,
  );
}

// -- 5. the physical name is unchanged --------------------------------------
const expectedName = `prices-${envName}-overview`;
if (dashboard.Properties.DashboardName !== expectedName) {
  failures.push(
    `dashboard name is "${dashboard.Properties.DashboardName}", expected "${expectedName}" — ` +
      'a rename replaces the CFN resource and changes the console URL handed to the Stellar reviewer',
  );
}

// -- 6. no password in the template -----------------------------------------
for (const bad of ['LoginProfile', 'AWS::IAM::AccessKey']) {
  if (JSON.stringify(template).includes(bad)) {
    failures.push(
      `template contains ${bad} — the viewer identity must carry no credential in source control; ` +
        'the console login is created out of band with `aws iam create-login-profile --password-reset-required`',
    );
  }
}
// CR-01: the VIEWER must not carry CloudWatchReadOnlyAccess — it grants
// logs:FilterLogEvents / logs:Get* and xray:Get* account-wide. Scoped to the
// viewer user: the AWS Chatbot role in this same stack legitimately carries
// that policy, so a template-wide grep would always fail.
const viewerUsers = Object.entries(template.Resources ?? {}).filter(
  ([, r]) =>
    r.Type === 'AWS::IAM::User' &&
    String(r.Properties?.UserName ?? '').endsWith('-stellar-viewer'),
);
if (viewerUsers.length !== 1) {
  failures.push(
    `expected exactly one *-stellar-viewer IAM user, found ${viewerUsers.length}`,
  );
} else {
  const [viewerId, viewer] = viewerUsers[0];
  const managed = JSON.stringify(viewer.Properties?.ManagedPolicyArns ?? []);
  if (
    managed.includes('CloudWatchReadOnlyAccess') ||
    managed.includes('ReadOnlyAccess')
  ) {
    failures.push(
      'viewer user carries a managed *ReadOnlyAccess policy — that reads every log group and trace ' +
        'in the shared account; use the scoped dashboard-read inline policy',
    );
  }
  const attached = Object.values(template.Resources ?? {}).filter(
    (r) =>
      r.Type === 'AWS::IAM::Policy' &&
      JSON.stringify(r.Properties?.Users ?? []).includes(viewerId),
  );
  const actions = attached.flatMap((p) =>
    p.Properties.PolicyDocument.Statement.flatMap((s) => [].concat(s.Action)),
  );
  const forbidden = actions.filter(
    (a) => !/^cloudwatch:(Get|List|Describe)/.test(a),
  );
  if (attached.length === 0) {
    failures.push(
      'viewer user has no inline policy — it could not open the dashboard',
    );
  } else if (forbidden.length > 0) {
    failures.push(
      `viewer inline policy grants non-read or non-CloudWatch actions: ${forbidden.join(', ')}`,
    );
  }
}

if (failures.length > 0) {
  console.error(`FAIL: ${TEMPLATE}`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}

console.log(`PASS: ${expectedName}`);
console.log(`  ${required.length} required metric/dimension fragments present`);
console.log(`  ${banned.length} known empty-panel dimension pairs absent`);
console.log(
  `  ${TOP_ROW_TILES} top-row tiles on a 24-hour window; 2 series filled to zero; threshold lines match opsAlarms per widget`,
);
console.log(
  `  ${percentileOnIngest.length} percentile stat(s) on Prices/Ingest backed by raw-value publication`,
);
console.log(
  `  alarm strip covers ${declared} alarms (${ownAlarms} own + ${IMPORTED_WORKER_ERROR_ALARMS} imported)`,
);
console.log('  periodOverride inherit, name unchanged, no login profile');
