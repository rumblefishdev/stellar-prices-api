import * as cdk from 'aws-cdk-lib';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface SecretsStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * Secrets Manager slots for the mTLS material that prices-api uses
 * to connect to BE's Hetzner ClickHouse over HTTPS-mTLS.
 *
 * Per ADR 0007 §3.5: two secrets per env (cert + key, separately).
 * BE's per-AWS-service issuance script (task 0050) produces the real
 * PEMs; an operator uploads them post-deploy via:
 *
 *     aws secretsmanager put-secret-value \
 *         --secret-id prices/{env}/clickhouse-mtls-cert \
 *         --secret-string "$(cat <cert>.pem)"
 *
 *     aws secretsmanager put-secret-value \
 *         --secret-id prices/{env}/clickhouse-mtls-key \
 *         --secret-string "$(cat <key>.pem)"
 *
 * The CDK template intentionally does NOT contain the PEM values —
 * `generateSecretString` creates a random placeholder on first
 * deploy; subsequent `cdk deploy` invocations do not re-randomize as
 * long as the generator parameters are unchanged. Re-running deploy
 * after the operator upload leaves the real PEMs intact.
 *
 * The Secret ARNs are published to SSM under the prices-api-owned
 * namespace (`/prices/{env}/mtls-{cert,key}-secret-arn`) so task
 * 0052's `clickhouse-client` crate can read them at Lambda init.
 */
export class SecretsStack extends cdk.Stack {
  public readonly mtlsCertSecret: secretsmanager.ISecret;
  public readonly mtlsKeySecret: secretsmanager.ISecret;

  constructor(scope: Construct, id: string, props: SecretsStackProps) {
    super(scope, id, props);

    const { envName } = props.config;

    this.mtlsCertSecret = new secretsmanager.Secret(this, 'MtlsCertSecret', {
      secretName: `prices/${envName}/clickhouse-mtls-cert`,
      description:
        `mTLS client certificate (PEM) for prices-api → BE Hetzner ClickHouse, ${envName}. ` +
        `Initial value is a CDK-generated random placeholder; operator replaces with the real ` +
        `cert via 'aws secretsmanager put-secret-value' after BE task 0050 issuance.`,
      generateSecretString: {
        passwordLength: 64,
        excludePunctuation: true,
      },
      removalPolicy: cdk.RemovalPolicy.RETAIN,
    });

    this.mtlsKeySecret = new secretsmanager.Secret(this, 'MtlsKeySecret', {
      secretName: `prices/${envName}/clickhouse-mtls-key`,
      description:
        `mTLS client private key (PEM) for prices-api → BE Hetzner ClickHouse, ${envName}. ` +
        `Initial value is a CDK-generated random placeholder; operator replaces with the real ` +
        `key via 'aws secretsmanager put-secret-value' after BE task 0050 issuance.`,
      generateSecretString: {
        passwordLength: 64,
        excludePunctuation: true,
      },
      removalPolicy: cdk.RemovalPolicy.RETAIN,
    });

    new ssm.StringParameter(this, 'MtlsCertSecretArnParam', {
      parameterName: `/prices/${envName}/mtls-cert-secret-arn`,
      stringValue: this.mtlsCertSecret.secretArn,
      description:
        'Secrets Manager ARN holding the prices-api mTLS client cert PEM',
    });

    new ssm.StringParameter(this, 'MtlsKeySecretArnParam', {
      parameterName: `/prices/${envName}/mtls-key-secret-arn`,
      stringValue: this.mtlsKeySecret.secretArn,
      description:
        'Secrets Manager ARN holding the prices-api mTLS client key PEM',
    });

    new cdk.CfnOutput(this, 'MtlsCertSecretArn', {
      value: this.mtlsCertSecret.secretArn,
      description: `mTLS cert Secrets Manager ARN for ${envName}`,
    });
    new cdk.CfnOutput(this, 'MtlsKeySecretArn', {
      value: this.mtlsKeySecret.secretArn,
      description: `mTLS key Secrets Manager ARN for ${envName}`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', envName);
  }
}
