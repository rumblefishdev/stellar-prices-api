import * as cdk from 'aws-cdk-lib';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
import type * as events from 'aws-cdk-lib/aws-events';
import * as targets from 'aws-cdk-lib/aws-events-targets';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as logs from 'aws-cdk-lib/aws-logs';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from './types.js';
import { mtlsSecretArnFromParts } from './mtls.js';

export interface BaselineLambdaContext {
  readonly config: EnvironmentConfig;
  readonly accountId: string;
  /**
   * Secrets Manager NAME of the single `{cert,key,ca}` bundle this Lambda
   * reads (its `MTLS_SECRET_NAME`). Each Lambda is granted read on only its
   * own bundle — least privilege, mirroring BE's per-service grant. The
   * secret is created out-of-band by the operator (see `SecretsStack`); the
   * grant is on the by-name wildcard ARN, so it does not require the secret
   * to exist at synth time. Derive with `mtlsSecretName(envName, role)`.
   */
  readonly mtlsSecretName: string;
}

/**
 * The IAM permissions every prices-api Lambda needs to function
 * against the post-ADR-0007 architecture:
 *
 * 1. CloudWatch Logs write — via AWSLambdaBasicExecutionRole managed
 *    policy (attached separately at role construction time).
 * 2. Read its own mTLS bundle secret from Secrets Manager (one secret,
 *    by-name wildcard ARN — BE-mirroring; the operator creates the value).
 * 3. Read both SSM namespaces — /platform/{env}/* (BE-published) and
 *    /prices/{env}/* (prices-api-published). Read-only here; the
 *    deploy role (CicdStack) is the only principal that writes.
 *
 * Downstream Lambdas extend these via `role.addToPolicy(...)` for
 * stack-specific needs (e.g. the ledger processor adds S3 read on
 * BE's ledger-data bucket — same-account, so a standard IAM policy
 * grant is sufficient with no bucket policy from BE).
 */
export function baselineLambdaPolicyStatements(
  ctx: BaselineLambdaContext,
): iam.PolicyStatement[] {
  const { config, accountId } = ctx;
  const region = config.awsRegion;
  const envName = config.envName;

  return [
    new iam.PolicyStatement({
      sid: 'ReadMtlsMaterial',
      actions: ['secretsmanager:GetSecretValue'],
      resources: [
        mtlsSecretArnFromParts(region, accountId, ctx.mtlsSecretName),
      ],
    }),
    new iam.PolicyStatement({
      sid: 'ReadSsmNamespaces',
      actions: [
        'ssm:GetParameter',
        'ssm:GetParameters',
        'ssm:GetParametersByPath',
      ],
      resources: [
        `arn:aws:ssm:${region}:${accountId}:parameter/platform/${envName}/*`,
        `arn:aws:ssm:${region}:${accountId}:parameter/prices/${envName}/*`,
      ],
    }),
  ];
}

/**
 * Canonical log-group name for a prices-api Lambda.
 *
 * Format: `/aws/lambda/prices-{env}-{lambdaName}` — matches BE's
 * naming convention (`/aws/lambda/{env}-soroban-explorer-{name}`)
 * but scoped to the `prices-` prefix so the two services never
 * collide in a shared CloudWatch view.
 */
export function lambdaLogGroupName(
  envName: string,
  lambdaName: string,
): string {
  return `/aws/lambda/prices-${envName}-${lambdaName}`;
}

/**
 * Deployed function name for a scheduled worker Lambda.
 *
 * Single source of truth: {@link createWorkerLambda} sets `functionName` from
 * this, and ObservabilityStack builds its `FunctionName` alarm dimension from
 * it too (task 0112). Alarms key on the deployed name as a plain string — an
 * alarm pointed at a name that does not exist does not error, it just never
 * fires — so a rename that updated only the stack would silently disarm the
 * alarms rather than break the build. Deriving both from here removes that
 * failure mode instead of relying on a reviewer to notice.
 */
export function workerFunctionName(
  envName: string,
  lambdaName: string,
): string {
  return `prices-${envName}-${lambdaName}`;
}

/**
 * Name of the invocation-errors alarm {@link createWorkerLambda} creates for a
 * scheduled worker.
 *
 * Same single-source-of-truth reasoning as {@link workerFunctionName}, one step
 * further out: these alarms are owned by EventBridgeStack and never re-exported,
 * so ObservabilityStack's dashboard imports them **by ARN** built from this
 * helper (task 0125). A drift between the two would not error — the alarm strip
 * would just silently cover nine fewer alarms than it claims.
 */
