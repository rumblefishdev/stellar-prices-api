/**
 * Configuration for the shared CI/CD stack (consumed by CicdStack).
 *
 * One CicdStack is deployed once per AWS account — it provisions the
 * GitHub Actions OIDC provider and the production deploy role. Both
 * prices-api and soroban-block-explorer share the same AWS account;
 * the two OIDC deploy roles are distinguished by role name prefix.
 * The resulting role ARN is stored as the GitHub Environment secret
 * used by the deploy workflow.
 */
export interface CicdConfig {
  readonly awsRegion: string;
  /** GitHub org/repo, e.g. "rumblefishdev/stellar-prices-api". */
  readonly githubRepo: string;
}

/**
 * Per-environment configuration for the prices-api CDK app.
 *
 * Production is the only supported AWS environment — staging was
 * retired during the eu-central-1 cutover (mirrors BE task 0239).
 * The single "production" environment is initially deployed with
 * conservative test-sized parameters so the AWS resource layout is
 * already in place; values are swapped to true production sizing
 * once the service is exercised in anger.
 *
 * Only includes fields consumed by existing stacks. Each new stack
 * task extends this interface with the fields it needs — no
 * placeholder fields for stacks that do not exist yet. (Mirrors BE's
 * `soroban-block-explorer/infra/src/lib/types.ts` convention.)
 */
export interface EnvironmentConfig {
  readonly envName: 'production';
  readonly awsRegion: string;

  // API Gateway (consumed by ApiGatewayStack)

  /**
   * Default per-method sustained requests/second on the stage before API Gateway
   * returns 429.
   *
   * NOT an aggregate across the stage: a per-API per-stage limit is applied per
   * method, so this value is granted to each method separately. Ten methods at
   * 200 is ten independent buckets of 200, not 200 shared ten ways.
   */
  readonly apiGatewayThrottleRate: number;
  /** Default per-method token-bucket capacity above the rate (same scope). */
  readonly apiGatewayThrottleBurst: number;
  /**
   * Per-key sustained requests/second on the self-service usage plan.
   *
   * The design doc's §2.1 / §7 figure was 100 req/s, sized for a key we hand
   * out deliberately. Task 0157 overrides it: a key anybody can mint by signing
   * in must not be able to consume the sustained load the whole system is
   * load-tested against.
   */
  readonly selfServicePlanRateLimit: number;
  /**
   * Token-bucket capacity for the self-service plan. Refill keeps the sustained
   * rate at `selfServicePlanRateLimit`; burst only lets the allowance be spent
   * unevenly — enough that the quickstart's parallel example queries don't 429.
   */
  readonly selfServicePlanBurstLimit: number;
  /**
   * Monthly request quota for the self-service plan (UsagePlan quota.limit).
   *
   * The operative limit a caller actually meets: at the per-second rate a key
   * could produce ~2.6M requests/month, so the quota binds ~26x harder than the
   * throttle. The period is in the name because a usage plan carries exactly one
   * quota — encoding it makes the unit impossible to misread.
   */
  readonly selfServicePlanMonthlyQuota: number;
  /**
   * Whether the API Gateway stage response cache (0.5 GB) is enabled. Per-route
   * TTLs are fixed in `ApiGatewayStack` per §2.1.
   */
  readonly apiGatewayCacheEnabled: boolean;

  /**
   * Public base URL of the deployed API, passed to the api-handler as
   * `API_BASE_URL` and stamped into the OpenAPI `servers` block (task 0124).
   *
   * MUST include the stage path. API Gateway serves the REST API at
   * `https://{id}.execute-api.{region}.amazonaws.com/{stage}`, so a value
   * without `/production` advertises a base that 403s on every route — the same
   * stage-prefix trap that made `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true`
   * necessary for `/v1` (task 0089).
   *
   * Configured rather than derived because ComputeStack (which owns the
   * function's environment) is a *dependency* of ApiGatewayStack (which owns
   * the URL) — reading `api.url` here would close the cycle Compute → Gateway →
   * Compute and fail synth. Update this one value when task 0126 lands the
   * custom domain.
   */
  readonly apiBaseUrl: string;

  // API handler Lambda (consumed by ComputeStack + ApiGatewayStack — task 0040)

