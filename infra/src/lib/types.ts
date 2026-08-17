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
   * Per-key sustained requests/second on the `pricing-api-free` usage plan.
   *
   * The design doc's §2.1 / §7 figure was 100 req/s, sized for a key we hand
   * out deliberately. Task 0157 overrides it: a key anybody can mint by signing
   * in must not be able to consume the sustained load the whole system is
   * load-tested against.
   */
  readonly pricingApiFreePlanRateLimit: number;
  /**
   * Token-bucket capacity for the `pricing-api-free` plan. Refill keeps the sustained
   * rate at `pricingApiFreePlanRateLimit`; burst only lets the allowance be spent
   * unevenly — enough that the quickstart's parallel example queries don't 429.
   */
  readonly pricingApiFreePlanBurstLimit: number;
  /**
   * Monthly request quota for the `pricing-api-free` plan (UsagePlan quota.limit).
   *
   * The operative limit a caller actually meets: at the per-second rate a key
   * could produce ~2.6M requests/month, so the quota binds ~26x harder than the
   * throttle. The period is in the name because a usage plan carries exactly one
   * quota — encoding it makes the unit impossible to misread.
   */
  readonly pricingApiFreePlanMonthlyQuota: number;
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
     * Rollup freshness probe (task 0137). Reads `now() - max(timestamp)` for
     * every OHLCV granularity and republishes it as the `Prices/Rollup`
     * `RollupLagSeconds` metric the per-tier rollup alarms watch. Every 15
     * minutes: the finest bound is 15 min, and a cadence coarser than the
     * tightest bound would let that tier breach and recover unobserved.
     */
    readonly rollupFreshnessProbe: string;
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
     * Per-tier rollup staleness thresholds in seconds, keyed by OHLCV table
     * name (task 0137). The rollup-freshness-probe publishes each tier's
     * `now() - max(timestamp)` as `Prices/Rollup` `RollupLagSeconds`; one alarm
     * per key fires when that tier's lag exceeds its threshold.
     *
     * ⚠️ **Every threshold must exceed its own bucket width.** `timestamp` is
     * the bucket *start*, so a healthy tier's lag sawtooths from 0 up to one
     * full bucket width before the next bucket opens — a `1w` tier reports a
     * six-day lag the day before rollover while perfectly healthy. Any
     * threshold at or below the bucket width false-fires once per bucket
     * forever, and a permanently-firing alarm gets muted, which is the exact
     * state task 0137 was filed to end.
     *
     * These values are duplicated from `ROLLUP_TIERS` in
     * `packages/rollup-freshness-probe/src/lib.rs`, which documents the full
     * rationale and unit-tests the bucket-width invariant. **This config is
     * authoritative for what the alarm actually does** — the Rust copy is
     * documentation and a test fixture, and a drift between them mis-tunes the
     * alarm without any test failing. Change both, or neither.
     */
    readonly rollupLagSeconds: Readonly<Record<string, number>>;
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
     * Free-space floor (percent) on the ClickHouse host's filesystem, below
     * which `prices-{env}-ch-disk-free` fires (task 0204, gap 1).
     *
     * The 2026-08-13 disk-full stall ran **11.5 h** and was discovered by
     * reading Lambda panic logs — nothing watched the condition. ⚠️ The volume
     * is **shared with the block-explorer team and we are 3.3% of it**, so we
     * can neither prevent it filling nor free a meaningful amount ourselves:
     * the only thing this alarm buys is **warning time**, and the threshold has
     * to be generous enough to deliver some.
     *
     * 20% of the 1.72 TiB volume is ~352 GiB. The incident consumed ~150 GiB,
     * so this fires with roughly twice that still free — hours of warning at the
     * rate that event moved — while sitting below the 2026-08-17 measurement of
     * 430.6 GiB free (25.0%), so it does not fire on the current steady state.
     * ⚠️ A bound at 25 would have been in ALARM the day it shipped.
     *
     * Mirrored as `DISK_FREE_PERCENT_BOUND` in
     * `packages/rollup-freshness-probe/src/disk.rs`, which documents the
     * reasoning and unit-tests it against both the measured steady state and a
     * replay of the incident. **This config is authoritative for what the alarm
     * does**; the Rust copy is documentation and a test fixture, and drift
     * between them mis-tunes the alarm without any test failing. Change both,
     * or neither.
     */
    readonly chDiskFreePercent: number;
    /**
     * Extra depths at which the ingest-DLQ alarm escalates (task 0204, gap 2).
     *
     * On 2026-08-13 Slack showed one message: `ApproximateNumberOfMessagesVisible
     * >= 1`. By morning the DLQ held **91**, and nobody reading the channel could
     * tell 1 from 91. That is not a tuning miss — a CloudWatch alarm notifies on
     * a **state transition**, so an alarm already latched in ALARM says nothing
     * further no matter how far the queue climbs.
     *
     * Each depth here becomes an additional alarm on the same metric, so a
     * growing DLQ crosses a new threshold and produces a new Slack message.
     * The `>= 1` rung is the pre-existing `prices-{env}-ledger-processor-dlq`
     * alarm and is NOT listed here — these are the rungs above it.
     *
     * Defaults to `[10, 50]`: 1 means a ledger was dropped and always warrants a
     * look; 10 means it is not a lone poison pill but something systemic; 50
     * means an outage is in progress (the 2026-08-13 event reached 91, so it
     * would have lit every rung).
     *
     * Must be strictly increasing integers above 1 — equal or descending rungs
     * would fire out of order and make the ladder unreadable.
     */
    readonly dlqEscalationDepths: readonly number[];
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
 * Worst-case lag a **healthy** tier reaches, per OHLCV granularity (task 0137).
 *
 * This is **not** a set of thresholds — it is the floor every
 * `opsAlarms.rollupLagSeconds` threshold must clear, and the list of tiers the
 * rollup alarms are expected to cover. Mirrors `ROLLUP_TIERS` in
 * `packages/rollup-freshness-probe/src/lib.rs`.
 *
 * A tier's healthy peak is its **bucket width plus the refresh interval of the
 * materialized view that feeds it** — the bucket cannot appear until that MV
 * next runs, so bucket width alone understates the peak and would let a
 * false-firing threshold pass validation. (`price_ohlcv_1w` at 8 d, for
 * instance, clears the 7 d bucket but not the real 8 d peak.)
 *
 * ⚠️ `price_ohlcv_1M` is the tightest tier and this floor still understates it:
 * buckets are weeks-attributed-by-start, so besides spanning ~31 days, a month's
 * first bucket does not appear until a week actually *starts* inside that month
 * — up to 6 further days. Its real worst case is nearer ~38 d against a 45 d
 * bound. Treat any proposal to lower it with suspicion.
 */
