import * as cdk from 'aws-cdk-lib';
import * as events from 'aws-cdk-lib/aws-events';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';
import { createWorkerLambda } from '../lambda-baseline.js';
import { mtlsSecretName, secretsManagerLayerArn } from '../mtls.js';

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

/** Cargo-lambda build output for the `enrichment-worker` binary (task 0026). */
const ENRICHMENT_WORKER_ASSET_DIR =
  process.env['ENRICHMENT_WORKER_ASSET_DIR'] ??
  '../target/lambda/enrichment-worker';

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
  public readonly assetDiscoveryFunction: lambda.Function;
  public readonly cleanupFunction: lambda.Function;
  public readonly supplyFunction: lambda.Function;
  public readonly oracleFunction: lambda.Function;
  public readonly enrichmentFunction: lambda.Function;

  constructor(scope: Construct, id: string, props: EventBridgeStackProps) {
    super(scope, id, props);

    const { config } = props;
    const env = config.envName;
    const region = config.awsRegion;
    const accountId = this.account;
    const schedules = config.scheduleExpressions;

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

    this.cleanupRule = new events.Rule(this, 'CleanupRule', {
      ruleName: `prices-${env}-cleanup`,
      description: `Old-data partition drop on prices.* tables (${env})`,
      schedule: events.Schedule.expression(schedules.cleanup),
    });

    this.enrichmentRule = new events.Rule(this, 'EnrichmentRule', {
      ruleName: `prices-${env}-enrichment`,
      description: `close_usd / volume_quote_usd enrichment of price_ohlcv_1m (${env})`,
      schedule: events.Schedule.expression(schedules.enrichment),
    });

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
      assetDir: SUPPLY_WORKER_ASSET_DIR,
      memorySize: 512,
      // Sequential Horizon GETs across the asset registry; generous headroom
      // under the 1h cadence (best-effort, so a timeout just defers).
      timeout: cdk.Duration.minutes(5),
      secretsExtensionLayer,
      chDomain,
      rule: this.assetSupplyRule,
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
    this.oracleFunction = createWorkerLambda(this, {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
      idPrefix: 'Oracle',
      name: 'oracle',
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
    }).function;

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
      },
      alarmDescription:
        'Enrichment Lambda invocation errors (close_usd / volume_quote_usd enrichment pass failed).',
      alarmPeriod: cdk.Duration.hours(1),
    });
    this.enrichmentFunction = enrichment.function;

    // The worker publishes the spec §5 metrics (EnrichmentRowsEnriched,
    // EnrichmentOracleMiss, EnrichmentRowsRemainingAtVolumeZero,
    // EnrichmentBatchDurationMs) under the `Prices/Enrichment` namespace.
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

    new cdk.CfnOutput(this, 'EnrichmentRuleArn', {
      value: this.enrichmentRule.ruleArn,
    });
    new cdk.CfnOutput(this, 'EnrichmentFunctionName', {
      value: this.enrichmentFunction.functionName,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', env);
  }
}