  /**
   * Sizing for the single axum api-handler Lambda (ADR 0008 — one function
   * serves all routes). §2.1: 256–512 MB, 15 s timeout.
   */
  readonly apiHandler: {
    readonly memoryMb: number;
    readonly timeoutSeconds: number;
    /**
     * Reserved concurrency for the api-handler (the ADR 0008 escape hatch / SLO
     * protection for the hot `/price` path). Omit for on-demand scaling; set to
     * guarantee dedicated concurrency.
     */
    readonly reservedConcurrency?: number;
  };

  // EventBridge (consumed by EventBridgeStack)

  /**
   * Per-worker schedule expressions for task 0039's periodic Lambdas.
   * Rule shells are pre-created here; the worker Lambdas attach as
   * targets when 0039 lands.
   *
   * Rollup is intentionally absent — ADR 0007 §3.4 replaces it with
   * a ClickHouse materialised-view chain (1m → 15m → 1h → ...).
   */
  readonly scheduleExpressions: {
    /**
     * Per-asset circulating-supply fetch (Horizon) → prices.asset_supply.
     * (The former price-updater is eliminated — ADR 0007 §3.4 / 0039 Q#1
     * replace it with the prices.current_prices refreshable MV.)
     */
    readonly assetSupply: string;
    /** Polls Stellar on-chain oracles. */
    readonly oracleWatcher: string;
    /** Periodic asset-registry maintenance. */
    readonly assetDiscovery: string;
    /** Old-data partition drop (ALTER TABLE … DROP PARTITION). */
    readonly cleanup: string;
    /**
     * volume_quote_usd / close_usd enrichment pass over price_ohlcv_1m
     * (task 0026). Bounded-batch INSERT…SELECT into the ReplacingMergeTree.
     */
    readonly enrichment: string;
    /**
     * Backfill push-freshness probe (task 0056). Reads
     * `prices.backfill_progress.last_push_at` over mTLS and republishes each
     * stream's push age as the `Prices/Backfill` `PushAgeSeconds` metric the
     * SDEX freshness alarm watches. §5.6 cadence: every 15 minutes.
     */
    readonly backfillFreshnessProbe: string;
    /**
     * mTLS client-cert NotAfter probe (task 0056). Reads the per-role cert
     * bundles from Secrets Manager and publishes days-to-expiry as the
     * `Prices/Mtls` `MinDaysToNotAfter` metric the cert-expiry alarm watches.
     * Daily is ample for a 30-day threshold.
     */
    readonly mtlsNotafterProbe: string;
  };

  // Ops alarms + notification (consumed by ObservabilityStack — task 0056)

  /**
   * Tranche-1 ops alarms (§5.6 / §7 / §11.4): the SDEX push-freshness alarm and
   * the mTLS cert-expiry alarm, both routed to the `prices-{env}-ops-alarms`
   * SNS topic. Thresholds live here so they are operator-tunable per env
   * without a code change (§5.6 "threshold is operator-tunable").
   */
  readonly opsAlarms: {
    /**
     * Operator email seeded as an SNS subscription. Optional: when omitted the
     * topic is still created (subscriptions can be managed directly in SNS
     * without a redeploy); when set, CDK seeds this one address.
     */
    readonly notificationEmail?: string;
    /**
     * Freshness threshold (seconds) for `sdex_archive` push age. Default 7 days
     * (604800) — the first-chunk push covers ~6 months of history (§5.6).
     */
    readonly sdexPushFreshnessSeconds: number;
    /** Days-to-NotAfter below which the mTLS cert-expiry alarm fires (30). */
    readonly mtlsNotAfterDaysThreshold: number;
    /**
     * Ingestion-lag threshold (seconds) for the live ledger-processor alarm
     * (task 0056 finding B). Watches the `prices-ingest-{env}` SQS queue's
     * `ApproximateAgeOfOldestMessage` — the honest "processor is falling
     * behind" signal, since the ledger-processor emits no custom lag metric.
     * Ledgers close ~every 5–6 s and a healthy processor drains the doorbell in
     * seconds, so an oldest-message age sustained (5×1 min) above this means
     * live ingestion is lagging. Default 120 s — above the ADR 0007 / task 0038
     * `lag_seconds > 60s` intent, to give routine deploys / cold starts headroom
     * before the sustained-lag alarm pages (a real stall keeps climbing well
     * past it).
     */
    readonly ledgerProcessorLagSeconds: number;
    /**
     * Optional AWS Chatbot → Slack routing for the ops-alarms topic (task 0056).
     * When set, `ObservabilityStack` subscribes `prices-{env}-ops-alarms` to a
     * Slack channel via a `SlackChannelConfiguration`, so alarms land in Slack —
     * matching how BE routes its own CloudWatch alarms (no ops email/mailing
     * list). Prices has its **own** channel (`#stellar-prices-api-bot`), not BE's,
     * so these point at **prices-owned** params
     * (`/prices/{env}/slack-{workspace,channel}-id`) per the SSM ownership split.
     * The workspace ID is the same shared Slack workspace BE already authorized in
     * AWS Chatbot (only the channel differs), so no re-authorization is needed.
     * Omit to leave the topic subscriber-less (managed manually in SNS).
     *
     * Values are SSM Parameter *names* (plain String, not credentials), resolved
     * at deploy — the workspace/channel IDs stay out of this public repo. The
     * named params must exist before deploying Observability, else synth/deploy
     * fails on the lookup.
     */
    readonly slack?: {
      readonly workspaceIdSsmParam: string;
      readonly channelIdSsmParam: string;
    };
  };

