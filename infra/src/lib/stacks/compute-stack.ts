import * as cdk from 'aws-cdk-lib';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as lambdaEventSources from 'aws-cdk-lib/aws-lambda-event-sources';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as snsSubscriptions from 'aws-cdk-lib/aws-sns-subscriptions';
import * as sqs from 'aws-cdk-lib/aws-sqs';
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

const DLQ_RETENTION_DAYS = 14;

/**
 * Cargo-lambda build output for the `prices-ledger-processor` binary.
 *
 * BE defines the equivalent Lambda with `cargo-lambda-cdk`'s
 * `RustFunction`, which shells out to `cargo lambda build` at synth.
 * The prices-api infra does not (yet) carry that dependency, so this
 * stack consumes the pre-built `provided.al2023` bootstrap via
 * `Code.fromAsset`. Build it first:
 *
 *     cargo lambda build -p prices-ledger-processor --release --arm64
 *
 * which writes `target/lambda/prices-ledger-processor/bootstrap`.
 * Production follow-up: add `cargo-lambda-cdk` and swap this for
 * `RustFunction` so synth builds the binary (mirrors BE exactly).
 * Override with `LEDGER_PROCESSOR_ASSET_DIR` if needed.
 */
const LEDGER_PROCESSOR_ASSET_DIR =
  process.env['LEDGER_PROCESSOR_ASSET_DIR'] ??
  '../target/lambda/prices-ledger-processor';

/**
 * SSM keys the BE team publishes under the platform namespace and the
 * prices-api CDK reads **at deploy time** (NOT at Lambda runtime — the
 * Lambda only ever sees env vars; SSM is the deploy handshake). See
 * task 0038 spec §C.2.
 *
 * `ledgerEventsTopicArn` is the artefact of the 2026-06-10 SNS
 * decision (§C.1): BE moves the bucket notification to an SNS topic
 * and publishes its ARN here for prices-api to subscribe to. Topic
 * ownership + the cross-team handshake is tracked by task 0050.
 */
function platformSsmKeys(envName: string) {
  const base = `/platform/${envName}`;
  return {
    ledgerBucketName: `${base}/stellar-ledger-data-bucket-name`,
    ledgerBucketArn: `${base}/stellar-ledger-data-bucket-arn`,
    chDomain: `${base}/ch-domain`,
    networkPassphrase: `${base}/stellar-network-passphrase`,
    ledgerEventsTopicArn: `${base}/ledger-events-topic-arn`,
  };
}

