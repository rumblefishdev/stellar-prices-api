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