  // Ledger Processor ingest (consumed by IngestStack — task 0038)

  /**
   * Sizing + SQS-source tuning for the live Prices Ledger Processor
   * Lambda. The Lambda is a content-free SQS "doorbell" consumer; per
   * the 2026-06-10 cross-team decision (task 0038 §C.1) the doorbells
   * arrive via SNS fan-out off BE's `stellar-ledger-data` bucket
   * (`S3 → SNS → prices-ingest SQS + DLQ → Lambda`).
   *
   * Mirrors BE's indexer knobs (`compute-stack.ts`): `batchSize = 1`
   * and `reservedConcurrency = 1` are **load-bearing for ordering**
   * — two concurrent invocations would race the cursor — not perf
   * preferences. `maxReceiveCount = 10` (vs the usual 3) absorbs the
   * ESM over-poll/throttle churn that `concurrency = 1` induces so a
   * processable doorbell is never false-DLQ'd.
   */
  readonly ledgerProcessor: {
    /** Lambda memory (MB). */
    readonly memoryMb: number;
    /** Lambda timeout (seconds). The SQS visibility timeout is set to this + 60s. */
    readonly timeoutSeconds: number;
    /** Reserved concurrency. MUST be 1 — serial execution is the ordering guarantee. */
    readonly reservedConcurrency: number;
    /** SQS event-source batch size. 1 mirrors BE (doorbell, body ignored). */
    readonly sqsBatchSize: number;
    /** SQS redrive threshold before a message lands in the DLQ. */
    readonly maxReceiveCount: number;
    /**
     * Max contiguous ledgers walked per reconcile run (`MAX_ITERATIONS`).
     * Bounds one invocation's S3 fetch + decode budget against the Lambda
     * timeout; the Rust default is 16.
     */
    readonly maxIterations: number;
    /**
     * KMS key ARN protecting BE's `stellar-ledger-data` bucket, if it is
     * SSE-KMS encrypted. When set, the ledger-processor role is granted
     * `kms:Decrypt` on this key — `grantRead` on a bucket imported by
     * attributes (no `encryptionKey`) does NOT add it, so without this a
     * KMS-encrypted bucket returns `AccessDenied` on every `GetObject`
     * (which the fetcher maps to a hard error that DLQ's the doorbell, not
     * a gap). Leave unset for an SSE-S3 / unencrypted bucket. Confirm with
     * BE (task 0038 §C.2).
     */
    readonly bucketKmsKeyArn?: string;
  };
}

/**
 * Validates an EnvironmentConfig at synth time. Throws on missing
 * or malformed values rather than letting `cdk synth`/`cdk deploy`
 * fail deep inside CloudFormation with cryptic errors.
 */
