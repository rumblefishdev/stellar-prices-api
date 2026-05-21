/**
 * Configuration for the shared CI/CD stack (consumed by CicdStack).
 *
 * One CicdStack is deployed once per AWS account — it provisions the
 * GitHub Actions OIDC provider and the per-env deploy roles. The
 * resulting role ARNs are stored as GitHub Environment secrets and
 * used by the deploy workflow (task 0008).
 */
export interface CicdConfig {
  readonly awsRegion: string;
  /** GitHub org/repo, e.g. "rumblefishdev/stellar-prices-api". */
  readonly githubRepo: string;
}

/**
 * Per-environment configuration for the prices-api CDK app.
 *
 * Only includes fields consumed by existing stacks. Each new stack
 * task extends this interface with the fields it needs — no
 * placeholder fields for stacks that do not exist yet. (Mirrors BE's
 * `soroban-block-explorer/infra/src/lib/types.ts` convention.)
 */
export interface EnvironmentConfig {
  readonly envName: 'staging' | 'production';
  readonly awsRegion: string;

  // API Gateway (consumed by ApiGatewayStack)

  /** Sustained requests per second before API Gateway returns 429. */
  readonly apiGatewayThrottleRate: number;
  /** Maximum concurrent requests allowed in a short burst above the rate limit. */
  readonly apiGatewayThrottleBurst: number;
  /** Daily request quota for API-key holders (UsagePlan quota.limit). */
  readonly apiGatewayPartnerDailyQuota: number;
}

/**
 * Validates an EnvironmentConfig at synth time. Throws on missing
 * or malformed values rather than letting `cdk synth`/`cdk deploy`
 * fail deep inside CloudFormation with cryptic errors.
 */
export function validateConfig(config: EnvironmentConfig): void {
  const errors: string[] = [];

  if (config.envName !== 'staging' && config.envName !== 'production') {
    errors.push(
      `envName must be "staging" or "production", got: "${config.envName}"`,
    );
  }

  if (!config.awsRegion || !/^[a-z]{2}-[a-z]+-\d+$/.test(config.awsRegion)) {
    errors.push(
      `awsRegion must be a valid AWS region (e.g. "us-east-1"), got: "${config.awsRegion}"`,
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

  if (errors.length > 0) {
    throw new Error(
      `Invalid EnvironmentConfig for "${config.envName}":\n  - ${errors.join(
        '\n  - ',
      )}`,
    );
  }
}
