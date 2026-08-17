import * as cdk from 'aws-cdk-lib';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';
import { mtlsSecretName, portalOauthSecretName } from '../mtls.js';

export interface SecretsStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * Publishes the canonical Secrets Manager **names** for the prices-api mTLS
 * bundles. It deliberately does NOT create the secrets.
 *
 * ## Why no `new secretsmanager.Secret` (BE-mirroring)
 *
 * BE never CDK-manages the mTLS material for its Lambdas: `compute-stack.ts`
 * builds the secret *name*, grants `secretsmanager:GetSecretValue` on the
 * by-name ARN, sets `MTLS_SECRET_NAME`, and the operator creates the secret
 * out-of-band (`infra-hetzner/ca/issue-client-cert.sh` → `aws secretsmanager
 * create-secret`). We mirror that exactly:
 *
 * - The secret holds the **single `{cert,key,ca}` JSON bundle** that
 *   `packages/prices-clickhouse/src/mtls.rs` parses at runtime — NOT the old
 *   two-secret cert/key split this stack used to create. The CA private key
 *   never enters CDK; cert/key bytes are operator-issued and uploaded.
 * - Letting CloudFormation own the secret would (a) require a random
 *   placeholder that the runtime client cannot parse as a bundle, and (b)
 *   collide with the operator's `create-secret` (CFN refuses to create a name
 *   that already exists). Naming-only avoids both.
 *
 * Per the SSM key contract, only the prices-owned secret **names** are
 * published to `/prices/{env}/*` (identifiers, never trust material) so the
 * issuance runbook and any out-of-band tooling read one source of truth. The
 * names themselves come from {@link mtlsSecretName} — the same helper
 * `ComputeStack` uses for the IAM grant + `MTLS_SECRET_NAME`, so the two can
 * never drift (the failure mode we found in BE's own README-vs-CDK).
 *
 * Two identities (0063 decision, env-suffixed CNs):
 * - `prices/{env}/clickhouse-mtls-prices-ingestion-{env}` → `prices_writer`
 * - `prices/{env}/clickhouse-mtls-prices-api-{env}`       → `prices_reader`
 *
 * Plus one secret that is not mTLS material at all but obeys exactly the same
 * rule (task 0186): `prices/{env}/portal-discord-oauth`, the onboarding portal's
 * Discord application credentials. It is here rather than in a stack of its own
 * because "the operator owns the value, CDK owns only the name" is the property
 * this stack exists to state, and it now holds for three secrets rather than
 * two. For the OAuth bundle it is load-bearing twice over: its `redirect_uri`
 * field is re-pointed by hand at the custom-domain cutover ([0195]), and a
 * CloudFormation-managed value would be restored to the committed one by the
 * next deploy — silently breaking sign-in some time after the cutover looked
 * like it had worked.
 */
export class SecretsStack extends cdk.Stack {
  /** Secrets Manager name of the ingestion (writer) `{cert,key,ca}` bundle. */
  public readonly ingestionSecretName: string;
  /** Secrets Manager name of the api (reader) `{cert,key,ca}` bundle. */
  public readonly apiSecretName: string;
  /**
   * Secrets Manager name of the portal's Discord OAuth bundle (task 0186):
   * `{client_id, client_secret, redirect_uri, session_signing_key}`.
   */
  public readonly portalOauthSecretName: string;

  constructor(scope: Construct, id: string, props: SecretsStackProps) {
    super(scope, id, props);

    const { envName } = props.config;

    this.ingestionSecretName = mtlsSecretName(envName, 'ingestion');
    this.apiSecretName = mtlsSecretName(envName, 'api');
    this.portalOauthSecretName = portalOauthSecretName(envName);

    new ssm.StringParameter(this, 'MtlsIngestionSecretNameParam', {
      parameterName: `/prices/${envName}/mtls-ingestion-secret-name`,
      stringValue: this.ingestionSecretName,
      description:
        'Secrets Manager NAME of the prices-api ingestion (writer) mTLS ' +
        '{cert,key,ca} bundle. Operator creates the secret out-of-band; ' +
        'CDK only names + grants. Value = MTLS_SECRET_NAME for writer Lambdas.',
    });

    new ssm.StringParameter(this, 'MtlsApiSecretNameParam', {
      parameterName: `/prices/${envName}/mtls-api-secret-name`,
      stringValue: this.apiSecretName,
      description:
        'Secrets Manager NAME of the prices-api api (reader) mTLS ' +
        '{cert,key,ca} bundle. Operator creates the secret out-of-band; ' +
        'CDK only names + grants. Value = MTLS_SECRET_NAME for reader Lambdas.',
    });

    new ssm.StringParameter(this, 'PortalOauthSecretNameParam', {
      parameterName: `/prices/${envName}/portal-oauth-secret-name`,
      stringValue: this.portalOauthSecretName,
      description:
        "Secrets Manager NAME of the onboarding portal's Discord OAuth bundle " +
        '{client_id, client_secret, redirect_uri, session_signing_key}. ' +
        'Operator creates AND updates the secret out-of-band (the redirect_uri ' +
        'is re-pointed at the custom-domain cutover); CDK only names + grants. ' +
        'Value = PORTAL_OAUTH_SECRET_NAME on the api-handler Lambda.',
    });

    new cdk.CfnOutput(this, 'MtlsIngestionSecretName', {
      value: this.ingestionSecretName,
      description: `mTLS ingestion (writer) bundle secret name for ${envName}`,
    });
    new cdk.CfnOutput(this, 'MtlsApiSecretName', {
      value: this.apiSecretName,
      description: `mTLS api (reader) bundle secret name for ${envName}`,
    });

    new cdk.CfnOutput(this, 'PortalOauthSecretName', {
      value: this.portalOauthSecretName,
      description: `Portal Discord OAuth bundle secret name for ${envName}`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', envName);
  }
}