export function workerErrorAlarmName(
  envName: string,
  lambdaName: string,
): string {
  return `prices-${envName}-${lambdaName}-errors`;
}

/**
 * Creates an IAM role for a prices-api Lambda with the baseline
 * permissions applied (CloudWatch Logs + mTLS secrets + SSM read).
 *
 * Downstream tasks call `role.addToPolicy(...)` to attach
 * stack-specific permissions.
 */
export function createPricesLambdaRole(
  scope: Construct,
  id: string,
  ctx: BaselineLambdaContext,
): iam.Role {
  const role = new iam.Role(scope, id, {
    assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
    description: `Execution role for a prices-api Lambda (${ctx.config.envName}).`,
    managedPolicies: [
      iam.ManagedPolicy.fromAwsManagedPolicyName(
        'service-role/AWSLambdaBasicExecutionRole',
      ),
    ],
  });

  for (const statement of baselineLambdaPolicyStatements(ctx)) {
    role.addToPolicy(statement);
  }

  return role;
}

/**
 * Default runtime/architecture for prices-api Rust Lambdas, per
 * ADR 0006 (axum + cargo-lambda) and ADR 0007 (no VPC). Spread
 * this object into every `RustFunction` / `Function` props block
 * so the conventions stay aligned across stacks.
 *
 * Notably:
 * - Architecture is ARM_64 (Graviton) — ~10-20% cheaper than x86.
 * - Runtime is PROVIDED_AL2023 — the custom runtime that cargo-lambda
 *   bootstrap binaries target.
 * - No `vpc` / `vpcSubnets` / `securityGroups` set anywhere; ADR 0007
 *   §3.6 forbids VPC attachment.
 */
export const pricesLambdaDefaults = {
  architecture: lambda.Architecture.ARM_64,
  runtime: lambda.Runtime.PROVIDED_AL2023,
} as const;

/**
 * Default retention for prices-api Lambda log groups. ONE_MONTH
 * matches BE's convention and balances debugging surface against
 * CloudWatch storage cost.
 */
export const PRICES_LAMBDA_LOG_RETENTION = logs.RetentionDays.ONE_MONTH;

/**
 * Default removal policy for prices-api Lambda log groups. DESTROY
 * means re-deploying the stack deletes the historical log records;
 * acceptable because logs over 30 days old aren't load-bearing.
 * Per-env override can be added if production needs RETAIN.
 */
export const PRICES_LAMBDA_LOG_REMOVAL_POLICY = cdk.RemovalPolicy.DESTROY;

export interface WorkerLambdaProps extends BaselineLambdaContext {
  /**
   * PascalCase base for the construct (logical) ids: `${idPrefix}Role`,
   * `${idPrefix}LogGroup`, `${idPrefix}Function`, `${idPrefix}ErrorAlarm`.
   * Must match the existing ids so CloudFormation does not replace
   * resources (e.g. `Cleanup`, `Supply`, `Oracle`, `AssetDiscovery`).
   */
  readonly idPrefix: string;
  /**
   * Short kebab name used for the physical resource names:
   * `prices-{env}-{name}` (function), the log group suffix, and the
   * `prices-{env}-{name}-errors` alarm (e.g. `cleanup`, `supply`,
   * `oracle`, `asset-discovery`).
   */
  readonly name: string;
  /** cargo-lambda `bootstrap` asset directory for this worker. */
  readonly assetDir: string;
  readonly memorySize: number;
  readonly timeout: cdk.Duration;
  /** Shared Secrets Manager extension layer (one per stack). */
  readonly secretsExtensionLayer: lambda.ILayerVersion;
  /** ClickHouse mTLS endpoint (`CH_DOMAIN`). */
  readonly chDomain: string;
  /**
   * Worker-specific environment variables. Merged over the common set
   * every worker gets (`ENV_NAME`, `RUST_LOG`, `CH_DOMAIN`,
   * `MTLS_SECRET_NAME`, `PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED`).
   */
  readonly environment?: Record<string, string>;
  /** EventBridge rule wired as the worker's invocation target. */
  readonly rule: events.Rule;
  readonly alarmDescription: string;
  /** Period over which the error alarm sums invocation errors. */
  readonly alarmPeriod: cdk.Duration;
  /**
   * Actions wired to the worker's `-errors` alarm (e.g. an ops SNS topic).
   * Optional: the alarm is created either way, but with no action it is inert
   * (transitions to ALARM but notifies no one). Wire an action for any worker
   * whose error alarm is a load-bearing signal — notably the probes, whose
   * NOT_BREACHING metric alarms rely on this alarm as their dead-probe backstop.
   */
  readonly errorAlarmActions?: readonly cloudwatch.IAlarmAction[];
  /**
   * Lambda **async-invocation** retry attempts (0–2, default 2) for
   * function-execution errors. Applied via the function's `EventInvokeConfig`
   * (`configureAsyncInvoke`) — EventBridge invokes the target asynchronously, so
   * a *function* error is retried by Lambda's own async config, NOT the target's
   * `RetryPolicy` (which only covers EventBridge→Lambda *delivery* failures). Set
   * to 0 for a long-running best-effort worker where a failed run must not be
   * re-driven 2× more (each a full multi-minute walk); the next schedule resumes
   * the work anyway (task 0084 supply).
   */
  readonly asyncRetryAttempts?: number;
}

