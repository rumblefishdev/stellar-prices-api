/**
 * Configuration for the shared CI/CD stack (consumed by CicdStack).
 *
 * One CicdStack is deployed once per AWS account — it provisions the
 * GitHub Actions OIDC provider and the production deploy role. The
 * resulting role ARN is stored as the GitHub Environment secret used
 * by the deploy workflow.
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

  /** Sustained requests per second before API Gateway returns 429. */
  readonly apiGatewayThrottleRate: number;
  /** Maximum concurrent requests allowed in a short burst above the rate limit. */
  readonly apiGatewayThrottleBurst: number;
  /** Daily request quota for API-key holders (UsagePlan quota.limit). */
  readonly apiGatewayPartnerDailyQuota: number;

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
    /** Refreshes `current_prices` aggregations. */
    readonly priceUpdater: string;
    /** Polls Stellar on-chain oracles. */
    readonly oracleWatcher: string;
    /** Periodic asset-registry maintenance. */
    readonly assetDiscovery: string;
    /** Old-data partition drop (ALTER TABLE … DROP PARTITION). */
    readonly cleanup: string;
  };

  // Hetzner ClickHouse — mTLS (consumed by ComputeStack, downstream
  // tasks 0038/0039/0040/0052 that talk to the BE-hosted ClickHouse)

  /**
   * FQDN that BE's HetznerDnsStack maps to the Hetzner ClickHouse box.
   * Used as the mTLS endpoint hostname by every prices-api Lambda
   * that reads from / writes to ClickHouse (passed as `CH_DOMAIN`
   * env var per the BE 0239 convention).
   *
   * BE owns the DNS record itself; prices-api just consumes the FQDN
   * via this config.
   */
  readonly chDomainName: string;
  /**
   * Secret-name prefix in AWS Secrets Manager for mTLS client cert
   * bundles. Each prices-api Lambda gets its own secret at
   * `${mtlsSecretNamePrefix}/<service-cn>` containing the
   * `{cert, key, ca}` PEM bundle issued by BE task 0050's CA pipeline.
   *
   * Example: `prices/production/mtls` → service secrets live at
   * `prices/production/mtls/ledger-processor-production`,
   * `prices/production/mtls/api-handler-production`, etc.
   *
   * Downstream stacks construct the full ARN at synth time using
   * `mtlsSecretArn` from `lib/mtls.ts` so cross-team naming stays
   * IAM-scopeable (single secret per Lambda, no wildcard sprawl).
   */
  readonly mtlsSecretNamePrefix: string;
}

/**
 * Validates an EnvironmentConfig at synth time. Throws on missing
 * or malformed values rather than letting `cdk synth`/`cdk deploy`
 * fail deep inside CloudFormation with cryptic errors.
 */
export function validateConfig(config: EnvironmentConfig): void {
  const errors: string[] = [];

  if (config.envName !== 'production') {
    errors.push(
      `envName must be "production", got: "${config.envName}"`,
    );
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

  const schedules = config.scheduleExpressions;
  if (!schedules || typeof schedules !== 'object') {
    errors.push(`scheduleExpressions missing or not an object`);
  } else {
    const expectedKeys = [
      'priceUpdater',
      'oracleWatcher',
      'assetDiscovery',
      'cleanup',
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

  if (!config.chDomainName) {
    errors.push(`chDomainName missing`);
  } else if (
    config.chDomainName.includes('CHANGE') ||
    config.chDomainName.includes('PLACEHOLDER')
  ) {
    errors.push(
      `chDomainName placeholder rejected: "${config.chDomainName}"`,
    );
  }

  if (
    !config.mtlsSecretNamePrefix ||
    config.mtlsSecretNamePrefix.includes('CHANGE') ||
    config.mtlsSecretNamePrefix.includes('PLACEHOLDER')
  ) {
    errors.push(
      `mtlsSecretNamePrefix missing or placeholder: "${config.mtlsSecretNamePrefix}"`,
    );
  }

  if (errors.length > 0) {
    throw new Error(
      `Invalid EnvironmentConfig for "${config.envName}":\n  - ${errors.join(
        '\n  - ',
      )}`,
    );
  }
}
