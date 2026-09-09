import * as cdk from 'aws-cdk-lib';
import * as cw_actions from 'aws-cdk-lib/aws-cloudwatch-actions';
import * as events from 'aws-cdk-lib/aws-events';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';
import { createWorkerLambda } from '../lambda-baseline.js';
import { opsAlarmsTopicName } from './observability-stack.js';
import {
  mtlsSecretName,
  mtlsSecretArnFromParts,
  secretsManagerLayerArn,
} from '../mtls.js';

/**
 * Cargo-lambda build output for the `asset-discovery` binary (task 0054).
 * Pre-built like the ledger processor (no `cargo-lambda-cdk` dependency yet):
 *
 *     cargo lambda build -p asset-discovery --release --arm64
 *
 * → `target/lambda/asset-discovery/bootstrap`. Override with
 * `ASSET_DISCOVERY_ASSET_DIR`.
 */
const ASSET_DISCOVERY_ASSET_DIR =
  process.env['ASSET_DISCOVERY_ASSET_DIR'] ??
  '../target/lambda/asset-discovery';

/** Cargo-lambda build output for the `cleanup-worker` binary (task 0039). */
const CLEANUP_WORKER_ASSET_DIR =
  process.env['CLEANUP_WORKER_ASSET_DIR'] ?? '../target/lambda/cleanup-worker';

/** Cargo-lambda build output for the `supply-worker` binary (task 0039). */
const SUPPLY_WORKER_ASSET_DIR =
  process.env['SUPPLY_WORKER_ASSET_DIR'] ?? '../target/lambda/supply-worker';

/** Cargo-lambda build output for the `oracle-worker` binary (task 0039). */
const ORACLE_WORKER_ASSET_DIR =
  process.env['ORACLE_WORKER_ASSET_DIR'] ?? '../target/lambda/oracle-worker';

/**
 * Cargo-lambda build output for the `coarse-sweep-worker` binary (task 0218).
 *
 * Its OWN crate, not a bin inside `enrichment-worker`: the Lambda tooling
 * requires crate name == bin name == `target/lambda/<name>`, and a bin in
 * another crate fails both `lambda-assets.sh` and `verify-lambda-assets.sh`
 * (`did not match any packages` / `CannotFindAsset`). The sweep LOGIC is still
 * shared — it is `enrichment_worker::repair`, consumed as a library.
 */
const COARSE_SWEEP_WORKER_ASSET_DIR =
  process.env['COARSE_SWEEP_WORKER_ASSET_DIR'] ??
  '../target/lambda/coarse-sweep-worker';

/** Cargo-lambda build output for the `enrichment-worker` binary (task 0026). */
const ENRICHMENT_WORKER_ASSET_DIR =
  process.env['ENRICHMENT_WORKER_ASSET_DIR'] ??
  '../target/lambda/enrichment-worker';

/** Cargo-lambda build output for the `backfill-freshness-probe` (task 0056). */
const BACKFILL_FRESHNESS_PROBE_ASSET_DIR =
  process.env['BACKFILL_FRESHNESS_PROBE_ASSET_DIR'] ??
  '../target/lambda/backfill-freshness-probe';

/** Cargo-lambda build output for the `rollup-freshness-probe` (task 0137). */
const ROLLUP_FRESHNESS_PROBE_ASSET_DIR =
  process.env['ROLLUP_FRESHNESS_PROBE_ASSET_DIR'] ??
  '../target/lambda/rollup-freshness-probe';

/** Cargo-lambda build output for the `mtls-notafter-probe` (task 0056). */
const MTLS_NOTAFTER_PROBE_ASSET_DIR =
  process.env['MTLS_NOTAFTER_PROBE_ASSET_DIR'] ??
  '../target/lambda/mtls-notafter-probe';

