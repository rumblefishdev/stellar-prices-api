import * as cdk from 'aws-cdk-lib';
import type * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import type * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';
import {
  PRICES_LAMBDA_LOG_REMOVAL_POLICY,
  PRICES_LAMBDA_LOG_RETENTION,
  createPricesLambdaRole,
  lambdaLogGroupName,
} from '../lambda-baseline.js';

export interface ComputeStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  readonly mtlsCertSecret: secretsmanager.ISecret;
  readonly mtlsKeySecret: secretsmanager.ISecret;
}

/**
 * Compute layer for prices-api: per-Lambda IAM roles + LogGroups
 * for the two anchor Lambdas, with no actual Function definitions
 * yet. Downstream tasks attach `RustFunction` constructs to these
 * roles + log groups:
 *
 * - `ledgerProcessorRole` / `ledgerProcessorLogGroup` — consumed by
 *   task 0038 (live S3-event-driven ingest Lambda).
 * - `apiHandlerRole` / `apiHandlerLogGroup` — consumed by task
 *   0040 (axum REST handlers behind API Gateway).
 *
 * The four periodic-worker roles (task 0039: price updater, oracle
 * watcher, asset discovery, cleanup) and the backfill-status role
 * (task 0055) are NOT pre-created here — those Lambdas are
 * closely coupled to the EventBridge Scheduler rules (0039) and
 * API Gateway routes (0055) defined alongside them. Each of those
 * tasks calls `createPricesLambdaRole` from `lib/lambda-baseline.ts`
 * to construct a baseline role and then extends it with
 * stack-specific permissions.
 *
 * No VPC. Per ADR 0007 §3.6, Lambdas reach the Hetzner Caddy
 * address over the public internet; gating is mTLS at Caddy.
 */
export class ComputeStack extends cdk.Stack {
  public readonly ledgerProcessorRole: iam.Role;
  public readonly ledgerProcessorLogGroup: logs.LogGroup;
  public readonly apiHandlerRole: iam.Role;
  public readonly apiHandlerLogGroup: logs.LogGroup;

  constructor(scope: Construct, id: string, props: ComputeStackProps) {
    super(scope, id, props);

    const { config, mtlsCertSecret, mtlsKeySecret } = props;
    const accountId = cdk.Stack.of(this).account;
    const ctx = { config, accountId, mtlsCertSecret, mtlsKeySecret };

    this.ledgerProcessorRole = createPricesLambdaRole(
      this,
      'LedgerProcessorRole',
      ctx,
    );
    this.ledgerProcessorLogGroup = new logs.LogGroup(
      this,
      'LedgerProcessorLogGroup',
      {
        logGroupName: lambdaLogGroupName(config.envName, 'ledger-processor'),
        retention: PRICES_LAMBDA_LOG_RETENTION,
        removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
      },
    );

    this.apiHandlerRole = createPricesLambdaRole(this, 'ApiHandlerRole', ctx);
    this.apiHandlerLogGroup = new logs.LogGroup(this, 'ApiHandlerLogGroup', {
      logGroupName: lambdaLogGroupName(config.envName, 'api-handler'),
      retention: PRICES_LAMBDA_LOG_RETENTION,
      removalPolicy: PRICES_LAMBDA_LOG_REMOVAL_POLICY,
    });

    new cdk.CfnOutput(this, 'LedgerProcessorRoleArn', {
      value: this.ledgerProcessorRole.roleArn,
      description: `Ledger Processor Lambda execution role ARN (${config.envName})`,
    });
    new cdk.CfnOutput(this, 'ApiHandlerRoleArn', {
      value: this.apiHandlerRole.roleArn,
      description: `API Handler Lambda execution role ARN (${config.envName})`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', config.envName);
  }
}