export interface ComputeStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * Compute layer for prices-api.
 *
 * Owns the per-Lambda IAM roles + LogGroups for the two anchor
 * Lambdas, plus the full wiring of the live **Prices Ledger Processor**
 * Lambda (task 0038): its SQS doorbell queue + DLQ, the SNS fan-out
 * subscription, the Function itself, and the event-source-mapping.
 *
 * - `ledgerProcessorRole` / `ledgerProcessorLogGroup` +
 *   `ledgerProcessorFunction` — task 0038 (live S3-event-driven
 *   ingest). Role + queue + function are deliberately co-located in
 *   one stack: the event-source-mapping and the queue/bucket grants
 *   mutate the role's policy, so splitting them across stacks creates
 *   a CloudFormation dependency cycle. (BE keeps the same shape in a
 *   single `compute-stack.ts`.)
 * - `apiHandlerRole` / `apiHandlerLogGroup` — consumed by task 0040
 *   (axum REST handlers behind API Gateway); no Function yet.
 *
 * The four periodic-worker roles (task 0039) and the backfill-status
 * role (task 0055) are NOT pre-created here — those Lambdas are
 * coupled to the EventBridge Scheduler rules (0039) and API Gateway
 * routes (0055) defined alongside them; each calls
 * `createPricesLambdaRole` from `lib/lambda-baseline.ts`.
 *
 * Ingest topology (2026-06-10 cross-team decision, spec §C.1 — SNS
 * fan-out):
 *
 *     ledger PutObject → S3 ObjectCreated (BE's stellar-ledger-data)
 *                      → SNS topic (BE-owned)
 *                      ├─ SQS  ledger-ingest-{env}   (BE indexer)
 *                      └─ SQS  prices-ingest-{env}   (this stack) + DLQ
 *                                                    → ledger-processor Lambda
 *
 * prices-api owns its **own** queue + DLQ subscribed to BE's topic, so
 * a prices-side backlog never pressures BE's indexer queue (failure
 * isolation). BE and prices share one AWS account, so the SNS→SQS
 * subscription is same-account — no cross-account topic policy is
 * required, only the queue resource policy `SqsSubscription` adds.
 *
 * Mirrors BE's `compute-stack.ts`: `reservedConcurrentExecutions = 1`
 * and `batchSize = 1` are load-bearing for ordering (the doorbell-
 * cursor reconcile loop races the cursor under concurrency), not perf
 * knobs; `maxReceiveCount = 10` absorbs the ESM over-poll/throttle
 * churn that concurrency=1 induces; `visibilityTimeout = timeout + 60s`
 * so SQS never redelivers a doorbell still being processed.
 *
 * No VPC (ADR 0007 §3.6); identity to the Hetzner Caddy/CH endpoint is
 * mTLS, the bundle sourced via the Parameters and Secrets extension
 * layer. Deploy is gated on BE 0227 + task 0047 + BE publishing the
 * platform SSM keys; this ingest wiring is authored prepare-only.
 */
export class ComputeStack extends cdk.Stack {
  public readonly ledgerProcessorRole: iam.Role;
  public readonly ledgerProcessorLogGroup: logs.LogGroup;
  public readonly ledgerProcessorFunction: lambda.Function;
  public readonly ingestQueue: sqs.Queue;
  public readonly ingestDlq: sqs.Queue;
  public readonly apiHandlerRole: iam.Role;
  public readonly apiHandlerLogGroup: logs.LogGroup;
  /**
   * `MTLS_SECRET_NAME` the ledger-processor (writer) Lambda reads — the
   * single `{cert,key,ca}` bundle for CH user `prices_writer`. Set on the
   * Function env below; the role is granted read on exactly this secret.
   */
  public readonly ledgerProcessorMtlsSecretName: string;
  /**
   * `MTLS_SECRET_NAME` the api-handler (reader) Lambda must read — the
   * single bundle for CH user `prices_reader`. Set on the Function env in
   * task 0040; the role is already granted read on exactly this secret.
   */
  public readonly apiHandlerMtlsSecretName: string;