export const ROLLUP_HEALTHY_PEAK_SECONDS: Readonly<Record<string, number>> = {
  // bucket + refresh interval of the MV that feeds the tier (schema/rollups.sql)
  price_ohlcv_1m: 60, // 1 min, written by ingestion (no MV)
  price_ohlcv_15m: 15 * 60 + 60, // + mv_ohlcv_1m_to_15m  EVERY 1 MINUTE
  price_ohlcv_1h: 60 * 60 + 15 * 60, // + mv_ohlcv_15m_to_1h EVERY 15 MINUTE
  price_ohlcv_4h: 4 * 60 * 60 + 60 * 60, // + mv_ohlcv_1h_to_4h  EVERY 1 HOUR
  price_ohlcv_1d: 86_400 + 4 * 60 * 60, // + mv_ohlcv_4h_to_1d  EVERY 4 HOUR
  price_ohlcv_1w: 7 * 86_400 + 86_400, // + mv_ohlcv_1d_to_1w  EVERY 1 DAY
  // + mv_ohlcv_1w_to_1M EVERY 1 DAY, + 6 d alignment slack: a month's bucket
  // does not exist until a week actually STARTS inside that month.
  price_ohlcv_1M: 31 * 86_400 + 86_400 + 6 * 86_400,
};

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
    !Number.isInteger(config.pricingApiFreePlanRateLimit) ||
    config.pricingApiFreePlanRateLimit < 1
  ) {
    errors.push(
      `pricingApiFreePlanRateLimit must be a positive integer, got: ${config.pricingApiFreePlanRateLimit}`,
    );
  }
  // The `< 1` test is not redundant with the rate check below it. Errors are
  // accumulated, not short-circuited, so with an invalid rate of -10 a burst of
  // -5 would pass `burst < rate` and go unreported until the rate was fixed.
  if (
    !Number.isInteger(config.pricingApiFreePlanBurstLimit) ||
    config.pricingApiFreePlanBurstLimit < 1 ||
    config.pricingApiFreePlanBurstLimit < config.pricingApiFreePlanRateLimit
  ) {
    errors.push(
      `pricingApiFreePlanBurstLimit must be a positive integer >= pricingApiFreePlanRateLimit (${config.pricingApiFreePlanRateLimit}), got: ${config.pricingApiFreePlanBurstLimit}`,
    );
  }
  if (
    !Number.isInteger(config.pricingApiFreePlanMonthlyQuota) ||
    config.pricingApiFreePlanMonthlyQuota < 1
  ) {
    errors.push(
      `pricingApiFreePlanMonthlyQuota must be a positive integer, got: ${config.pricingApiFreePlanMonthlyQuota}`,
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

  // The floor the ratio implies, stated as its own rule rather than left to be
  // discovered through an instruction that cannot be followed. A plan limit must
  // be >= 1, so a stage default below MAX_PLAN_SHARE_OF_STAGE_DEFAULT admits no
  // legal plan limit at all: the ratio check would report "the maximum is 0"
  // while the checks above reject anything under 1. Fail here instead, naming
  // the field that is actually wrong. At 200 this is nowhere near binding, but
  // an operator clamping the stage throttle to shed load has to stay at or
  // above 10 — and lower the plan limits with it, not instead of it.
  const stageFloors: ReadonlyArray<readonly [string, number]> = [
    ['apiGatewayThrottleRate', config.apiGatewayThrottleRate],
    ['apiGatewayThrottleBurst', config.apiGatewayThrottleBurst],
  ];
  for (const [stageField, stageValue] of stageFloors) {
    if (
      Number.isInteger(stageValue) &&
      stageValue >= 1 &&
      stageValue < MAX_PLAN_SHARE_OF_STAGE_DEFAULT
    ) {
      errors.push(
        `${stageField} (${stageValue}) must be at least ${MAX_PLAN_SHARE_OF_STAGE_DEFAULT}: ` +
          `a usage-plan limit may be at most one ${MAX_PLAN_SHARE_OF_STAGE_DEFAULT}th of it and must itself be >= 1, ` +
          `so a lower stage default leaves no satisfiable plan limit`,
      );
    }
  }

  const planVsStage: ReadonlyArray<readonly [string, number, string, number]> =
    [
      [
        'pricingApiFreePlanRateLimit',
        config.pricingApiFreePlanRateLimit,
        'apiGatewayThrottleRate',
        config.apiGatewayThrottleRate,
      ],
      [
        'pricingApiFreePlanBurstLimit',
        config.pricingApiFreePlanBurstLimit,
        'apiGatewayThrottleBurst',
        config.apiGatewayThrottleBurst,
      ],
    ];
  // Both guards keep this from piling a derived error on top of a primary one:
  // with a stage rate of -200 the checks above already report it, and "the
  // maximum is -20" would add noise, not information. The stage-side guard is
  // the floor check rather than `>= 1` for the same reason — below 10 the floor
  // check above has already named the problem, and reporting "the maximum is 0"
  // alongside it would only tell the operator to do something impossible.
  for (const [planField, planValue, stageField, stageValue] of planVsStage) {
    if (
      Number.isInteger(stageValue) &&
      stageValue >= MAX_PLAN_SHARE_OF_STAGE_DEFAULT &&
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
      'rollupFreshnessProbe',
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
    // Percent, so bounded 0–100 exclusive at both ends: 0 can never fire and
    // 100 is always firing. Non-integers are allowed (a 12.5% floor is a
    // reasonable thing to want); NaN is not.
    if (
      typeof ops.chDiskFreePercent !== 'number' ||
      !Number.isFinite(ops.chDiskFreePercent) ||
      ops.chDiskFreePercent <= 0 ||
      ops.chDiskFreePercent >= 100
    ) {
      errors.push(
        `opsAlarms.chDiskFreePercent must be a number in (0, 100) exclusive, got: ${ops.chDiskFreePercent}`,
      );
    }
    if (!Array.isArray(ops.dlqEscalationDepths)) {
      errors.push('opsAlarms.dlqEscalationDepths missing or not an array');
    } else {
      // Rung 1 is the pre-existing `>= 1` alarm, so every configured rung must
      // sit above it, and they must ascend — a ladder that repeats or descends
      // fires out of order and tells the reader nothing about severity.
      let previous = 1;
      for (const depth of ops.dlqEscalationDepths) {
        if (!Number.isInteger(depth) || depth <= previous) {
          errors.push(
            `opsAlarms.dlqEscalationDepths must be strictly increasing integers above 1, got: [${ops.dlqEscalationDepths.join(', ')}]`,
          );
          break;
        }
        previous = depth;
      }
    }
    if (!ops.rollupLagSeconds || typeof ops.rollupLagSeconds !== 'object') {
      errors.push('opsAlarms.rollupLagSeconds missing or not an object');
    } else {
      const configured = Object.keys(ops.rollupLagSeconds).sort();
      const expected = Object.keys(ROLLUP_HEALTHY_PEAK_SECONDS).sort();
      if (configured.join(',') !== expected.join(',')) {
        errors.push(
          `opsAlarms.rollupLagSeconds must cover exactly [${expected.join(', ')}], got: [${configured.join(', ')}]`,
        );
      }
      for (const [table, peak] of Object.entries(ROLLUP_HEALTHY_PEAK_SECONDS)) {
        const threshold = ops.rollupLagSeconds[table];
        if (threshold === undefined) continue;
        if (!Number.isInteger(threshold) || threshold < 1) {
          errors.push(
            `opsAlarms.rollupLagSeconds.${table} must be a positive integer (seconds), got: ${threshold}`,
          );
        } else if (threshold <= peak) {
          // The sawtooth trap. `timestamp` is the bucket START, so a healthy
          // tier's lag climbs to a full bucket width — plus the refresh interval
          // of the MV feeding it — before the next bucket opens. A threshold at
          // or below that fires every bucket, forever, and an alarm that always
          // fires gets muted, which is precisely the blind spot task 0137 exists
          // to close. Reject at synth rather than ship an alarm guaranteed to
          // cry wolf.
          errors.push(
            `opsAlarms.rollupLagSeconds.${table} (${threshold}s) must exceed the tier's healthy peak of ${peak}s (bucket width + feeding MV refresh interval), or the alarm false-fires once per bucket forever`,
          );
        }
      }
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
