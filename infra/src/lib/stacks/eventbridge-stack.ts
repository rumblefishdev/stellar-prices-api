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
 * The price-updater + oracle-watcher + cleanup rules are still shells
 * (task 0039 attaches their targets). The asset-discovery rule now has
 * its target: the `asset-discovery` Lambda created here, which seeds the
 * major-asset baseline and scans recent ledgers for new assets
 * (`prices.assets`), advancing `prices.discovery_state`.
 *
 * The Rollup worker that appeared in the original task 0039 spec is
 * intentionally absent — ADR 0007 §3.4 replaces it with a ClickHouse
 * materialised-view chain.
 */
export class EventBridgeStack extends cdk.Stack {
  public readonly priceUpdaterRule: events.Rule;
  public readonly oracleWatcherRule: events.Rule;
  public readonly assetDiscoveryRule: events.Rule;
  public readonly cleanupRule: events.Rule;
  public readonly assetDiscoveryFunction: lambda.Function;

  constructor(scope: Construct, id: string, props: EventBridgeStackProps) {
    super(scope, id, props);

    const { config } = props;
    const env = config.envName;
    const region = config.awsRegion;
    const accountId = this.account;
    const schedules = config.scheduleExpressions;

    this.priceUpdaterRule = new events.Rule(this, 'PriceUpdaterRule', {
      ruleName: `prices-${env}-price-updater`,
      description: `Refreshes current_prices aggregations (${env})`,
      schedule: events.Schedule.expression(schedules.priceUpdater),
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

    new cdk.CfnOutput(this, 'PriceUpdaterRuleArn', {
      value: this.priceUpdaterRule.ruleArn,
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

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', env);
  }
}
