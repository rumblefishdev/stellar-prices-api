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
import {
  mtlsSecretArnFromParts,
  mtlsSecretName,
  portalOauthSecretName,
  secretsManagerLayerArn,
} from '../mtls.js';

const DLQ_RETENTION_DAYS = 14;

/**
 * Deterministic physical names for the live ledger-processor Lambda and its SQS
 * doorbell queue + DLQ. ComputeStack owns these resources; ObservabilityStack
 * imports these helpers to build CloudWatch alarm metric dimensions by name —
 * so the two stacks share a single source of truth with no cross-stack CFN
 * reference (mirrors `opsAlarmsTopicName`). A rename here now flows to the alarms
 * automatically instead of silently pointing them at a non-existent metric.
 */
export function ledgerProcessorFunctionName(envName: string): string {
  return `prices-${envName}-ledger-processor`;
}
export function ingestQueueName(envName: string): string {
  return `prices-ingest-${envName}`;
}
export function ingestDlqName(envName: string): string {
  return `prices-ingest-dlq-${envName}`;
}

/**
 * Cargo-lambda build output for the `prices-ledger-processor` binary.
 *
 * BE defines the equivalent Lambda with `cargo-lambda-cdk`'s
 * `RustFunction`, which shells out to `cargo lambda build` at synth.
 * The prices-api infra does not (yet) carry that dependency, so this
 * stack consumes the pre-built `provided.al2023` bootstrap via
 * `Code.fromAsset`. Build it first:
 *
 *     cargo lambda build -p prices-ledger-processor --release --arm64 --features lambda
 *
 * `--features lambda` is REQUIRED: the `prices-ledger-processor` bin is
 * `required-features = ["lambda"]`, so a build without it silently skips
 * the bin and produces no bootstrap — CDK would then package a stale
 * asset. Writes `target/lambda/prices-ledger-processor/bootstrap`.
 * Production follow-up: add `cargo-lambda-cdk` and swap this for
 * `RustFunction` so synth builds the binary (mirrors BE exactly).
 * Override with `LEDGER_PROCESSOR_ASSET_DIR` if needed.
 */
const LEDGER_PROCESSOR_ASSET_DIR =
  process.env['LEDGER_PROCESSOR_ASSET_DIR'] ??
  '../target/lambda/prices-ledger-processor';

/**
 * Cargo-lambda build output for the `prices-api` binary (the single axum
 * api-handler, ADR 0008). Build it first:
 *
 *     cargo lambda build -p prices-api --release --arm64 --features lambda
 *
 * which writes `target/lambda/prices-api/bootstrap`. Same pre-built-asset
 * convention as the ledger processor (RustFunction adoption is a shared
 * follow-up). Override with `API_HANDLER_ASSET_DIR`.
 */