  constructor(scope: Construct, id: string, props: ComputeStackProps) {
    super(scope, id, props);

    const { config } = props;
    const { envName, awsRegion } = config;
    const lp = config.ledgerProcessor;
    const accountId = cdk.Stack.of(this).account;
    const keys = platformSsmKeys(envName);

    // Two mTLS identities, mirroring BE's per-service split: the ledger
    // processor writes as `prices_writer` (ingestion bundle); the api handler
    // reads as `prices_reader` (api bundle). Each role is granted read on ONLY
    // its own secret (least privilege). The secrets are created out-of-band by
    // the operator (see SecretsStack); CDK only names + grants + sets the env.
    this.ledgerProcessorMtlsSecretName = mtlsSecretName(envName, 'ingestion');
    this.apiHandlerMtlsSecretName = mtlsSecretName(envName, 'api');

    // ---------------------------------------------------------------
    // Ledger Processor: baseline role + log group
    // ---------------------------------------------------------------
    this.ledgerProcessorRole = createPricesLambdaRole(
      this,
      'LedgerProcessorRole',
      {
        config,
        accountId,
        mtlsSecretName: this.ledgerProcessorMtlsSecretName,
      },
    );
    this.ledgerProcessorLogGroup = new logs.LogGroup(
      this,
      'LedgerProcessorLogGroup',
      {
        logGroupName: lambdaLogGroupName(envName, 'ledger-processor'),
        retention: PRICES_LAMBDA_LOG_RETENTION,
        removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
      },
    );

    // ---------------------------------------------------------------
    // Deploy-time SSM reads (BE-published platform identifiers).
    // valueForStringParameter resolves via a CFN parameter at deploy;
    // the Lambda only ever sees the resulting env-var values.
    // ---------------------------------------------------------------
    const ledgerBucketName = ssm.StringParameter.valueForStringParameter(
      this,
      keys.ledgerBucketName,
    );
    const ledgerBucketArn = ssm.StringParameter.valueForStringParameter(
      this,
      keys.ledgerBucketArn,
    );
    const chDomain = ssm.StringParameter.valueForStringParameter(
      this,
      keys.chDomain,
    );
    const networkPassphrase = ssm.StringParameter.valueForStringParameter(
      this,
      keys.networkPassphrase,
    );
    const ledgerEventsTopicArn = ssm.StringParameter.valueForStringParameter(
      this,
      keys.ledgerEventsTopicArn,
    );
    // Bootstrap ledger for the doorbell cursor (`INITIAL_CURSOR`). The
    // reconcile loop's `/tmp` cursor file is empty on a fresh container, so
    // without a seed `cursor.read()` errors and every doorbell DLQs — the
    // Lambda can never start. Sourced from the prices-owned SSM namespace
    // (the operator seeds "live ingestion starts here" at deploy prep, like
    // the mTLS secrets) rather than committed config, so it is never a
    // stale magic number. One-time bootstrap; superseded by the durable
    // CH-backed cursor (task 0064).
    const initialCursor = ssm.StringParameter.valueForStringParameter(
      this,
      `/prices/${envName}/ledger-processor/initial-cursor`,
    );

    // ---------------------------------------------------------------
    // SQS DLQ + prices ingest queue (prices-owned doorbell source)
    // ---------------------------------------------------------------
    this.ingestDlq = new sqs.Queue(this, 'PricesIngestDlq', {
      queueName: `prices-ingest-dlq-${envName}`,
      retentionPeriod: cdk.Duration.days(DLQ_RETENTION_DAYS),
    });

    this.ingestQueue = new sqs.Queue(this, 'PricesIngestQueue', {
      queueName: `prices-ingest-${envName}`,
      // MUST be >= the Lambda timeout, else SQS redelivers a doorbell
      // the Lambda is still legitimately draining. timeout + 60s margin.
      visibilityTimeout: cdk.Duration.seconds(lp.timeoutSeconds + 60),
      retentionPeriod: cdk.Duration.days(DLQ_RETENTION_DAYS),
      deadLetterQueue: {
        queue: this.ingestDlq,
        maxReceiveCount: lp.maxReceiveCount,
      },
    });

    // ---------------------------------------------------------------
    // SNS fan-out subscription — BE-owned topic → our queue
    // ---------------------------------------------------------------
    // Import the BE topic by ARN (published under the platform SSM key
    // per the SNS decision). Adding the SqsSubscription creates the
    // AWS::SNS::Subscription in THIS stack and attaches the queue
    // resource policy that lets the topic deliver — prices owns the
    // subscription side, BE owns the topic. rawMessageDelivery keeps
    // the body the bare S3 event (our Lambda ignores it regardless).
    const ledgerEventsTopic = sns.Topic.fromTopicArn(
      this,
      'BeLedgerEventsTopic',
      ledgerEventsTopicArn,
    );
    ledgerEventsTopic.addSubscription(
      new snsSubscriptions.SqsSubscription(this.ingestQueue, {
        rawMessageDelivery: true,
      }),
    );

    // ---------------------------------------------------------------
    // Secrets Manager extension layer (mTLS bundle fetch at cold start)
    // ---------------------------------------------------------------
    const secretsExtensionLayer = lambda.LayerVersion.fromLayerVersionArn(
      this,
      'SecretsExtensionLayer',
      secretsManagerLayerArn(awsRegion),
    );

    // ---------------------------------------------------------------
    // Ledger Processor Lambda + SQS event-source-mapping
    // ---------------------------------------------------------------
    this.ledgerProcessorFunction = new lambda.Function(
      this,
      'LedgerProcessorFunction',
      {
        ...pricesLambdaDefaults, // ARM64 + PROVIDED_AL2023 (ADR 0006/0007)
        functionName: `prices-${envName}-ledger-processor`,
        // cargo-lambda emits a single self-contained `bootstrap` binary;
        // PROVIDED_AL2023 custom runtimes always use the `bootstrap`
        // handler name.
        handler: 'bootstrap',
        code: lambda.Code.fromAsset(LEDGER_PROCESSOR_ASSET_DIR),
        role: this.ledgerProcessorRole,
        logGroup: this.ledgerProcessorLogGroup,
        memorySize: lp.memoryMb,
        timeout: cdk.Duration.seconds(lp.timeoutSeconds),
        // Load-bearing: serial execution is the ordering guarantee. The
        // reconcile loop reads a cursor and advances it; two concurrent
        // invocations would race it. validateConfig pins this to 1.
        reservedConcurrentExecutions: lp.reservedConcurrency,
        tracing: lambda.Tracing.ACTIVE,
        layers: [secretsExtensionLayer],
        environment: {
          ENV_NAME: envName,
          RUST_LOG: 'info',
          // Source bucket for ledger XDR objects. The Lambda derives S3
          // keys from ledger numbers (Galexie scheme) and HEAD/GETs this
          // bucket; it does NOT parse the SQS doorbell body.
          BUCKET_NAME: ledgerBucketName,
          // mTLS endpoint (Caddy host on the Hetzner box).
          CH_DOMAIN: chDomain,
          // Required by xdr-parser's network-id cache (SAC derivation).
          STELLAR_NETWORK_PASSPHRASE: networkPassphrase,
          // Single {cert,key,ca} bundle secret (task 0052/0063). Task 0052's
          // clickhouse client crate reads exactly this one env var
          // (`MTLS_SECRET_NAME`) and parses the JSON bundle via the extension
          // — see packages/prices-clickhouse/src/mtls.rs:233. Name is derived
          // from the shared mtlsSecretName helper, so it can't drift from the
          // SecretsStack publication or the operator's create-secret.
          MTLS_SECRET_NAME: this.ledgerProcessorMtlsSecretName,
          // Bootstrap cursor seed. main.rs writes the `/tmp` cursor file
          // from this on a fresh container; without it `cursor.read()` errors
          // and every doorbell DLQs (the Lambda never starts).
          INITIAL_CURSOR: initialCursor,
          // Explicit cursor checkpoint path. `/tmp` is the only writable
          // Lambda filesystem; matches the Rust default but pinned here so
          // the runtime contract is visible. (Per-container ephemeral —
          // durable cursor is task 0064.)
          CURSOR_FILE: '/tmp/prices-cursor.txt',
          // Max contiguous ledgers per reconcile run (bounds fetch+decode
          // against the Lambda timeout).
          MAX_ITERATIONS: String(lp.maxIterations),
          // In-memory caching in the secrets extension — repeat reads in
          // one execution environment hit RAM, not Secrets Manager.
          PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED: 'true',
        },
      },
    );

    // batchSize 1 mirrors BE (the doorbell body is ignored, so larger
    // batches buy nothing under concurrency=1). reportBatchItemFailures
    // lets the handler fail just the offending doorbell; SQS redelivers
    // it up to maxReceiveCount, then it lands in the DLQ.
    //
    // maxConcurrency caps the event-source's poller scaling. By default the
    // ESM scales to 5 concurrent batches; with reservedConcurrency=1 the
    // other 4 are throttle-rejected and their messages re-enqueue, each
    // incrementing receiveCount — under a burst a processable doorbell can
    // hit maxReceiveCount and false-DLQ before it is ever handled. 2 is the
    // ESM minimum (it cannot equal the reserved 1), so this shrinks the
    // over-poll window from 5→2 to complement maxReceiveCount.
    this.ledgerProcessorFunction.addEventSource(
      new lambdaEventSources.SqsEventSource(this.ingestQueue, {
        batchSize: lp.sqsBatchSize,
        reportBatchItemFailures: true,
        maxConcurrency: 2,
      }),
    );

    // ---------------------------------------------------------------
    // IAM — S3 read on BE's bucket (same-account → plain IAM grant, no
    // bucket policy from BE) + CloudWatch lag metric + X-Ray.
    // ---------------------------------------------------------------
    const ledgerBucket = s3.Bucket.fromBucketAttributes(this, 'LedgerBucket', {
      bucketArn: ledgerBucketArn,
      bucketName: ledgerBucketName,
    });
    ledgerBucket.grantRead(this.ledgerProcessorRole);

    // grantRead on a bucket imported by attributes (no `encryptionKey`)
    // cannot infer an SSE-KMS key, so it adds no kms:Decrypt. If BE's
    // bucket is KMS-encrypted, every GetObject would 403 (AccessDenied) —
    // which S3Fetcher maps to a hard error that DLQ's the doorbell, not a
    // gap. Grant decrypt explicitly when the key ARN is configured.
    if (lp.bucketKmsKeyArn) {
      this.ledgerProcessorRole.addToPrincipalPolicy(
        new iam.PolicyStatement({
          sid: 'DecryptLedgerObjects',
          actions: ['kms:Decrypt'],
          resources: [lp.bucketKmsKeyArn],
        }),
      );
    }

    this.ledgerProcessorRole.addToPrincipalPolicy(
      new iam.PolicyStatement({
        sid: 'PublishLagMetric',
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: { 'cloudwatch:namespace': 'PricesApi/LedgerProcessor' },
        },
      }),
    );
    this.ledgerProcessorRole.addToPrincipalPolicy(
      new iam.PolicyStatement({
        sid: 'XRayWrite',
        actions: ['xray:PutTraceSegments', 'xray:PutTelemetryRecords'],
        resources: ['*'],
      }),
    );

