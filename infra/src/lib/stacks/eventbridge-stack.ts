import * as cdk from 'aws-cdk-lib';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
import * as events from 'aws-cdk-lib/aws-events';
import * as targets from 'aws-cdk-lib/aws-events-targets';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';
import {
  PRICES_LAMBDA_LOG_REMOVAL_POLICY,
  PRICES_LAMBDA_LOG_RETENTION,
  createPricesLambdaRole,
  lambdaLogGroupName,
  pricesLambdaDefaults,
} from '../lambda-baseline.js';
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
  public readonly assetDiscoveryFunction: lambda.Function;
  public readonly cleanupFunction: lambda.Function;
  public readonly supplyFunction: lambda.Function;
  public readonly oracleFunction: lambda.Function;

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

    const discoveryRole = createPricesLambdaRole(this, 'AssetDiscoveryRole', {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
    });

    const discoveryLogGroup = new logs.LogGroup(
      this,
      'AssetDiscoveryLogGroup',
      {
        logGroupName: lambdaLogGroupName(env, 'asset-discovery'),
        retention: PRICES_LAMBDA_LOG_RETENTION,
        removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
      },
    );

    const secretsExtensionLayer = lambda.LayerVersion.fromLayerVersionArn(
      this,
      'SecretsExtensionLayer',
      secretsManagerLayerArn(region),
    );

    this.assetDiscoveryFunction = new lambda.Function(
      this,
      'AssetDiscoveryFunction',
      {
        ...pricesLambdaDefaults, // ARM64 + PROVIDED_AL2023 (ADR 0006/0007)
        functionName: `prices-${env}-asset-discovery`,
        handler: 'bootstrap',
        code: lambda.Code.fromAsset(ASSET_DISCOVERY_ASSET_DIR),
        role: discoveryRole,
        logGroup: discoveryLogGroup,
        memorySize: 512,
        // Bounded by MAX_LEDGERS in the binary; a catch-up run fetches+decodes
        // many S3 objects, so allow generous headroom under the 1h cadence.
        timeout: cdk.Duration.minutes(5),
        tracing: lambda.Tracing.ACTIVE,
        layers: [secretsExtensionLayer],
        environment: {
          ENV_NAME: env,
          RUST_LOG: 'info',
          // mTLS endpoint (Caddy host on the Hetzner box).
          CH_DOMAIN: chDomain,
          // Single {cert,key,ca} bundle secret (task 0052/0063).
          MTLS_SECRET_NAME: discoveryMtlsSecretName,
          // Source bucket for ledger XDR objects (Galexie key scheme).
          BUCKET_NAME: ledgerBucketName,
          STELLAR_NETWORK_PASSPHRASE: networkPassphrase,
          // In-memory caching in the secrets extension.
          PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED: 'true',
          // NB: INITIAL_DISCOVERY_LEDGER is intentionally NOT set here — the
          // binary seeds gracefully without it and only scans once a
          // `prices.discovery_state` cursor exists. Operator activates the
          // ledger scan as a deploy-prep step (seed the cursor or set the
          // env), so synth is not gated on an operator value.
        },
      },
    );

    // S3 read on BE's ledger bucket (same-account → plain IAM grant, no
    // bucket policy from BE). Imported by attributes; the bucket is SSE-S3
    // (BE task 0306/0278), so no kms:Decrypt is needed.
    const ledgerBucket = s3.Bucket.fromBucketAttributes(this, 'LedgerBucket', {
      bucketArn: ledgerBucketArn,
      bucketName: ledgerBucketName,
    });
    ledgerBucket.grantRead(discoveryRole);

    // Wire the existing rate(1h) rule to the worker.
    this.assetDiscoveryRule.addTarget(
      new targets.LambdaFunction(this.assetDiscoveryFunction),
    );

    // Informational error alarm — registry maintenance is non-critical
    // (a failed run just defers new-asset pickup to the next hour).
    new cloudwatch.Alarm(this, 'AssetDiscoveryErrorAlarm', {
      alarmName: `prices-${env}-asset-discovery-errors`,
      alarmDescription:
        'Asset Discovery Lambda invocation errors (informational; registry maintenance is non-critical).',
      metric: this.assetDiscoveryFunction.metricErrors({
        period: cdk.Duration.hours(1),
        statistic: 'Sum',
      }),
      threshold: 1,
      evaluationPeriods: 1,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    });

    // -----------------------------------------------------------------
    // Cleanup worker Lambda (task 0039) + its cron target. CH-only (no S3,
    // no VPC): issues ALTER TABLE … DROP PARTITION over mTLS per §3.6.
    // Reuses the same `ingestion` mTLS identity + secrets extension layer
    // as the discovery worker.
    // -----------------------------------------------------------------
    const cleanupRole = createPricesLambdaRole(this, 'CleanupRole', {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
    });

    const cleanupLogGroup = new logs.LogGroup(this, 'CleanupLogGroup', {
      logGroupName: lambdaLogGroupName(env, 'cleanup'),
      retention: PRICES_LAMBDA_LOG_RETENTION,
      removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
    });

    this.cleanupFunction = new lambda.Function(this, 'CleanupFunction', {
      ...pricesLambdaDefaults, // ARM64 + PROVIDED_AL2023 (ADR 0006/0007)
      functionName: `prices-${env}-cleanup`,
      handler: 'bootstrap',
      code: lambda.Code.fromAsset(CLEANUP_WORKER_ASSET_DIR),
      role: cleanupRole,
      logGroup: cleanupLogGroup,
      memorySize: 256,
      // DROP PARTITION is metadata-only; the run is a handful of queries.
      timeout: cdk.Duration.minutes(2),
      tracing: lambda.Tracing.ACTIVE,
      layers: [secretsExtensionLayer],
      environment: {
        ENV_NAME: env,
        RUST_LOG: 'info',
        CH_DOMAIN: chDomain,
        MTLS_SECRET_NAME: discoveryMtlsSecretName,
        PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED: 'true',
      },
    });

    this.cleanupRule.addTarget(
      new targets.LambdaFunction(this.cleanupFunction),
    );

    new cloudwatch.Alarm(this, 'CleanupErrorAlarm', {
      alarmName: `prices-${env}-cleanup-errors`,
      alarmDescription:
        'Cleanup Lambda invocation errors (retention partition-drop failed).',
      metric: this.cleanupFunction.metricErrors({
        period: cdk.Duration.days(1),
        statistic: 'Sum',
      }),
      threshold: 1,
      evaluationPeriods: 1,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    });

    // -----------------------------------------------------------------
    // Supply worker Lambda (task 0039) + its rate(1h) target. CH mTLS +
    // public Horizon egress (no S3, no VPC). Fills prices.asset_supply that
    // the current_prices MV multiplies by the live price for market_cap.
    // Reuses the ingestion mTLS identity + secrets extension layer.
    // -----------------------------------------------------------------
    const supplyRole = createPricesLambdaRole(this, 'SupplyRole', {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
    });

    const supplyLogGroup = new logs.LogGroup(this, 'SupplyLogGroup', {
      logGroupName: lambdaLogGroupName(env, 'supply'),
      retention: PRICES_LAMBDA_LOG_RETENTION,
      removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
    });

    this.supplyFunction = new lambda.Function(this, 'SupplyFunction', {
      ...pricesLambdaDefaults, // ARM64 + PROVIDED_AL2023 (ADR 0006/0007)
      functionName: `prices-${env}-supply`,
      handler: 'bootstrap',
      code: lambda.Code.fromAsset(SUPPLY_WORKER_ASSET_DIR),
      role: supplyRole,
      logGroup: supplyLogGroup,
      memorySize: 512,
      // Sequential Horizon GETs across the asset registry; generous headroom
      // under the 1h cadence (best-effort, so a timeout just defers).
      timeout: cdk.Duration.minutes(5),
      tracing: lambda.Tracing.ACTIVE,
      layers: [secretsExtensionLayer],
      environment: {
        ENV_NAME: env,
        RUST_LOG: 'info',
        CH_DOMAIN: chDomain,
        MTLS_SECRET_NAME: discoveryMtlsSecretName,
        PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED: 'true',
        // HORIZON_URL unset → the binary's public-Horizon default.
      },
    });

    this.assetSupplyRule.addTarget(
      new targets.LambdaFunction(this.supplyFunction),
    );

    new cloudwatch.Alarm(this, 'SupplyErrorAlarm', {
      alarmName: `prices-${env}-supply-errors`,
      alarmDescription:
        'Supply Lambda invocation errors (informational; supply is best-effort, market_cap degrades to 0).',
      metric: this.supplyFunction.metricErrors({
        period: cdk.Duration.hours(1),
        statistic: 'Sum',
      }),
      threshold: 1,
      evaluationPeriods: 1,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    });

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
    const oracleRole = createPricesLambdaRole(this, 'OracleRole', {
      config,
      accountId,
      mtlsSecretName: discoveryMtlsSecretName,
    });

    const oracleLogGroup = new logs.LogGroup(this, 'OracleLogGroup', {
      logGroupName: lambdaLogGroupName(env, 'oracle'),
      retention: PRICES_LAMBDA_LOG_RETENTION,
      removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
    });

    this.oracleFunction = new lambda.Function(this, 'OracleFunction', {
      ...pricesLambdaDefaults, // ARM64 + PROVIDED_AL2023 (ADR 0006/0007)
      functionName: `prices-${env}-oracle`,
      handler: 'bootstrap',
      code: lambda.Code.fromAsset(ORACLE_WORKER_ASSET_DIR),
      role: oracleRole,
      logGroup: oracleLogGroup,
      memorySize: 256,
      timeout: cdk.Duration.minutes(2),
      tracing: lambda.Tracing.ACTIVE,
      layers: [secretsExtensionLayer],
      environment: {
        ENV_NAME: env,
        RUST_LOG: 'info',
        CH_DOMAIN: chDomain,
        MTLS_SECRET_NAME: discoveryMtlsSecretName,
        PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED: 'true',
        // SOROBAN_RPC_URL / REFLECTOR_CONTRACT unset → the binary's mainnet
        // defaults (public Soroban RPC + the Reflector CEX/DEX oracle).
      },
    });

    this.oracleWatcherRule.addTarget(
      new targets.LambdaFunction(this.oracleFunction),
    );

    new cloudwatch.Alarm(this, 'OracleErrorAlarm', {
      alarmName: `prices-${env}-oracle-errors`,
      alarmDescription:
        'Oracle Lambda invocation errors (informational; oracle is non-critical — §2.2 — and degrades to last-known value).',
      metric: this.oracleFunction.metricErrors({
        period: cdk.Duration.minutes(5),
        statistic: 'Sum',
      }),
      threshold: 1,
      evaluationPeriods: 1,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    });

    new cdk.CfnOutput(this, 'OracleFunctionName', {
      value: this.oracleFunction.functionName,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', env);
  }
}