const API_HANDLER_ASSET_DIR =
  process.env['API_HANDLER_ASSET_DIR'] ?? '../target/lambda/prices-api';

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
   * The single axum api-handler Lambda (ADR 0008). Consumed by
   * `ApiGatewayStack` as the proxy integration for all `/v1` routes.
   */
  public readonly apiHandlerFunction: lambda.Function;
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
  /**
   * `PORTAL_OAUTH_SECRET_NAME` the api-handler reads for portal sign-in
   * (task 0186) — the Discord client id/secret, the registered redirect URI and
   * the session signing key, as one JSON bundle. Set on the Function env below;
   * the role is granted read on exactly this secret and nothing else. The
   * secret's VALUE never appears in this template, in an env var, or in a log.
   */
  public readonly portalOauthSecretName: string;
  /**
   * `PORTAL_FREE_PLAN_PARAM` the api-handler reads for self-service key
   * issuance (task 0187) — the NAME of the SSM parameter holding the
   * `pricing-api-free` usage-plan id, not the id itself.
   *
   * It cannot be the id, and it cannot be a cross-stack reference: the plan is
   * created by `ApiGatewayStack`, which depends on this stack, so importing it
   * here would close a Compute -> Gateway -> Compute cycle — the same shape of
   * problem `apiBaseUrl` has. And it must not be hard-coded, because AWS
   * generates the id and it changes if the plan is ever replaced. So the
   * handler reads it at cold start through the Parameters and Secrets extension
   * already attached below, exactly as it reads secret VALUES by NAME.
   */
  public readonly portalFreePlanParameterName: string;

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
    // Same pattern, third secret: the name is computed by the shared helper, the
    // grant is on that name's wildcard ARN, and the operator owns the value
    // (task 0186). Only the api-handler reads it — no worker signs a cookie.
    this.portalOauthSecretName = portalOauthSecretName(envName);
    // Must match `ApiGatewayStack`'s `PricingApiFreePlanIdParam` exactly — the
    // two are the write and the read of one value, in two stacks that cannot
    // reference each other. Task 0194 audits the pair.
    this.portalFreePlanParameterName = `/prices/${envName}/pricing-api-free-plan-id`;

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
      queueName: ingestDlqName(envName),
      retentionPeriod: cdk.Duration.days(DLQ_RETENTION_DAYS),
    });

    this.ingestQueue = new sqs.Queue(this, 'PricesIngestQueue', {
      queueName: ingestQueueName(envName),
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
        functionName: ledgerProcessorFunctionName(envName),
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
          // First-run bootstrap seed for the durable cursor (task 0064). main.rs
          // seeds `prices.ingest_cursor` from this ONLY when the table is empty
          // (a genuine first run); thereafter the persisted ClickHouse value is
          // authoritative and survives execution-environment recycles.
          INITIAL_CURSOR: initialCursor,
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
    // We deliberately do NOT set the ESM `maxConcurrency`. AWS requires
    // 2 ≤ maxConcurrency ≤ the function's reserved concurrency; with the
    // load-bearing reservedConcurrency=1 (the serial-ordering guarantee,
    // enforced in types.ts) there is no legal value — the API rejects it with
    // "MaximumConcurrency: 2 is greater than Function Reserved Concurrency: 1".
    // reservedConcurrency=1 already caps execution to a single concurrent
    // invocation (the ordering guarantee); the ESM's default poller scaling is
    // throttled down to that 1, and any over-poll re-drives are bounded by the
    // queue's maxReceiveCount → DLQ. Mirrors BE's compute-stack (no maxConcurrency).
    this.ledgerProcessorFunction.addEventSource(
      new lambdaEventSources.SqsEventSource(this.ingestQueue, {
        batchSize: lp.sqsBatchSize,
        reportBatchItemFailures: true,
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

    // Read on the portal's Discord OAuth bundle (task 0186). Added here rather
    // than in `createPricesLambdaRole`'s baseline: every prices Lambda needs its
    // own mTLS material, but exactly one of them terminates an OAuth flow, and a
    // client secret readable by the cleanup worker is a client secret with a
    // wider blast radius than it needs.
    //
    // The grant is on the by-name wildcard ARN, so it does not require the
    // secret to exist at synth time — the operator creates it, and until they
    // do, the Lambda simply never asks (`PORTAL_ENABLED=false` short-circuits
    // the read; see `AppConfig::load_portal_oauth`).
    this.apiHandlerRole.addToPrincipalPolicy(
      new iam.PolicyStatement({
        sid: 'ReadPortalOauthSecret',
        actions: ['secretsmanager:GetSecretValue'],
        resources: [
          mtlsSecretArnFromParts(
            awsRegion,
            accountId,
            this.portalOauthSecretName,
          ),
        ],
      }),
    );

    // ---------------------------------------------------------------
    // Self-service API keys (task 0187) — API Gateway CONTROL plane.
    // ---------------------------------------------------------------
    //
    // Five of the seven calls the portal makes. The other two —
    // `POST /usageplans/{id}/keys` (task 0187's attach) and
    // `GET /usageplans/{id}/usage` (task 0188's `GetUsage`) — are granted in
    // `ApiGatewayStack` instead, because the plan id lives there and importing
    // it here would close the Compute -> Gateway -> Compute cycle described on
    // `portalFreePlanParameterName` above. Each grant is declared where its
    // resource is known; task 0194 audits the set as one policy.
    //
    // Control-plane ARNs carry no account id — `arn:aws:apigateway:<region>::`
    // with a doubled colon — and the resource is the API's own path.
    //
    // **Three limits, and only two of them are forced.** Recorded here rather
    // than in a commit message because they are the boundary of what this
    // policy can be — and because task 0194's IAM audit reads this comment as
    // its statement of intent. A sentence here that is wrong is an audit that
    // passes without asking the question.
    //
    // 1. **`POST /apikeys` cannot be narrowed.** There is no ARN for "keys this
    //    function created", so the grant to create a key is a grant to create
    //    any key. Mitigated by what the code does with it — one deterministic
    //    name per Discord id, attached only to the self-service plan — and by
    //    the `ManagedBy=prices-portal` tag every created key carries, which is
    //    what makes them answerable after the fact.
    // 2. **`GET /apikeys` cannot be narrowed either, and it is the biggest of
    //    the three.** The reconciler needs the collection (see the sid below),
    //    and a collection has one ARN: there is no per-key form of it, and tag
    //    conditions do not apply to a list. `GetApiKeys` accepts
    //    `includeValues=true`, so this grant permits reading the VALUE of every
    //    API key in the account — the partner keys on `/v1/` included, which are
    //    not this feature's to see. The handler asks for `includeValues=false`
    //    on every listing (`Gateway::list_named`, asserted by
    //    `the_listing_never_requests_key_values`), so the exposure is what an
    //    attacker with code execution in this Lambda would gain, not what the
    //    feature does. Worth stating precisely because it is easy to read the
    //    list of verbs as harmless next to `DELETE`.
    // 3. **`GET`/`PATCH`/`DELETE` on `/apikeys/*` ARE narrowed — by tag.** The
    //    path wildcard is forced — AWS generates the key id, so it is
    //    unknowable at synth time — but API Gateway supports
    //    `aws:ResourceTag/${TagKey}` conditions on per-key control-plane
    //    actions
    //    (docs.aws.amazon.com/apigateway/latest/developerguide/apigateway-tagging-iam-policy.html),
    //    and every key this feature creates carries `ManagedBy=prices-portal`
    //    from the create call. Task 0191 wrote the condition (it was
    //    "available and unwritten" since 0187, parked for 0194) once `PATCH`
    //    joined the statement: a per-key read is `GetApiKey includeValue=true`
    //    — the value of ANY key in the account by id, partner keys included,
    //    and the call the code actually makes with the value flag on — and a
    //    per-key patch can rename any key into a portal name the reconciler
    //    would then adopt and reveal, or re-enable a key its owner revoked.
    //    The accepted behaviour change: an exact-name key created BY HAND in
    //    the console is untagged, so it is no longer adopted — read, disable
    //    and delete all `AccessDenied`, the routes answer `502`, and the key
    //    stays exactly as the human left it. The code guard in
    //    `portal/keys/naming.rs` (never rank or delete a key whose name is not
    //    exactly the caller's) still stands underneath.
    //
    // What is deliberately NOT here: `apigateway:*`, `PUT /tags/*` (the portal
    // never re-tags a key), and any grant on `/usageplans` beyond the key
    // attachment and the usage read — both of those need the plan id, so both
    // live in `ApiGatewayStack`'s standalone policy (`POST …/keys` for 0187's
    // attach, `GET …/usage` for 0188's `GetUsage`).
    //
    // `DELETE` **is** here, and it is this slice's: the reconciler removes
    // duplicate keys after a double-submit ("keep the earliest createdDate,
    // DeleteApiKey the rest").
    //
    // `PATCH` on `/apikeys/*` is task 0191's: `UpdateApiKey(enabled=false)`,
    // the revocation behind "Replace my key". Disable rather than delete,
    // because the disabled key IS the record of the revocation — its
    // `lastUpdatedDate` is what refuses a re-issue inside the same quota
    // period, and there is no registry (task 0190) to hold that fact
    // otherwise. The handler sends exactly one patch operation,
    // `replace /enabled false`; it never re-enables, renames or re-tags a
    // key, and `PATCH` on `/apikeys/*` cannot reach a usage plan or a stage.
    // Same wildcard limit as `DELETE` (the key id is unknowable at synth
    // time) and the same tag-condition tightening available to task 0194.
    this.apiHandlerRole.addToPrincipalPolicy(
      new iam.PolicyStatement({
        sid: 'PortalCreateAndListApiKeys',
        actions: ['apigateway:POST', 'apigateway:GET'],
        resources: [`arn:aws:apigateway:${awsRegion}::/apikeys`],
      }),
    );
    this.apiHandlerRole.addToPrincipalPolicy(
      new iam.PolicyStatement({
        sid: 'PortalReadDisableAndDeleteOwnApiKeys',
        actions: ['apigateway:GET', 'apigateway:PATCH', 'apigateway:DELETE'],
        resources: [`arn:aws:apigateway:${awsRegion}::/apikeys/*`],
        // See limit 3 above: portal-made keys only.
        conditions: {
          StringEquals: { 'aws:ResourceTag/ManagedBy': 'prices-portal' },
        },
      }),
    );

    // The usage-plan id, read at cold start.
    //
    // **Currently redundant, and kept deliberately.** The baseline role already
    // carries `ReadSsmNamespaces`, which grants `ssm:GetParameter` across the
    // whole `/prices/${envName}/*` namespace — so this statement adds no access
    // today. It names the one parameter this feature depends on, so that
    // narrowing that baseline (which task 0194 may well want to) does not
    // silently break key issuance at the next cold start. Stated rather than
    // left implicit, because an IAM statement that looks like the reason
    // something works, while something broader is the actual reason, is worse
    // than no statement at all.
    this.apiHandlerRole.addToPrincipalPolicy(
      new iam.PolicyStatement({
        sid: 'PortalReadFreePlanIdParameter',
        actions: ['ssm:GetParameter'],
        resources: [
          `arn:aws:ssm:${awsRegion}:${accountId}:parameter${this.portalFreePlanParameterName}`,
        ],
      }),
    );

    // The eligibility gate's two knobs (task 0189), read at runtime — per
    // issuance, not at cold start alone. The same currently-redundant-and-kept
    // reasoning as `PortalReadFreePlanIdParameter` above: the baseline's
    // `ReadSsmNamespaces` already covers `/prices/${envName}/*`, and this
    // statement names the two parameters the gate depends on so a narrowed
    // baseline cannot silently break issuance.
    //
    // **CDK must never CREATE these parameters** — no `ssm.StringParameter`,
    // and no `valueForStringParameter` (which freezes the value into the
    // template at deploy time, defeating "tunable without a redeploy"). A
    // CloudFormation-managed parameter is CDK-owned, so the next `cdk deploy`
    // silently restores the committed value — which, after task 0179 points
    // production at the real Stellar guild, would un-flip it back to the test
    // guild. The operator seeds both values at deploy prep (runbook §2a), the
    // same ownership split as the OAuth secret. CI pins the rule:
    // `verify-openapi-routes.mjs` check 7 refuses any `AWS::SSM::Parameter`
    // with either name in any synthesized template.
    this.apiHandlerRole.addToPrincipalPolicy(
      new iam.PolicyStatement({
        sid: 'PortalReadEligibilityParameters',
        actions: ['ssm:GetParameter'],
        resources: [
          `arn:aws:ssm:${awsRegion}:${accountId}:parameter/prices/${envName}/discord-guild-id`,
          `arn:aws:ssm:${awsRegion}:${accountId}:parameter/prices/${envName}/min-account-age-minutes`,
        ],
      }),
    );

    // The single axum api-handler (ADR 0008). Reads as `prices_reader` over
    // mTLS; reuses the same `chDomain` SSM value + secrets extension layer as
    // the ledger processor. No `API_KEYS` env → the in-app key gate stays
    // DISARMED; the per-key rate and monthly quota are enforced at the API
    // Gateway usage plan (ADR 0008; limits set by task 0157 —
    // `pricingApiFreePlanRateLimit` / `pricingApiFreePlanMonthlyQuota`, not the design
    // doc's 100 req/s). `reservedConcurrentExecutions` is the optional SLO escape
    // hatch (only set when configured). API Gateway grants invoke via the
    // integration's resource policy (no role-cycle, unlike the SQS ESM above).
    const apiHandler = config.apiHandler;
    this.apiHandlerFunction = new lambda.Function(this, 'ApiHandlerFunction', {
      ...pricesLambdaDefaults, // ARM64 + PROVIDED_AL2023 (ADR 0006/0007)
      functionName: `prices-${envName}-api-handler`,
      handler: 'bootstrap',
      code: lambda.Code.fromAsset(API_HANDLER_ASSET_DIR),
      role: this.apiHandlerRole,
      logGroup: this.apiHandlerLogGroup,
      memorySize: apiHandler.memoryMb,
      timeout: cdk.Duration.seconds(apiHandler.timeoutSeconds),
      ...(apiHandler.reservedConcurrency !== undefined
        ? { reservedConcurrentExecutions: apiHandler.reservedConcurrency }
        : {}),
      tracing: lambda.Tracing.ACTIVE,
      layers: [secretsExtensionLayer],
      environment: {
        ENV_NAME: envName,
        RUST_LOG: 'info',
        CH_DOMAIN: chDomain,
        // Reader bundle ({cert,key,ca}) for CH user prices_reader.
        MTLS_SECRET_NAME: this.apiHandlerMtlsSecretName,
        // Build the mTLS CH client eagerly at cold start (primes the pool).
        CH_ENABLED: 'true',
        // Onboarding portal kept dark (task 0183). There is one environment and
        // it is production — `envName` is typed 'production' and `infra/envs/`
        // holds only production.json — so every portal slice (0184-0195) is
        // publicly reachable the moment it deploys unless this says otherwise.
        // Set explicitly rather than omitted: the Rust side defaults to false
        // either way, but spelling it here makes opening the portal a one-word
        // diff that shows up in a deploy review. Flipped by task 0194, after
        // 0189's eligibility gate passes — never as a side effect of finishing
        // some other slice.
        PORTAL_ENABLED: 'false',
        // The NAME of the portal's Discord OAuth bundle, never its value
        // (task 0186; ADR 0007's precedent, audited by Tranche 3 AC 6). The
        // handler reads the value through the Parameters & Secrets extension
        // layer already attached above — the same mechanism, the same cache, the
        // same "no secret in an env var" rule as `MTLS_SECRET_NAME`.
        //
        // Set unconditionally even though `PORTAL_ENABLED` is false, because the
        // Rust side only performs the read when the portal is open. Wiring the
        // name now means opening the portal stays the one-word diff the flag was
        // designed to be, rather than a two-line change made under time pressure.
        PORTAL_OAUTH_SECRET_NAME: this.portalOauthSecretName,
        // The NAME of the SSM parameter holding the `pricing-api-free` usage
        // plan id (task 0187) — see `portalFreePlanParameterName` for why it is
        // a name, why it is not a cross-stack reference, and why it is not
        // hard-coded. Read through the same extension layer, and only when the
        // portal is open, so a closed portal has no control-plane client in the
        // process at all.
        //
        // Set unconditionally alongside `PORTAL_OAUTH_SECRET_NAME`, and for the
        // same reason: opening the portal stays a one-word diff.
        PORTAL_FREE_PLAN_PARAM: this.portalFreePlanParameterName,
        // The NAMES of the eligibility gate's two SSM parameters (task 0189):
        // which Discord guild membership is checked against, and the minimum
        // account age in minutes. Names, never values — the handler resolves
        // them through the extension **per issuance**, so an operator's
        // `aws ssm put-parameter` takes effect without a redeploy (bounded
        // only by the extension's ~5 min cache). The values are
        // operator-seeded and deliberately NOT CloudFormation resources — see
        // the `PortalReadEligibilityParameters` statement above for the
        // un-flip-after-0179 hazard that rule prevents. Set unconditionally,
        // same one-word-diff reasoning as the two names above.
        PORTAL_GUILD_ID_PARAM: `/prices/${envName}/discord-guild-id`,
        PORTAL_MIN_ACCOUNT_AGE_PARAM: `/prices/${envName}/min-account-age-minutes`,
        // The free plan's per-key rate limit, for the portal dashboard to STATE
        // (task 0188) — the same `pricingApiFreePlanRateLimit` ApiGatewayStack
        // hands to `addUsagePlan`, so the figure on the page and the figure the
        // gateway enforces cannot disagree.
        //
        // It travels as an env var rather than being read back from
        // `GetUsagePlan` because that would cost the portal a control-plane
        // grant task 0188 deliberately does not take, and rather than being a
        // literal in the bundle because that is the one number on that panel
        // that could then go stale: raise the limit here, deploy, and a
        // dashboard whose stated theme is honesty would keep stating the old
        // one. Not a secret, and not conditional on `PORTAL_ENABLED` — same
        // one-word-diff reasoning as the two names above.
        PORTAL_RATE_LIMIT: String(config.pricingApiFreePlanRateLimit),
        PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED: 'true',
        // Strip the `/{stage}` prefix (`/production`) that API Gateway REST
        // proxy puts in the path, so lambda_http hands axum `/v1/...` (not
        // `/production/v1/...`). Without this every `/v1` route 404s — the axum
        // router is mounted at `/v1`, `/health`, etc. Mirrors BE's api-handler
        // (see BE `crates/api/src/common/edge_lock.rs`). The health check works
        // regardless only because it is a keyless API Gateway mock, not a
        // Lambda route, which is why this went unnoticed until the first real
        // deploy (0040's tests are all in-process `tower::oneshot`).
        AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH: 'true',
        // Stamped into the OpenAPI `servers` block served at /api-docs-json
        // (task 0124). Config-supplied, not derived from `api.url`: this stack
        // is a dependency of ApiGatewayStack, so reading the gateway's URL here
        // would close a Compute → Gateway → Compute cycle. Includes the stage
        // path — see the `apiBaseUrl` validation in types.ts.
        API_BASE_URL: config.apiBaseUrl,
      },
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
    new cdk.CfnOutput(this, 'ApiHandlerFunctionArn', {
      value: this.apiHandlerFunction.functionArn,
      description: `API Handler (axum) Lambda ARN (${envName})`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', envName);
  }
}
