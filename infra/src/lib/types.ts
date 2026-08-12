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

  /** Stage-wide sustained requests per second before API Gateway returns 429. */
  readonly apiGatewayThrottleRate: number;
  /** Stage-wide maximum concurrent requests in a short burst above the rate. */
  readonly apiGatewayThrottleBurst: number;
  /** Daily request quota for API-key holders (UsagePlan quota.limit). */
  readonly apiGatewayPartnerDailyQuota: number;
  /**
   * Per-API-key sustained requests/second (UsagePlan throttle). The §2.1 / §7
   * contract is 100 req/s per key; this is the value enforced per key holder.
   */
  readonly apiKeyRateLimit: number;
  /** Per-API-key burst limit (UsagePlan throttle). */
  readonly apiKeyBurstLimit: number;
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
  if (
    !Number.isInteger(config.apiGatewayThrottleBurst) ||
    config.apiGatewayThrottleBurst < config.apiGatewayThrottleRate
  ) {
    errors.push(
      `apiGatewayThrottleBurst must be a positive integer >= apiGatewayThrottleRate (${config.apiGatewayThrottleRate}), got: ${config.apiGatewayThrottleBurst}`,
    );
  }
  if (
    !Number.isInteger(config.apiGatewayPartnerDailyQuota) ||
    config.apiGatewayPartnerDailyQuota < 1
  ) {
    errors.push(
      `apiGatewayPartnerDailyQuota must be a positive integer, got: ${config.apiGatewayPartnerDailyQuota}`,
    );
  }
  if (!Number.isInteger(config.apiKeyRateLimit) || config.apiKeyRateLimit < 1) {
    errors.push(
      `apiKeyRateLimit must be a positive integer, got: ${config.apiKeyRateLimit}`,
    );
  }
  if (
    !Number.isInteger(config.apiKeyBurstLimit) ||
    config.apiKeyBurstLimit < config.apiKeyRateLimit
  ) {
    errors.push(
      `apiKeyBurstLimit must be a positive integer >= apiKeyRateLimit (${config.apiKeyRateLimit}), got: ${config.apiKeyBurstLimit}`,
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

  // The stage-wide throttle is a hard ceiling across ALL keys, so it must be at
  // least the advertised per-key rate — otherwise a single key can never reach
  // its SLA and compliant traffic gets spurious 429s.
  if (
    Number.isInteger(config.apiGatewayThrottleRate) &&
    Number.isInteger(config.apiKeyRateLimit) &&
    config.apiGatewayThrottleRate < config.apiKeyRateLimit
  ) {
    errors.push(
      `apiGatewayThrottleRate (${config.apiGatewayThrottleRate}) must be >= apiKeyRateLimit (${config.apiKeyRateLimit}) so a single key can reach its per-key SLA`,
    );
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
