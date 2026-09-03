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
 *   3. The alarm strip's coverage, published as the `DashboardAlarmCount`
 *      output, equals the number of alarm RESOURCES in this template plus the
 *      nine per-worker `-errors` alarms owned by EventBridgeStack and imported
 *      by ARN. Deriving the expected number from the template's own resources
 *      rather than hard-coding it is what keeps this honest when the alarm set
 *      changes — an alarm added or removed does not need this file edited, but
 *      an alarm dropped from the strip fails here.
 *   4. `periodOverride` is `inherit`. Its default is `auto`, which silently
 *      overrides every per-widget period and makes the trend rows inert.
 *   5. The dashboard's physical name is unchanged — a rename replaces the CFN
 *      resource and changes the console URL handed to the reviewer.
 *   6. No `LoginProfile` appears anywhere in the template: the read-only viewer
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

const TEMPLATE = `infra/cdk.out/Prices-${envName}-Observability.template.json`;

/**
 * Per-worker `-errors` alarms created by `createWorkerLambda` and owned by
 * EventBridgeStack. They are imported into the strip by ARN (no cross-stack
 * CFN reference), so they are not resources in THIS template and have to be
 * added to the expected count by hand. Keep in step with the worker list in
 * `observability-stack.ts`.
 */
const IMPORTED_WORKER_ERROR_ALARMS = 9;

/** Floor from the live account on 2026-09-03: 40 own + 9 imported. */
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
  ['volume-zero caveat', 'permanent by design'],
  // Oracle.
  ['oracle rows written', 'OracleRowsWritten'],
  ['oracle timestamps rejected', 'OracleTimestampRejected'],
  // The Decision A substitution has to be readable ON the dashboard, because
  // the reviewer reads the dashboard, not the evidence file.
  ['Decision A substitution', 'ADR 0007'],
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

// -- 3. the alarm strip is complete and derived -----------------------------
const ownAlarms = Object.values(template.Resources ?? {}).filter(
  (r) => r.Type === 'AWS::CloudWatch::Alarm',
).length;
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

// -- 4. the physical name is unchanged --------------------------------------
const expectedName = `prices-${envName}-overview`;
if (dashboard.Properties.DashboardName !== expectedName) {
  failures.push(
    `dashboard name is "${dashboard.Properties.DashboardName}", expected "${expectedName}" — ` +
      'a rename replaces the CFN resource and changes the console URL handed to the Stellar reviewer',
  );
}

// -- 5. no password in the template -----------------------------------------
if (JSON.stringify(template).includes('LoginProfile')) {
  failures.push(
    'template contains a LoginProfile — the viewer identity must carry no password in source control; ' +
      'the console login is created out of band with `aws iam create-login-profile --password-reset-required`',
  );
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
  `  alarm strip covers ${declared} alarms (${ownAlarms} own + ${IMPORTED_WORKER_ERROR_ALARMS} imported)`,
);
console.log('  periodOverride inherit, name unchanged, no login profile');
