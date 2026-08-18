import * as cdk from 'aws-cdk-lib';

/**
 * AWS-managed Lambda layer ARN for the
 * "AWS Parameters and Secrets Lambda Extension" (ARM64 build).
 *
 * AWS publishes this layer in every region from a region-specific
 * account; the version number is bumped roughly quarterly. This map
 * captures the regions we deploy into. If a new region is added,
 * extend the map — do NOT fall back to a single fixed ARN, because
 * cross-region layer references fail at deploy time with a confusing
 * "layer not found" error.
 *
 * Versions captured 2026-05-21 from the AWS docs (last AWS update
 * 2026-05-14). Bump these manually when refreshing — alternative
 * approach is the SSM dynamic reference
 * `{{resolve:ssm:/aws/service/aws-parameters-and-secrets-lambda-extension/arm64/latest}}`
 * which auto-resolves to the latest version at deploy time. Manual
 * pin chosen here for reproducibility; revisit if the rotation
 * cadence becomes painful.
 *
 * Source list: https://docs.aws.amazon.com/systems-manager/latest/userguide/ps-integration-lambda-extensions.html
 */
const SECRETS_EXTENSION_LAYER_ARN_ARM64: Record<string, string> = {
  'eu-central-1':
    'arn:aws:lambda:eu-central-1:187925254637:layer:AWS-Parameters-and-Secrets-Lambda-Extension-Arm64:78',
  'us-east-1':
    'arn:aws:lambda:us-east-1:177933569100:layer:AWS-Parameters-and-Secrets-Lambda-Extension-Arm64:80',
};

/**
 * Return the AWS-managed Secrets Manager Lambda Extension layer ARN
 * for the given region (ARM64 build). Throws on unknown regions so
 * synth fails fast instead of CloudFormation failing mid-deploy.
 */
export function secretsManagerLayerArn(region: string): string {
  const arn = SECRETS_EXTENSION_LAYER_ARN_ARM64[region];
  if (!arn) {
    throw new Error(
      `No AWS Parameters and Secrets Lambda Extension layer ARN known for region "${region}". ` +
        `Add it to SECRETS_EXTENSION_LAYER_ARN_ARM64 in infra/src/lib/mtls.ts. ` +
        `See https://docs.aws.amazon.com/secretsmanager/latest/userguide/retrieving-secrets_lambda.html for the regional ARN list.`,
    );
  }
  return arn;
}

/**
 * The two mTLS client identities prices-api presents to BE's Hetzner
 * ClickHouse, mirroring BE's per-service Lambda model:
 *
 * - `ingestion` → CH user `prices_writer` (ledger processor + periodic
 *   workers; `SELECT, INSERT, OPTIMIZE ON prices.*`).
 * - `api`       → CH user `prices_reader` (axum read handlers;
 *   `SELECT ON prices.*`).
 */
export type MtlsRole = 'ingestion' | 'api';

/**
 * Canonical mTLS client-cert CN for a role, env-suffixed to mirror BE
 * (`lambda-ingestion-production`). prices-api shares BE's CA, so these CNs
 * live in BE's CA namespace and must stay globally unique there — hence the
 * `-${envName}` suffix. The CN is the single thread tying the cert subject,
 * the Caddy `CLICKHOUSE_CN_USER_MAP` key, the CH user, and the secret name
 * together; keep all four derived from this one string.
 *
 *   mtlsClientCn('production', 'ingestion') === 'prices-ingestion-production'
 */
export function mtlsClientCn(envName: string, role: MtlsRole): string {
  return `prices-${role}-${envName}`;
}

/**
 * Secrets Manager secret name holding the single `{cert,key,ca}` JSON bundle
 * for a role — the value `MTLS_SECRET_NAME` resolves to at Lambda runtime
 * (see `packages/prices-clickhouse/src/mtls.rs`). The secret is created
 * out-of-band by the operator (BE-mirroring: CDK does NOT manage the material;
 * see `SecretsStack`), so this name must match the `--secret-id` the issuance
 * runbook uploads to (0063 `notes/G-provisioning-plan.md` §5).
 *
 *   mtlsSecretName('production', 'ingestion')
 *     === 'prices/production/clickhouse-mtls-prices-ingestion-production'
 */
export function mtlsSecretName(envName: string, role: MtlsRole): string {
  return `prices/${envName}/clickhouse-mtls-${mtlsClientCn(envName, role)}`;
}

/**
 * Secrets Manager secret name holding the onboarding portal's Discord OAuth
 * bundle — the value `PORTAL_OAUTH_SECRET_NAME` resolves to at Lambda runtime
 * (see `packages/prices-api/src/portal/auth/secret.rs`, task 0186).
 *
 * A JSON object with four fields:
 *
 * ```json
 * {
 *   "client_id":           "...",
 *   "client_secret":       "...",
 *   "redirect_uri":        "https://<host>/api-tokens/api/auth/callback",
 *   "session_signing_key": "<openssl rand -hex 32>"
 * }
 * ```
 *
 * Deliberately alongside {@link mtlsSecretName} rather than in a stack: this is
 * the third place the same rule applies, and putting the helper here is what
 * stops the name drifting between the IAM grant, the env var and the runbook the
 * operator types `create-secret` from. Same shape, same reasoning, one file.
 *
 * **Created out-of-band by the operator, never by CDK** — the same rule
 * `SecretsStack` states for the mTLS material, and here it is load-bearing for a
 * second reason: `redirect_uri` changes at the custom-domain cutover ([0195]),
 * and a CloudFormation-managed value would be silently restored to the committed
 * one by the next `cdk deploy`, un-pointing sign-in hours or weeks after the
 * cutover appeared to succeed.
 *
 *   portalOauthSecretName('production')
 *     === 'prices/production/portal-discord-oauth'
 */
export function portalOauthSecretName(envName: string): string {
  return `prices/${envName}/portal-discord-oauth`;
}

/**
 * Build the wildcard-suffixed Secrets Manager ARN for a secret name.
 *
 * AWS Secrets Manager appends a random 6-char suffix to every secret
 * ARN (e.g. `…secret:prices/production/clickhouse-mtls-prices-api-production-aBcDeF`).
 * IAM grants must use a wildcard to match. Returns the ARN form
 * `arn:aws:secretsmanager:<region>:<account>:secret:<name>-*`.
 *
 * The account ID is resolved via `cdk.Stack.of(scope).account` so this
 * works in both per-account synth and assumed-role deploys.
 */
export function mtlsSecretArn(scope: cdk.Stack, secretName: string): string {
  return mtlsSecretArnFromParts(scope.region, scope.account, secretName);
}

/**
 * Scope-free variant of {@link mtlsSecretArn} for callers that already hold
 * the region + account (e.g. `lambda-baseline` builds the grant from the
 * resolved `EnvironmentConfig.awsRegion` + account id, without a `Stack`).
 */
export function mtlsSecretArnFromParts(
  region: string,
  account: string,
  secretName: string,
): string {
  // Reject IAM-meaningful wildcards in the secret name. The function builds
  // an ARN like `…:secret:${secretName}-*` for IAM grants; an unexpected `*`
  // or `?` inside `secretName` would silently widen the grant beyond the
  // intended single-secret scope.
  if (/[*?]/.test(secretName)) {
    throw new Error(
      `mtlsSecretArn: secretName must not contain '*' or '?' (got: "${secretName}"); ` +
        `those characters widen the IAM grant beyond a single secret.`,
    );
  }
  return `arn:aws:secretsmanager:${region}:${account}:secret:${secretName}-*`;
}