    // ---------------------------------------------------------------
    // API Handler: baseline role + log group (Function lands in 0040)
    // ---------------------------------------------------------------
    this.apiHandlerRole = createPricesLambdaRole(this, 'ApiHandlerRole', {
      config,
      accountId,
      mtlsSecretName: this.apiHandlerMtlsSecretName,
    });
    this.apiHandlerLogGroup = new logs.LogGroup(this, 'ApiHandlerLogGroup', {
      logGroupName: lambdaLogGroupName(envName, 'api-handler'),
      retention: PRICES_LAMBDA_LOG_RETENTION,
      removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
    });

    // ---------------------------------------------------------------
    // Outputs
    // ---------------------------------------------------------------
    new cdk.CfnOutput(this, 'LedgerProcessorRoleArn', {
      value: this.ledgerProcessorRole.roleArn,
      description: `Ledger Processor Lambda execution role ARN (${envName})`,
    });
    new cdk.CfnOutput(this, 'LedgerProcessorFunctionArn', {
      value: this.ledgerProcessorFunction.functionArn,
      description: `Prices Ledger Processor Lambda ARN (${envName})`,
    });
    new cdk.CfnOutput(this, 'PricesIngestQueueUrl', {
      value: this.ingestQueue.queueUrl,
      description: `Prices ingest queue URL (${envName})`,
    });
    new cdk.CfnOutput(this, 'PricesIngestDlqUrl', {
      value: this.ingestDlq.queueUrl,
      description: `Prices ingest DLQ URL (${envName})`,
    });
    new cdk.CfnOutput(this, 'ApiHandlerRoleArn', {
      value: this.apiHandlerRole.roleArn,
      description: `API Handler Lambda execution role ARN (${envName})`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', envName);
  }
}