export function validateConfig(config: EnvironmentConfig): void {
  const errors: string[] = [];

  if (config.envName !== 'production') {
    errors.push(`envName must be "production", got: "${config.envName}"`);
  }

  if (!config.awsRegion || !/^[a-z]{2}-[a-z]+-\d+$/.test(config.awsRegion)) {
    errors.push(
      `awsRegion must be a valid AWS region (e.g. "eu-central-1"), got: "${config.awsRegion}"`,
    );
  }

  if (
    !Number.isInteger(config.apiGatewayThrottleRate) ||
    config.apiGatewayThrottleRate < 1
  ) {
    errors.push(
      `apiGatewayThrottleRate must be a positive integer, got: ${config.apiGatewayThrottleRate}`,
    );
  }
  // `< 1` for the same reason as the self-service burst check below: errors are
  // accumulated, so without it an invalid rate of -5 lets a burst of -3 through
  // unreported (-3 >= -5).
  if (
    !Number.isInteger(config.apiGatewayThrottleBurst) ||
    config.apiGatewayThrottleBurst < 1 ||
    config.apiGatewayThrottleBurst < config.apiGatewayThrottleRate
  ) {
    errors.push(
      `apiGatewayThrottleBurst must be a positive integer >= apiGatewayThrottleRate (${config.apiGatewayThrottleRate}), got: ${config.apiGatewayThrottleBurst}`,
    );
  }
  if (
    !Number.isInteger(config.selfServicePlanRateLimit) ||
    config.selfServicePlanRateLimit < 1
  ) {
    errors.push(
      `selfServicePlanRateLimit must be a positive integer, got: ${config.selfServicePlanRateLimit}`,
    );
  }
  // The `< 1` test is not redundant with the rate check below it. Errors are
  // accumulated, not short-circuited, so with an invalid rate of -10 a burst of
  // -5 would pass `burst < rate` and go unreported until the rate was fixed.
  if (
    !Number.isInteger(config.selfServicePlanBurstLimit) ||
    config.selfServicePlanBurstLimit < 1 ||
    config.selfServicePlanBurstLimit < config.selfServicePlanRateLimit
  ) {
    errors.push(
      `selfServicePlanBurstLimit must be a positive integer >= selfServicePlanRateLimit (${config.selfServicePlanRateLimit}), got: ${config.selfServicePlanBurstLimit}`,
    );
  }
  if (
    !Number.isInteger(config.selfServicePlanMonthlyQuota) ||
    config.selfServicePlanMonthlyQuota < 1
  ) {
    errors.push(
      `selfServicePlanMonthlyQuota must be a positive integer, got: ${config.selfServicePlanMonthlyQuota}`,
    );
  }
  if (typeof config.apiGatewayCacheEnabled !== 'boolean') {
    errors.push(
      `apiGatewayCacheEnabled must be a boolean, got: ${config.apiGatewayCacheEnabled}`,
    );
  }
  // `servers` in the published OpenAPI document is a promise that the URL
  // serves the API. Assert the shape here rather than discovering at runtime
  // that the advertised base is missing its stage path and 403s (task 0124).
  if (typeof config.apiBaseUrl !== 'string' || !config.apiBaseUrl) {
    errors.push(
      `apiBaseUrl must be a non-empty string, got: ${config.apiBaseUrl}`,
    );
  } else {
    if (!config.apiBaseUrl.startsWith('https://')) {
      errors.push(
        `apiBaseUrl must start with "https://", got: "${config.apiBaseUrl}"`,
      );
    }
    if (config.apiBaseUrl.endsWith('/')) {
      errors.push(
        `apiBaseUrl must not end with "/" (routes are appended as "/v1/..."), got: "${config.apiBaseUrl}"`,
      );
    }
    // An execute-api host serves the API only under /{stage}; a bare host is
    // the stage-prefix trap. A custom domain (task 0126) has no such
    // requirement, so only enforce this for execute-api URLs.
    if (
      config.apiBaseUrl.includes('.execute-api.') &&
      !config.apiBaseUrl.endsWith(`/${config.envName}`)
    ) {
      errors.push(
        `apiBaseUrl is an execute-api URL and must end with the stage path "/${config.envName}", got: "${config.apiBaseUrl}"`,
      );
    }
  }

  // A self-issued key must not be sized anywhere near what the stage hands a
  // single method — that is the entire reason task 0157 exists. Cap the plan at
  // one tenth of the stage default, for both rate and burst.
  //
  // Read the ratio for what it is: a proportionality guard, NOT a capacity
  // calculation. A per-method default is not a shared pool that ten keys "fit
  // under" — every method gets its own bucket, so the plan limit and the method
  // default never compete for the same tokens. The number 10 is a judgement
  // call, and its real job is to stop the design doc's 100 req/s being
  // reinstated by typo: at a default of 200, a rate of 1 passes and 100 does
  // not. A bare `<=` against the default would not catch it (200 >= 100).
  //
  // The only genuinely shared ceiling above the usage plan is the account limit
  // (10 000 RPS / 5 000 burst in eu-central-1), which nothing here approaches.
  const MAX_PLAN_SHARE_OF_STAGE_DEFAULT = 10;
  const planVsStage: ReadonlyArray<readonly [string, number, string, number]> =
    [
      [
        'selfServicePlanRateLimit',
        config.selfServicePlanRateLimit,
        'apiGatewayThrottleRate',
        config.apiGatewayThrottleRate,
      ],
      [
        'selfServicePlanBurstLimit',
        config.selfServicePlanBurstLimit,
        'apiGatewayThrottleBurst',
        config.apiGatewayThrottleBurst,
      ],
    ];
  // Both `>= 1` guards keep this from piling a derived error on top of a primary
  // one: with a stage rate of -200 the checks above already report it, and
  // "the maximum is -20" would add noise, not information.
  //
  // Note the implied floor this creates: since a plan limit must be >= 1, a
  // stage default below MAX_PLAN_SHARE_OF_STAGE_DEFAULT makes the config
  // unsatisfiable. At 200 that is nowhere near binding, but anyone dialling the
  // stage throttle down has to stay at or above 10.
  for (const [planField, planValue, stageField, stageValue] of planVsStage) {
    if (
      Number.isInteger(stageValue) &&
      stageValue >= 1 &&
      Number.isInteger(planValue) &&
      planValue >= 1 &&
      planValue * MAX_PLAN_SHARE_OF_STAGE_DEFAULT > stageValue
    ) {
      errors.push(
        `${planField} (${planValue}) exceeds one ${MAX_PLAN_SHARE_OF_STAGE_DEFAULT}th of ${stageField} (${stageValue}): ` +
          `a self-service key must not be sized within an order of magnitude of the stage's default per-method limit, ` +
          `so the maximum is ${Math.floor(stageValue / MAX_PLAN_SHARE_OF_STAGE_DEFAULT)}`,
      );
    }
  }

  const api = config.apiHandler;
  if (!api || typeof api !== 'object') {
    errors.push('apiHandler missing or not an object');
  } else {
    if (!Number.isInteger(api.memoryMb) || api.memoryMb < 128) {
      errors.push(
        `apiHandler.memoryMb must be an integer >= 128, got: ${api.memoryMb}`,
      );
    }
    if (!Number.isInteger(api.timeoutSeconds) || api.timeoutSeconds < 1) {
      errors.push(
        `apiHandler.timeoutSeconds must be a positive integer, got: ${api.timeoutSeconds}`,
      );
    }
    if (
      api.reservedConcurrency !== undefined &&
      (!Number.isInteger(api.reservedConcurrency) ||
        api.reservedConcurrency < 1)
    ) {
      errors.push(
        `apiHandler.reservedConcurrency, when set, must be a positive integer, got: ${api.reservedConcurrency}`,
      );
    }
  }

  const schedules = config.scheduleExpressions;
  if (!schedules || typeof schedules !== 'object') {
    errors.push(`scheduleExpressions missing or not an object`);
  } else {
    const expectedKeys = [
      'assetSupply',
      'oracleWatcher',
      'assetDiscovery',
      'cleanup',
      'enrichment',
      'backfillFreshnessProbe',
      'mtlsNotafterProbe',
    ] as const;
    for (const key of expectedKeys) {
      const value = schedules[key];
      if (typeof value !== 'string' || value.trim().length === 0) {
        errors.push(`scheduleExpressions.${key} missing or empty`);
      } else if (!/^(rate\(|cron\()/.test(value)) {
        errors.push(
          `scheduleExpressions.${key} must start with 'rate(' or 'cron(', got: "${value}"`,
        );
      }
    }
  }

  const ops = config.opsAlarms;
  if (!ops || typeof ops !== 'object') {
    errors.push('opsAlarms missing or not an object');
  } else {
    if (
      !Number.isInteger(ops.sdexPushFreshnessSeconds) ||
      ops.sdexPushFreshnessSeconds < 1
    ) {
      errors.push(
        `opsAlarms.sdexPushFreshnessSeconds must be a positive integer (seconds), got: ${ops.sdexPushFreshnessSeconds}`,
      );
    }
    if (
      !Number.isInteger(ops.mtlsNotAfterDaysThreshold) ||
      ops.mtlsNotAfterDaysThreshold < 1
    ) {
      errors.push(
        `opsAlarms.mtlsNotAfterDaysThreshold must be a positive integer (days), got: ${ops.mtlsNotAfterDaysThreshold}`,
      );
    }
    if (
      !Number.isInteger(ops.ledgerProcessorLagSeconds) ||
      ops.ledgerProcessorLagSeconds < 1
    ) {
      errors.push(
        `opsAlarms.ledgerProcessorLagSeconds must be a positive integer (seconds), got: ${ops.ledgerProcessorLagSeconds}`,
      );
    }
    if (ops.slack !== undefined) {
      const isSsmName = (v: unknown): boolean =>
        typeof v === 'string' && v.startsWith('/') && v.length > 1;
      if (
        typeof ops.slack !== 'object' ||
        ops.slack === null ||
        !isSsmName(ops.slack.workspaceIdSsmParam) ||
        !isSsmName(ops.slack.channelIdSsmParam)
      ) {
        errors.push(
          'opsAlarms.slack, when set, must be { workspaceIdSsmParam, channelIdSsmParam } with absolute SSM parameter names (leading "/")',
        );
      }
    }
    // Require a real local@domain.tld shape, not just a stray '@': a value like
    // '@' or 'ops@' passes an includes('@') check but yields an undeliverable
    // SNS subscription (a silent notification black hole).
    const emailShape = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (
      ops.notificationEmail !== undefined &&
      (typeof ops.notificationEmail !== 'string' ||
        !emailShape.test(ops.notificationEmail))
    ) {
      errors.push(
        `opsAlarms.notificationEmail, when set, must be an email address, got: ${ops.notificationEmail}`,
      );
    }
  }

  const lp = config.ledgerProcessor;
  if (!lp || typeof lp !== 'object') {
    errors.push('ledgerProcessor missing or not an object');
  } else {
    if (!Number.isInteger(lp.memoryMb) || lp.memoryMb < 128) {
      errors.push(
        `ledgerProcessor.memoryMb must be an integer >= 128, got: ${lp.memoryMb}`,
      );
    }
    if (!Number.isInteger(lp.timeoutSeconds) || lp.timeoutSeconds < 1) {
      errors.push(
        `ledgerProcessor.timeoutSeconds must be a positive integer, got: ${lp.timeoutSeconds}`,
      );
    }
    // Ordering correctness depends on serial execution — reject anything
    // but 1. Two concurrent invocations would race the cursor (BE's
    // load-bearing `reservedConcurrentExecutions = 1`, mirrored here).
    if (lp.reservedConcurrency !== 1) {
      errors.push(
        `ledgerProcessor.reservedConcurrency must be exactly 1 (serial execution is the ordering guarantee), got: ${lp.reservedConcurrency}`,
      );
    }
    if (!Number.isInteger(lp.sqsBatchSize) || lp.sqsBatchSize < 1) {
      errors.push(
        `ledgerProcessor.sqsBatchSize must be a positive integer, got: ${lp.sqsBatchSize}`,
      );
    }
    if (!Number.isInteger(lp.maxReceiveCount) || lp.maxReceiveCount < 1) {
      errors.push(
        `ledgerProcessor.maxReceiveCount must be a positive integer, got: ${lp.maxReceiveCount}`,
      );
    }
    if (!Number.isInteger(lp.maxIterations) || lp.maxIterations < 1) {
      errors.push(
        `ledgerProcessor.maxIterations must be a positive integer, got: ${lp.maxIterations}`,
      );
    }
    if (
      lp.bucketKmsKeyArn !== undefined &&
      (typeof lp.bucketKmsKeyArn !== 'string' ||
        !lp.bucketKmsKeyArn.startsWith('arn:aws:kms:'))
    ) {
      errors.push(
        `ledgerProcessor.bucketKmsKeyArn, when set, must be a KMS key ARN, got: ${lp.bucketKmsKeyArn}`,
      );
    }
  }

  if (errors.length > 0) {
    throw new Error(
      `Invalid EnvironmentConfig for "${config.envName}":\n  - ${errors.join(
        '\n  - ',
      )}`,
    );
  }
}