export interface EventBridgeStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * EventBridge rules for the periodic prices-api workers (task 0039),
 * plus the **Asset Discovery** worker Lambda + target (task 0054).
 *
 * Uses `aws-events` `events.Rule` (CloudFormation `AWS::Events::Rule`),
 * not EventBridge Scheduler (`AWS::Scheduler::Schedule`).
 *
 * All four rules now have their worker Lambdas attached (task 0039 / 0054):
 * asset-discovery, asset-supply, oracle-watcher, and cleanup.
 *
 * Two workers from the original 0039 spec are intentionally absent — ADR 0007
 * replaces them with ClickHouse MVs: the **Rollup** worker (§3.4, the rollup
 * MV chain) and the **price-updater** (§Q#1, the `current_prices` MV). So
 * there is no price-updater rule; the former slot is now **asset-supply**.
 */
export class EventBridgeStack extends cdk.Stack {
  public readonly assetSupplyRule: events.Rule;
  public readonly oracleWatcherRule: events.Rule;
  public readonly assetDiscoveryRule: events.Rule;
  public readonly cleanupRule: events.Rule;
  public readonly enrichmentRule: events.Rule;
  public readonly coarseSweepRule: events.Rule;
  public readonly backfillFreshnessProbeRule: events.Rule;
  public readonly rollupFreshnessProbeRule: events.Rule;
  public readonly mtlsNotafterProbeRule: events.Rule;
  public readonly assetDiscoveryFunction: lambda.Function;
  public readonly cleanupFunction: lambda.Function;
  public readonly supplyFunction: lambda.Function;
  public readonly oracleFunction: lambda.Function;
  public readonly enrichmentFunction: lambda.Function;
  public readonly coarseSweepFunction: lambda.Function;
  public readonly backfillFreshnessProbeFunction: lambda.Function;
  public readonly rollupFreshnessProbeFunction: lambda.Function;
  public readonly mtlsNotafterProbeFunction: lambda.Function;

  constructor(scope: Construct, id: string, props: EventBridgeStackProps) {
    super(scope, id, props);

    const { config } = props;
    const env = config.envName;
    const region = config.awsRegion;
    const accountId = this.account;
    const schedules = config.scheduleExpressions;

    // Shared ops SNS action, wired to EVERY worker's `-errors` alarm.
    //
    // createWorkerLambda creates `prices-{env}-{name}-errors` for each worker,
    // but an alarm with no action is inert: it transitions to ALARM and tells
    // nobody. Only the two probes passed `errorAlarmActions`, so the five cron
    // workers had error alarms that could never notify. That is how
    // prices-production-enrichment failed 72/72 invocations a day for four days
    // (2026-07-14 to 07-17) with nobody paged (task 0112).
    //
    // Imported by deterministic name — no cross-stack CFN reference, so the two
    // stacks stay independently deployable (see `opsAlarmsTopicName`).
    const opsAlarmsTopic = sns.Topic.fromTopicArn(
      this,
      'OpsAlarmsTopicRef',
      `arn:aws:sns:${region}:${accountId}:${opsAlarmsTopicName(env)}`,
    );
    const opsAlarmAction = new cw_actions.SnsAction(opsAlarmsTopic);

    this.assetSupplyRule = new events.Rule(this, 'AssetSupplyRule', {
      ruleName: `prices-${env}-asset-supply`,
      description: `Per-asset circulating-supply fetch → asset_supply (${env})`,
      schedule: events.Schedule.expression(schedules.assetSupply),
    });

    this.oracleWatcherRule = new events.Rule(this, 'OracleWatcherRule', {
      ruleName: `prices-${env}-oracle-watcher`,
      description: `Polls Stellar on-chain oracles (${env})`,
      schedule: events.Schedule.expression(schedules.oracleWatcher),
    });

    this.assetDiscoveryRule = new events.Rule(this, 'AssetDiscoveryRule', {
      ruleName: `prices-${env}-asset-discovery`,
      description: `Periodic asset-registry maintenance (${env})`,
      schedule: events.Schedule.expression(schedules.assetDiscovery),
    });

    // 🔴 DISABLED IN CODE, and this line is load-bearing (task 0204, 2026-08-20).
    //
    // The live rule has been DISABLED by operator action since task 0200, but
    // this template asserted `State: ENABLED` — so **every deploy of this stack
    // silently re-enabled cleanup**, and the only thing standing between a
    // routine deploy and a destructive sweep was remembering to check
    // `describe-rule` afterwards. That is not a safeguard, it is a habit.
    //
    // ⚠️ The blast radius has GROWN while the rule sat off. `price_ohlcv_1m`
    // carries a 7-day retention in the cleanup worker, and with cleanup stopped
    // the table now holds years of history — including the 1.56M rows task 0212
    // must repair. One sweep would drop all of it, irreversibly.
    //
    // Whether cleanup should EVER run again is task 0200's decision and this
    // does not pre-empt it: the change only makes CDK state what production
    // already is, so the template can no longer contradict reality. Re-enabling
    // is a deliberate edit here, reviewed, rather than a side effect of
    // deploying something unrelated.
    this.cleanupRule = new events.Rule(this, 'CleanupRule', {
      ruleName: `prices-${env}-cleanup`,
      description: `Old-data partition drop on prices.* tables (${env})`,
      schedule: events.Schedule.expression(schedules.cleanup),
      enabled: false,
    });

    this.enrichmentRule = new events.Rule(this, 'EnrichmentRule', {
      ruleName: `prices-${env}-enrichment`,
      description: `close_usd / volume_quote_usd enrichment of price_ohlcv_1m (${env})`,
      schedule: events.Schedule.expression(schedules.enrichment),
    });

    // Task 0218. Hourly, matching the cadence the sweep effectively had as a
    // stage of the enrichment worker — but on its own rule, so a run that does
    // not happen is visible as zero invocations rather than as silence inside
    // another function's handler.
    //
    // ⚠️ Deliberately OFFSET from the enrichment rule (see
    // `scheduleExpressions.coarseSweep`). Serialized inside one invocation the
    // two never overlapped; on independent schedules they can. Their writes are
    // disjoint so that is safe, but both issue `FINAL` scans against a shared
    // ClickHouse where BE owns most of the volume and has filled it before, so
    // there is no reason to stack their peaks in the same minute.
    this.coarseSweepRule = new events.Rule(this, 'CoarseSweepRule', {
      ruleName: `prices-${env}-coarse-sweep`,
      description: `close_usd / volume_quote_usd sweep of the coarse OHLCV rollups (${env})`,
      schedule: events.Schedule.expression(schedules.coarseSweep),
    });

    this.backfillFreshnessProbeRule = new events.Rule(
      this,
      'BackfillFreshnessProbeRule',
      {
        ruleName: `prices-${env}-backfill-freshness-probe`,
        description: `Publishes backfill_progress push age → Prices/Backfill PushAgeSeconds (${env})`,
        schedule: events.Schedule.expression(schedules.backfillFreshnessProbe),
      },
    );

    this.rollupFreshnessProbeRule = new events.Rule(
      this,
      'RollupFreshnessProbeRule',
      {
        ruleName: `prices-${env}-rollup-freshness-probe`,
        description: `Publishes per-tier OHLCV rollup lag → Prices/Rollup RollupLagSeconds (${env})`,
        schedule: events.Schedule.expression(schedules.rollupFreshnessProbe),
      },
    );

    this.mtlsNotafterProbeRule = new events.Rule(
      this,
      'MtlsNotafterProbeRule',
      {
        ruleName: `prices-${env}-mtls-notafter-probe`,
        description: `Publishes mTLS cert days-to-NotAfter → Prices/Mtls (${env})`,
        schedule: events.Schedule.expression(schedules.mtlsNotafterProbe),
      },
    );

    // -----------------------------------------------------------------
    // Asset Discovery worker Lambda (task 0054) + its rule target.
    // No VPC (ADR 0007 §6); mTLS to ClickHouse + S3 read on BE's ledger
    // bucket, mirroring the ledger processor's conventions.
    // -----------------------------------------------------------------
    const base = `/platform/${env}`;
    const chDomain = ssm.StringParameter.valueForStringParameter(
      this,
      `${base}/ch-domain`,
    );
    const networkPassphrase = ssm.StringParameter.valueForStringParameter(
      this,
      `${base}/stellar-network-passphrase`,
    );
    const ledgerBucketName = ssm.StringParameter.valueForStringParameter(
      this,
      `${base}/stellar-ledger-data-bucket-name`,
    );
    const ledgerBucketArn = ssm.StringParameter.valueForStringParameter(
      this,
      `${base}/stellar-ledger-data-bucket-arn`,
    );

    // Writes prices.assets → the same `ingestion`-class mTLS identity the
    // ledger processor uses (created out-of-band by the operator).
    const discoveryMtlsSecretName = mtlsSecretName(env, 'ingestion');

    const secretsExtensionLayer = lambda.LayerVersion.fromLayerVersionArn(
      this,
      'SecretsExtensionLayer',
      secretsManagerLayerArn(region),
    );

    const discovery = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'AssetDiscovery',
      name: 'asset-discovery',
      errorAlarmActions: [opsAlarmAction],
      assetDir: ASSET_DISCOVERY_ASSET_DIR,
      memorySize: 512,
      // Bounded by MAX_LEDGERS in the binary; a catch-up run fetches+decodes
      // many S3 objects, so allow generous headroom under the 1h cadence.
      timeout: cdk.Duration.minutes(5),
      secretsExtensionLayer,
      chDomain,
      rule: this.assetDiscoveryRule,
      environment: {
        // Source bucket for ledger XDR objects (Galexie key scheme).
        BUCKET_NAME: ledgerBucketName,
        STELLAR_NETWORK_PASSPHRASE: networkPassphrase,
        // NB: INITIAL_DISCOVERY_LEDGER is intentionally NOT set here — the
        // binary seeds gracefully without it and only scans once a
        // `prices.discovery_state` cursor exists. Operator activates the
        // ledger scan as a deploy-prep step (seed the cursor or set the
        // env), so synth is not gated on an operator value.
      },
      // Informational — registry maintenance is non-critical (a failed run
      // just defers new-asset pickup to the next hour).
      alarmDescription:
        'Asset Discovery Lambda invocation errors (informational; registry maintenance is non-critical).',
      alarmPeriod: cdk.Duration.hours(1),
    });
    this.assetDiscoveryFunction = discovery.function;

    // S3 read on BE's ledger bucket (same-account → plain IAM grant, no
    // bucket policy from BE). Imported by attributes; the bucket is SSE-S3
    // (BE task 0306/0278), so no kms:Decrypt is needed.
    const ledgerBucket = s3.Bucket.fromBucketAttributes(this, 'LedgerBucket', {
      bucketArn: ledgerBucketArn,
      bucketName: ledgerBucketName,
    });
    ledgerBucket.grantRead(discovery.role);

    // -----------------------------------------------------------------
    // Cleanup worker Lambda (task 0039) + its cron target. CH-only (no S3,
    // no VPC): issues ALTER TABLE … DROP PARTITION over mTLS per §3.6.
    // Reuses the same `ingestion` mTLS identity + secrets extension layer
    // as the discovery worker.
    // -----------------------------------------------------------------
    this.cleanupFunction = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'Cleanup',
      name: 'cleanup',
      errorAlarmActions: [opsAlarmAction],
      assetDir: CLEANUP_WORKER_ASSET_DIR,
      memorySize: 256,
      // DROP PARTITION is metadata-only; the run is a handful of queries.
      timeout: cdk.Duration.minutes(2),
      secretsExtensionLayer,
      chDomain,
      rule: this.cleanupRule,
      alarmDescription:
        'Cleanup Lambda invocation errors (retention partition-drop failed).',
      alarmPeriod: cdk.Duration.days(1),
    }).function;

    // -----------------------------------------------------------------
    // Supply worker Lambda (task 0039) + its rate(1h) target. CH mTLS +
    // public Horizon egress (no S3, no VPC). Fills prices.asset_supply that
    // the current_prices MV multiplies by the live price for market_cap.
    // Reuses the ingestion mTLS identity + secrets extension layer.
    // -----------------------------------------------------------------
    this.supplyFunction = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'Supply',
      name: 'supply',
      errorAlarmActions: [opsAlarmAction],
      assetDir: SUPPLY_WORKER_ASSET_DIR,
      memorySize: 512,
      // Sequential Horizon GETs across the asset registry. The worker walks the
      // stalest assets first inside a wall-clock budget (task 0084), so a single
      // run finishes cleanly under this timeout and successive hourly runs
      // round-robin the whole registry — no single invoke needs the full walk.
      timeout: cdk.Duration.minutes(5),
      secretsExtensionLayer,
      chDomain,
      rule: this.assetSupplyRule,
      environment: {
        // Stop the Horizon walk at 240 s, 60 s under the 300 s Lambda timeout,
        // so a run never ends `Status: timeout`. Remaining stalest assets defer
        // to the next tick.
        SUPPLY_TIME_BUDGET_SECS: '240',
      },
      // Best-effort + self-resuming: a failed run must not be async-retried 2×
      // more (each a full multi-minute walk); the next schedule picks up the
      // stalest assets anyway (task 0084). Applied to the Lambda's async invoke
      // config (function-error retries), not just the EventBridge target.
      asyncRetryAttempts: 0,
      // HORIZON_URL unset → the binary's public-Horizon default.
      alarmDescription:
        'Supply Lambda invocation errors (informational; supply is best-effort, market_cap degrades to 0).',
      alarmPeriod: cdk.Duration.hours(1),
    }).function;

    new cdk.CfnOutput(this, 'AssetSupplyRuleArn', {
      value: this.assetSupplyRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'SupplyFunctionName', {
      value: this.supplyFunction.functionName,
    });
    new cdk.CfnOutput(this, 'OracleWatcherRuleArn', {
      value: this.oracleWatcherRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'AssetDiscoveryRuleArn', {
      value: this.assetDiscoveryRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'AssetDiscoveryFunctionName', {
      value: this.assetDiscoveryFunction.functionName,
    });
    new cdk.CfnOutput(this, 'CleanupRuleArn', {
      value: this.cleanupRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'CleanupFunctionName', {
      value: this.cleanupFunction.functionName,
    });

    // -----------------------------------------------------------------
    // Oracle worker Lambda (task 0039) + its rate(5m) target. CH mTLS +
    // public Soroban-RPC egress (no S3, no VPC). Polls the Reflector SEP-40
    // oracle and writes prices.oracle_prices. Non-critical (§2.2).
    // -----------------------------------------------------------------
    const oracle = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'Oracle',
      name: 'oracle',
      errorAlarmActions: [opsAlarmAction],
      assetDir: ORACLE_WORKER_ASSET_DIR,
      memorySize: 256,
      timeout: cdk.Duration.minutes(2),
      secretsExtensionLayer,
      chDomain,
      rule: this.oracleWatcherRule,
      // SOROBAN_RPC_URL / REFLECTOR_CONTRACT unset → the binary's mainnet
      // defaults (public Soroban RPC + the Reflector CEX/DEX oracle).
      alarmDescription:
        'Oracle Lambda invocation errors (informational; oracle is non-critical — §2.2 — and degrades to last-known value).',
      alarmPeriod: cdk.Duration.minutes(5),
    });
    this.oracleFunction = oracle.function;

    // The worker publishes its pass metrics (OracleRuns, OracleFailedRuns,
    // OracleSymbolsQueried, OracleRowsWritten, OracleRowsSkipped,
    // OracleTimestampRejected, OracleUsdRatesSnapshotted) under the
    // `Prices/Oracle` namespace (task 0231). PutMetricData has no
    // resource-level scoping, so it is `*` constrained to that namespace, as
    // the enrichment role below already is. The ObservabilityStack's
    // `-dark-feed` and `-timestamp-rejected` alarms read these.
    //
    // ⚠️ The namespace here, the `METRIC_NAMESPACE` constant in
    // `oracle-worker/src/metrics.rs`, and the alarms' `namespace` must agree
    // exactly. A mismatch is invisible at deploy time: the publish fails closed
    // at runtime (logged as a non-fatal warning) and the alarms read a series
    // that never appears, which CloudWatch reports as 0 datapoints rather than
    // as an error (task 0204).
    oracle.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'PublishOracleMetrics',
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: { 'cloudwatch:namespace': 'Prices/Oracle' },
        },
      }),
    );

    new cdk.CfnOutput(this, 'OracleFunctionName', {
      value: this.oracleFunction.functionName,
    });

    // -----------------------------------------------------------------
    // Enrichment worker Lambda (task 0026) + its hourly cron target. CH-only
    // (no S3, no Horizon, no VPC): reads price_ohlcv_1m + oracle_prices and
    // re-inserts higher-`version` rows with close_usd / volume_quote_usd that
    // the ReplacingMergeTree collapses on merge. Writes prices.* → the same
    // `ingestion` mTLS identity the other writers use. Idempotency comes from
    // the `FINAL WHERE … = 0` read filter, so concurrency is not pinned.
    // -----------------------------------------------------------------
    const enrichment = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'Enrichment',
      name: 'enrichment',
      errorAlarmActions: [opsAlarmAction],
      assetDir: ENRICHMENT_WORKER_ASSET_DIR,
      memorySize: 512,
      // A bounded pass is MAX_BATCHES × BATCH_SIZE rows of set-based
      // INSERT…SELECT; generous headroom under the hourly cadence (overflow
      // just defers to the next run). The one-shot historical drain
      // (ENRICHMENT_ONE_SHOT=true) is NOT this hourly function — it would run far
      // longer than 5 min and must not be set here (every cron run would then be
      // unbounded). A dedicated one-time invocation with its own longer timeout
      // is deploy-time work (task 0026 Option 3 / spec §4).
      timeout: cdk.Duration.minutes(5),
      secretsExtensionLayer,
      chDomain,
      rule: this.enrichmentRule,
      environment: {
        CLICKHOUSE_DATABASE: 'prices',
        CLICKHOUSE_TABLE: 'price_ohlcv_1m',
        // ORACLE_NAME / FORWARD_FILL_WINDOW_S / PIVOT_WINDOW_S / BATCH_SIZE /
        // MAX_BATCHES unset → the binary's ChEnrichConfig defaults
        // (reflector / 300 / 86400 / 10000 / 20). ENRICHMENT_ONE_SHOT is left
        // unset (false) here — it belongs only on a dedicated one-time drain
        // invocation, never this hourly target (see the timeout note above).
        // NOTE: the coarse-table USD sweep (task 0114) used to run here as a
        // stage of this worker. It moved to its own function and schedule in
        // task 0218 — see CoarseSweepFunction below. Do NOT re-add the
        // COARSE_SWEEP_* vars here: two schedules running the same sweep would
        // be two writers to the same coarse tables.
        // --- Task 0111: partition-bounded passes -------------------------
        // Monthly partitions the scheduled 1m pass scans, newest-first
        // (current + previous). Unbounded, every statement re-scanned all 102
        // partitions — 736M rows / 18.4 GiB — to serve a live window of 17M,
        // which walked Duration to the 300s timeout and fired
        // prices-production-enrichment-duration-near-timeout at 300,338 ms.
        // Measured 2026-08-21: an oracle batch cost 35.5s and a peg-pivot batch
        // 82.3s, so an invocation budgeting 40 batches achieved 5.4.
        // '0' restores the pre-0111 unbounded pass without a code change.
        // Widen this only if ingest starts landing rows older than the previous
        // month — NOT to cover the historical drain, which is the sweep below.
        ENRICH_LIVE_PARTITIONS: '2',
        // Frontier-driven historical drain (task 0111 phase 2). Walks the
        // partitions BELOW the live window oldest-first, one month per run,
        // recording progress in prices.enrichment_frontier — which must exist
        // (created by hand, like every other table here; the worker has no DDL
        // grant). Self-disables unless ENRICH_LIVE_PARTITIONS > 0, since an
        // unbounded live pass already covers the whole table and the two would
        // contend for the same partitions.
        //
        // Best-effort and time-budgeted, like the coarse sweep: it can never
        // fail the invocation or regress the live pass.
        ENRICH_HISTORICAL_SWEEP: 'true',
        // One month per invocation. A month that still has backlog stays
        // `pending` and resumes next run, so throughput comes from the hourly
        // cadence rather than from doing many months at once — which keeps any
        // single invocation's cost bounded and predictable.
        ENRICH_HISTORICAL_MAX_MONTHS: '1',
        // Wall-clock budget (seconds) for the historical drain. Since task
        // 0218 moved the coarse sweep to its own Lambda this is the only stage
        // after the live pass, so it no longer shares what the pass leaves.
        ENRICH_HISTORICAL_TIME_BUDGET_SECS: '120',
        // How long an `exhausted` mark is trusted before it is re-confirmed
        // against the data (7 days). Without this the frontier would be a
        // permanent verdict: a backfill writing into a partition the sweep had
        // finished (tasks 0088 / 0201 do exactly that) would leave those rows
        // unenriched forever while the frontier read clean — the "skipped rows
        // that look healthy" failure class that cost 26 days in task 0215.
        ENRICH_HISTORICAL_RECHECK_SECS: '604800',
        // Re-checks per invocation, capped separately from MAX_MONTHS so drift
        // correction can never starve the drain. 4/run × 24 runs/day rotates
        // the ~102 partitions roughly daily.
        ENRICH_HISTORICAL_MAX_RECHECKS: '4',
      },
      alarmDescription:
        'Enrichment Lambda invocation errors (close_usd / volume_quote_usd enrichment pass failed).',
      alarmPeriod: cdk.Duration.hours(1),
    });
    this.enrichmentFunction = enrichment.function;

    // The worker publishes the enrichment metrics (EnrichmentRowsEnriched,
    // EnrichmentOracleMiss, EnrichmentRowsRemainingAtVolumeZero,
    // EnrichmentRowsRemainingRecent, EnrichmentPassDurationMs, and a derived
    // EnrichmentAvgBatchDurationMs) under the `Prices/Enrichment` namespace.
    // PutMetricData has no resource-level scoping, so it is `*` constrained to
    // that namespace. The ObservabilityStack alarms on these metrics.
    enrichment.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'PublishEnrichmentMetrics',
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: { 'cloudwatch:namespace': 'Prices/Enrichment' },
        },
      }),
    );

    // -----------------------------------------------------------------
    // Coarse-table USD sweep (task 0218) — split out of the enrichment worker.
    //
    // It used to be the last stage of the enrichment handler, after
    // `pass.run().await?`. That gave it two failure modes it could not escape,
    // and it NEVER executed in production under either: skipped whenever the 1m
    // pass errored (task 0215 had it erroring on every invocation for 26 days),
    // and starved once it stopped erroring, because the sweep's budget is the
    // Lambda deadline minus whatever the pass left.
    //
    // Task 0111 bounded the 1m pass and the sweep began running — but that is
    // the symptom. Any stage after an unbounded stage in one fixed budget is
    // starved by construction, and the next growth in the 1m pass starves it
    // again, silently.
    //
    // ⚠️ The `?` is the deciding reason for a separate function, not the budget:
    // reserving time inside the one Lambda fixes starvation and leaves skipping
    // intact. Only a separate invocation removes the dependency on the 1m pass
    // SUCCEEDING. It also makes a missing run observable — a zero-invocations
    // alarm cannot watch a stage buried inside another function's handler.
    //
    // Writes only the coarse rollups; `price_ohlcv_1m` is refused by
    // `is_coarse_table` in the worker, so this function and the enrichment
    // function have provably disjoint write sets and may run concurrently.
    // (The coarse tables are already written every 60s by mv_ohlcv_1m_to_15m,
    // so concurrent writers are the pre-existing steady state.)
    // -----------------------------------------------------------------
    const coarseSweep = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'CoarseSweep',
      name: 'coarse-sweep',
      errorAlarmActions: [opsAlarmAction],
      assetDir: COARSE_SWEEP_WORKER_ASSET_DIR,
      // Peak usage measured 52-54 MB, so this is over-provisioned — but Lambda
      // CPU scales with memory, and the sweep spends its time on TLS and
      // response parsing against ClickHouse. Cutting memory would cut CPU and
      // could make the ~21s sweep slower for a saving of a few cents a month.
      memorySize: 512,
      // The sweep self-limits to COARSE_SWEEP_TIME_BUDGET_SECS and always stops
      // a margin before this deadline, so the timeout is a backstop, not the
      // control. 5 min matches the enrichment worker.
      timeout: cdk.Duration.minutes(5),
      // A failed sweep must NOT be re-driven twice: the work is idempotent and
      // the next hourly run resumes it anyway, so retries would triple the load
      // on a shared cluster and turn one bad hour into three error datapoints
      // instead of one clean signal. Same reasoning as the 0084 supply worker.
      asyncRetryAttempts: 0,
      secretsExtensionLayer,
      chDomain,
      rule: this.coarseSweepRule,
      environment: {
        // Moved here from the enrichment worker (task 0218). Every rollup table
        // is included so no stored USD value is left wrong. `_15m` is a 30-day
        // rolling window (cleanup-worker RETENTION) unlike {1h,4h,1d,1w,1M}
        // which are retained forever, so the 2-month lookback naturally finds
        // only ~30 days of `_15m`; harmless, the sweep covers whatever exists.
        // `price_ohlcv_1m` is the live base table and is refused by the worker.
        //
        // A non-empty list is still the on switch: clearing it disables the
        // sweep with no code change. The function then still runs and still
        // logs each invocation, so "configured off" stays distinguishable from
        // "never ran" — which is the whole point of task 0218.
        COARSE_SWEEP_TABLES:
          'price_ohlcv_15m,price_ohlcv_1h,price_ohlcv_4h,price_ohlcv_1d,price_ohlcv_1w,price_ohlcv_1M',
        COARSE_SWEEP_LOOKBACK_MONTHS: '2',
        COARSE_SWEEP_MAX_BATCHES: '20',
        COARSE_SWEEP_TIME_BUDGET_SECS: '120',
      },
      alarmDescription:
        'Coarse-table USD sweep Lambda invocation errors (close_usd / volume_quote_usd sweep of the OHLCV rollups failed).',
      alarmPeriod: cdk.Duration.hours(1),
    });
    this.coarseSweepFunction = coarseSweep.function;

    // At most one sweep at a time. At ~21s per run on an hourly schedule an
    // overlap is already near-impossible, so this costs nothing and closes the
    // tail risk of several sweeps stacking against BE's shared ClickHouse.
    const coarseSweepCfn = coarseSweep.function.node
      .defaultChild as lambda.CfnFunction;
    coarseSweepCfn.reservedConcurrentExecutions = 1;

    // Publishes CoarseSweepRowsEnriched / CoarseSweepTableFailures /
    // CoarseSweepTablesSkipped under `Prices/Enrichment`. PutMetricData has no
    // resource-level scoping, so it is `*` constrained to that namespace.
    coarseSweep.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'PublishCoarseSweepMetrics',
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: { 'cloudwatch:namespace': 'Prices/Enrichment' },
        },
      }),
    );

    new cdk.CfnOutput(this, 'CoarseSweepRuleArn', {
      value: this.coarseSweepRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'CoarseSweepFunctionName', {
      value: this.coarseSweepFunction.functionName,
    });

    new cdk.CfnOutput(this, 'EnrichmentRuleArn', {
      value: this.enrichmentRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'EnrichmentFunctionName', {
      value: this.enrichmentFunction.functionName,
    });

    // -----------------------------------------------------------------
    // Both probes' metric alarms (SdexPushFreshnessAlarm / MtlsNotAfterAlarm in
    // ObservabilityStack) are treatMissingData: NOT_BREACHING and so rely on
    // each probe's own `-errors` alarm as the dead-probe backstop. Route those
    // error alarms to the shared ops SNS topic (owned by ObservabilityStack).
    // Imported by deterministic name — no cross-stack CFN reference, so the two
    // stacks stay independently deployable (see `opsAlarmsTopicName`).
    // -----------------------------------------------------------------

    // -----------------------------------------------------------------
    // Backfill freshness probe (task 0056) + its rate(15m) target. CH-only
    // (no S3, no VPC): SELECTs prices.backfill_progress over the ingestion
    // mTLS identity and republishes each stream's push age as the
    // Prices/Backfill PushAgeSeconds metric the SDEX freshness alarm watches.
    // -----------------------------------------------------------------
    const freshness = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'BackfillFreshnessProbe',
      name: 'backfill-freshness-probe',
      assetDir: BACKFILL_FRESHNESS_PROBE_ASSET_DIR,
      memorySize: 256,
      // Two-row SELECT + one PutMetricData; trivially fast.
      timeout: cdk.Duration.minutes(1),
      secretsExtensionLayer,
      chDomain,
      rule: this.backfillFreshnessProbeRule,
      alarmDescription:
        'Backfill freshness probe invocation errors — the SDEX push-age metric may be stale, blinding the freshness alarm.',
      alarmPeriod: cdk.Duration.minutes(15),
      errorAlarmActions: [opsAlarmAction],
    });
    this.backfillFreshnessProbeFunction = freshness.function;

    freshness.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'PublishBackfillMetrics',
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: { 'cloudwatch:namespace': 'Prices/Backfill' },
        },
      }),
    );

    // -----------------------------------------------------------------
    // Rollup freshness probe (task 0137) + its rate(15m) target. CH-only
    // (no S3, no VPC): reads `now() - max(timestamp)` for each OHLCV
    // granularity over the ingestion mTLS identity and republishes it as the
    // Prices/Rollup RollupLagSeconds metric the per-tier rollup alarms watch.
    //
    // Task 0136 froze every coarse table for nine days while the MVs kept
    // reporting success, because rolling up nothing is not an error. This probe
    // measures the data instead of the MV, which is the only signal that could
    // have caught it.
    //
    // ⚠️ The probe is NO LONGER rollup-only. Task 0204 added three more reads
    // to the same invocation, deliberately reusing this schedule and this
    // Prices/Rollup grant so the work stayed out of THIS stack — see the
    // namespace note on the PublishRollupMetrics policy below, and task 0200.
    // Each invocation now does, in this order and each after the previous has
    // published: rollup lag (0137), ClickHouse disk headroom (gap 1), USD-value
    // correctness on the USDT quote leg (gap 4), materialized-view drift
    // (gap 3). A failure in any one is recorded and the rest still run.
    //
    // Still needs nothing beyond the SELECT the ingestion identity already has,
    // which remains deliberate — the runtime users are XML-managed by BE and
    // cannot be SQL-GRANTed by us (task 0134). ⚠️ But the old claim that it
    // "touches no `system.*` table" is now FALSE: the drift read queries
    // `system.tables`. That is fine and needs no grant, because `system.tables`
    // is grant-FILTERED (a prices-only user sees the prices objects, measured at
    // 32 on 26.3.10.60). Contrast `system.disks`, which is grant-DENIED and
    // cannot be granted — which is exactly why the disk read calls filesystem
    // *functions* instead.
    // -----------------------------------------------------------------
    const rollupFreshness = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'RollupFreshnessProbe',
      name: 'rollup-freshness-probe',
      assetDir: ROLLUP_FRESHNESS_PROBE_ASSET_DIR,
      memorySize: 256,
      // ⚠️ NOT REVISITED for the three task-0204 reads, and the reasoning
      // below covers only the original seven. Left at 1 minute on measurement,
      // not on assumption — but verify it rather than trusting this comment.
      //
      // The original seven metadata-only max() reads are answered from per-part
      // min/max indexes rather than a column scan (47 rows / 1.10 KiB / 1 ms on
      // CH 26.3.10.60), so they stay trivially fast even against the 735M-row
      // `price_ohlcv_1m`. Added since:
      //   - disk: two filesystem function calls, no table read;
      //   - USD sanity: a FINAL scan of `price_ohlcv_1h` bounded to 7 days and
      //     scoped to one quote leg. ⚠️ Measure it by what it READS, not what it
      //     returns — prod 2026-08-19 returns 423 rows but reads ~1.37M rows /
      //     ~70 MiB in 41-50 ms, because the cost is the FINAL merge and the
      //     `assets` lookup rather than the result size. (`_1d` was 984,706 rows
      //     / 50.5 MiB / 44-62 ms — only 1.4x cheaper, which is why the tier
      //     choice was decided on grace arithmetic instead. See SANITY_TABLE.)
      //   - MV drift: ~20 SEQUENTIAL round trips (a declared-side format, a
      //     live DDL fetch and a fingerprint per MV, plus the undeclared-writer
      //     sweep). This is the only one whose cost scales with round-trip
      //     latency rather than data volume, and it is the one to watch.
      //
      // ⚠️ A hard Lambda timeout is NOT a Rust Err, so it kills the invocation
      // with nothing published — and every alarm the probe feeds except the MV
      // drift ones is treatMissingData: NOT_BREACHING, i.e. scores missing data
      // as healthy. Confirm headroom by reading the function's Duration metric
      // in CloudWatch after the observability stack deploys; that needs NO
      // change to this stack, which matters because deploying this stack is the
      // CleanupRule hazard (task 0200).
      timeout: cdk.Duration.minutes(1),
      secretsExtensionLayer,
      chDomain,
      rule: this.rollupFreshnessProbeRule,
      alarmDescription:
        'Rollup freshness probe invocation errors — the per-tier rollup lag metric may be stale, blinding every rollup freshness alarm (the task 0136 blind spot).',
      alarmPeriod: cdk.Duration.minutes(15),
      errorAlarmActions: [opsAlarmAction],
    });
    this.rollupFreshnessProbeFunction = rollupFreshness.function;

    rollupFreshness.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'PublishRollupMetrics',
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: { 'cloudwatch:namespace': 'Prices/Rollup' },
        },
      }),
    );

    // -----------------------------------------------------------------
    // mTLS NotAfter probe (task 0056) + its rate(1d) target. Reads BOTH the
    // ingestion and api cert bundles from Secrets Manager (via the extension)
    // and publishes days-to-NotAfter → Prices/Mtls. It builds no CH client, so
    // CH_DOMAIN / MTLS_SECRET_NAME (set by the factory) are unused; the probe
    // reads MTLS_PROBE_SECRETS instead.
    // -----------------------------------------------------------------
    const apiMtlsSecretName = mtlsSecretName(env, 'api');
    const notafter = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'MtlsNotafterProbe',
      name: 'mtls-notafter-probe',
      assetDir: MTLS_NOTAFTER_PROBE_ASSET_DIR,
      memorySize: 256,
      timeout: cdk.Duration.minutes(1),
      secretsExtensionLayer,
      chDomain,
      rule: this.mtlsNotafterProbeRule,
      environment: {
        MTLS_PROBE_SECRETS: `ingestion=${discoveryMtlsSecretName},api=${apiMtlsSecretName}`,
      },
      alarmDescription:
        'mTLS NotAfter probe invocation errors — cert days-to-expiry metric may be stale, blinding the expiry alarm.',
      alarmPeriod: cdk.Duration.days(1),
      errorAlarmActions: [opsAlarmAction],
    });
    this.mtlsNotafterProbeFunction = notafter.function;

    // The factory's baseline grants read on the ingestion secret only; the
    // probe also reads the api role's bundle, so grant that second secret.
    notafter.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'ReadApiMtlsMaterial',
        actions: ['secretsmanager:GetSecretValue'],
        resources: [
          mtlsSecretArnFromParts(region, accountId, apiMtlsSecretName),
        ],
      }),
    );
    notafter.role.addToPolicy(
      new iam.PolicyStatement({
        sid: 'PublishMtlsMetrics',
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: { 'cloudwatch:namespace': 'Prices/Mtls' },
        },
      }),
    );

    new cdk.CfnOutput(this, 'BackfillFreshnessProbeFunctionName', {
      value: this.backfillFreshnessProbeFunction.functionName,
    });
    new cdk.CfnOutput(this, 'RollupFreshnessProbeFunctionName', {
      value: this.rollupFreshnessProbeFunction.functionName,
    });
    new cdk.CfnOutput(this, 'MtlsNotafterProbeFunctionName', {
      value: this.mtlsNotafterProbeFunction.functionName,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', env);

    assertCleanupRuleStaysDisabled(this.cleanupRule);
  }
}