export interface WorkerLambda {
  readonly function: lambda.Function;
  readonly role: iam.Role;
  /** The `prices-{env}-{name}-errors` alarm, exposed so callers can attach
   * further actions or reference it in a dashboard. */
  readonly errorAlarm: cloudwatch.Alarm;
}

/**
 * Builds one periodic prices-api worker Lambda end to end: baseline IAM
 * role, log group, `Function` (ARM64 + PROVIDED_AL2023), EventBridge rule
 * target, and an informational error alarm. Collapses the four
 * near-identical wiring blocks (asset-discovery, cleanup, supply, oracle)
 * into a single factory.
 *
 * Returns the `function` and `role` so callers can attach worker-specific
 * permissions afterwards (e.g. asset-discovery grants S3 read on BE's
 * ledger bucket via `role`).
 */
export function createWorkerLambda(
  scope: Construct,
  props: WorkerLambdaProps,
): WorkerLambda {
  const {
    config,
    accountId,
    mtlsSecretName,
    idPrefix,
    name,
    assetDir,
    memorySize,
    timeout,
    secretsExtensionLayer,
    chDomain,
    rule,
    alarmDescription,
    alarmPeriod,
  } = props;
  const env = config.envName;

  const role = createPricesLambdaRole(scope, `${idPrefix}Role`, {
    config,
    accountId,
    mtlsSecretName,
  });

  const logGroup = new logs.LogGroup(scope, `${idPrefix}LogGroup`, {
    logGroupName: lambdaLogGroupName(env, name),
    retention: PRICES_LAMBDA_LOG_RETENTION,
    removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
  });

  const fn = new lambda.Function(scope, `${idPrefix}Function`, {
    ...pricesLambdaDefaults, // ARM64 + PROVIDED_AL2023 (ADR 0006/0007)
    functionName: workerFunctionName(env, name),
    handler: 'bootstrap',
    code: lambda.Code.fromAsset(assetDir),
    role,
    logGroup,
    memorySize,
    timeout,
    tracing: lambda.Tracing.ACTIVE,
    layers: [secretsExtensionLayer],
    environment: {
      ENV_NAME: env,
      RUST_LOG: 'info',
      CH_DOMAIN: chDomain,
      MTLS_SECRET_NAME: mtlsSecretName,
      PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED: 'true',
      ...props.environment,
    },
  });

  // Function-execution retries live on the Lambda's async invoke config, not the
  // EventBridge target's RetryPolicy (which only governs delivery). Set both to
  // the same bound so neither layer re-drives a long best-effort run.
  if (props.asyncRetryAttempts !== undefined) {
    fn.configureAsyncInvoke({ retryAttempts: props.asyncRetryAttempts });
  }
  rule.addTarget(
    new targets.LambdaFunction(fn, {
      retryAttempts: props.asyncRetryAttempts,
    }),
  );

  const errorAlarm = new cloudwatch.Alarm(scope, `${idPrefix}ErrorAlarm`, {
    alarmName: workerErrorAlarmName(env, name),
    alarmDescription,
    metric: fn.metricErrors({ period: alarmPeriod, statistic: 'Sum' }),
    threshold: 1,
    evaluationPeriods: 1,
    treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
  });
  // Both directions. Only the ALARM action was wired before, so a worker that
  // broke and later recovered produced one notification and then silence —
  // leaving the reader unable to tell "still broken" from "fixed itself".
  // Every alarm in ObservabilityStack already wires both; this brings the
  // per-worker error alarms in line (task 0112).
  for (const action of props.errorAlarmActions ?? []) {
    errorAlarm.addAlarmAction(action);
    errorAlarm.addOkAction(action);
  }

  return { function: fn, role, errorAlarm };
}
