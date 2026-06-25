import * as cdk from 'aws-cdk-lib';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';
import { mtlsSecretName } from '../mtls.js';

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
 */
export class SecretsStack extends cdk.Stack {
  /** Secrets Manager name of the ingestion (writer) `{cert,key,ca}` bundle. */
  public readonly ingestionSecretName: string;
  /** Secrets Manager name of the api (reader) `{cert,key,ca}` bundle. */
  public readonly apiSecretName: string;

  constructor(scope: Construct, id: string, props: SecretsStackProps) {
    super(scope, id, props);

    const { envName } = props.config;

    this.ingestionSecretName = mtlsSecretName(envName, 'ingestion');
    this.apiSecretName = mtlsSecretName(envName, 'api');

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

    new cdk.CfnOutput(this, 'MtlsIngestionSecretName', {
      value: this.ingestionSecretName,
      description: `mTLS ingestion (writer) bundle secret name for ${envName}`,
    });
    new cdk.CfnOutput(this, 'MtlsApiSecretName', {
      value: this.apiSecretName,
      description: `mTLS api (reader) bundle secret name for ${envName}`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', envName);
  }
}