/**
 * Refuse to synthesize a template that would re-enable the cleanup worker.
 *
 * 🔴 **This is a data-loss guard, not a style rule.** `prices-production-cleanup`
 * drops old partitions from `prices.*`. It has been DISABLED by operator action
 * since task 0200, and for most of that time this template asserted
 * `State: ENABLED` — so every deploy of this stack silently re-enabled it and
 * the only thing standing in the way was somebody remembering to run
 * `describe-rule` afterwards. Task 0204 (2026-08-20) corrected the template;
 * this guard is what stops the correction being undone by accident.
 *
 * ⚠️ The blast radius is far larger than "some old data". `price_ohlcv_1m`
 * carries a 7-day retention in the cleanup worker, and with cleanup stopped the
 * table now holds **years** of history — including the 1.56M peg-valued rows
 * task 0212 must repair. A single sweep drops all of it, irreversibly. Enabling
 * cleanup during the 0088 backfill already cost five days once.
 *
 * It reads the **resolved CloudFormation property**, not the constructor
 * argument, because those are not the same thing — the alarm-description limit
 * in `observability-stack.ts` taught that lesson by failing a production deploy
 * that synth had waved through.
 *
 * ⛔ **To re-enable cleanup you must delete this function**, not flip a boolean.
 * That is deliberate: whether cleanup should ever run again is task 0200's
 * decision, and it deserves a reviewed diff that says so out loud rather than a
 * one-character change nobody notices.
 */
function assertCleanupRuleStaysDisabled(rule: events.Rule): void {
  const cfn = rule.node.defaultChild as events.CfnRule;
  const state = cdk.Stack.of(rule).resolve(cfn.state) as unknown;

  if (state !== 'DISABLED') {
    throw new Error(
      `CleanupRule must synthesize as DISABLED, got ${JSON.stringify(state)}. ` +
        'This rule drops old partitions from prices.* and is deliberately off ' +
        '(task 0200); price_ohlcv_1m now holds years of history against a 7-day ' +
        'retention, including the rows task 0212 must repair. If you genuinely ' +
        'intend to re-enable cleanup, delete assertCleanupRuleStaysDisabled() ' +
        'in the same commit and say why in the message.',
    );
  }
}
