import * as cdk from 'aws-cdk-lib';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as logs from 'aws-cdk-lib/aws-logs';
import type * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from './types.js';

export interface BaselineLambdaContext {
  readonly config: EnvironmentConfig;
  readonly accountId: string;
  readonly mtlsCertSecret: secretsmanager.ISecret;
  readonly mtlsKeySecret: secretsmanager.ISecret;
}

/**
 * The IAM permissions every prices-api Lambda needs to function
 * against the post-ADR-0007 architecture:
 *
 * 1. CloudWatch Logs write — via AWSLambdaBasicExecutionRole managed
 *    policy (attached separately at role construction time).
 * 2. Read the two mTLS material secrets from Secrets Manager.
 * 3. Read both SSM namespaces — /platform/{env}/* (BE-published) and
 *    /prices/{env}/* (prices-api-published). Read-only here; the
 *    deploy role (CicdStack) is the only principal that writes.
 *
 * Downstream Lambdas extend these via `role.addToPolicy(...)` for
 * stack-specific needs (e.g. the ledger processor adds S3 read on
 * the BE-owned bucket; the API handler adds nothing extra).
 */
export function baselineLambdaPolicyStatements(
  ctx: BaselineLambdaContext,
): iam.PolicyStatement[] {
  const { config, accountId } = ctx;
  const region = config.awsRegion;
  const envName = config.envName;

  return [
    new iam.PolicyStatement({
      sid: 'ReadMtlsMaterial',
      actions: ['secretsmanager:GetSecretValue'],
      resources: [ctx.mtlsCertSecret.secretArn, ctx.mtlsKeySecret.secretArn],
    }),
    new iam.PolicyStatement({
      sid: 'ReadSsmNamespaces',
      actions: [
        'ssm:GetParameter',
        'ssm:GetParameters',
        'ssm:GetParametersByPath',
      ],
      resources: [
        `arn:aws:ssm:${region}:${accountId}:parameter/platform/${envName}/*`,
        `arn:aws:ssm:${region}:${accountId}:parameter/prices/${envName}/*`,
      ],
    }),
  ];
}

/**
 * Canonical log-group name for a prices-api Lambda.
 *
 * Format: `/aws/lambda/prices-{env}-{lambdaName}` — matches BE's
 * naming convention (`/aws/lambda/{env}-soroban-explorer-{name}`)
 * but scoped to the `prices-` prefix so the two services never
 * collide in a shared CloudWatch view.
 */
export function lambdaLogGroupName(
  envName: string,
  lambdaName: string,
): string {
  return `/aws/lambda/prices-${envName}-${lambdaName}`;
}

/**
 * Creates an IAM role for a prices-api Lambda with the baseline
 * permissions applied (CloudWatch Logs + mTLS secrets + SSM read).
 *
 * Downstream tasks call `role.addToPolicy(...)` to attach
 * stack-specific permissions.
 */
export function createPricesLambdaRole(
  scope: Construct,
  id: string,
  ctx: BaselineLambdaContext,
): iam.Role {
  const role = new iam.Role(scope, id, {
    assumedBy: new iam.ServicePrincipal('lambda.amazonaws.com'),
    description: `Execution role for a prices-api Lambda (${ctx.config.envName}).`,
    managedPolicies: [
      iam.ManagedPolicy.fromAwsManagedPolicyName(
        'service-role/AWSLambdaBasicExecutionRole',
      ),
    ],
  });

  for (const statement of baselineLambdaPolicyStatements(ctx)) {
    role.addToPolicy(statement);
  }

  return role;
}

/**
 * Default runtime/architecture for prices-api Rust Lambdas, per
 * ADR 0006 (axum + cargo-lambda) and ADR 0007 (no VPC). Spread
 * this object into every `RustFunction` / `Function` props block
 * so the conventions stay aligned across stacks.
 *
 * Notably:
 * - Architecture is ARM_64 (Graviton) — ~10-20% cheaper than x86.
 * - Runtime is PROVIDED_AL2023 — the custom runtime that cargo-lambda
 *   bootstrap binaries target.
 * - No `vpc` / `vpcSubnets` / `securityGroups` set anywhere; ADR 0007
 *   §3.6 forbids VPC attachment.
 */
export const pricesLambdaDefaults = {
  architecture: lambda.Architecture.ARM_64,
  runtime: lambda.Runtime.PROVIDED_AL2023,
} as const;

/**
 * Default retention for prices-api Lambda log groups. ONE_MONTH
 * matches BE's convention and balances debugging surface against
 * CloudWatch storage cost.
 */
export const PRICES_LAMBDA_LOG_RETENTION = logs.RetentionDays.ONE_MONTH;

/**
 * Default removal policy for prices-api Lambda log groups. DESTROY
 * means re-deploying the stack deletes the historical log records;
 * acceptable because logs over 30 days old aren't load-bearing.
 * Per-env override can be added if production needs RETAIN.
 */
export const PRICES_LAMBDA_LOG_REMOVAL_POLICY = cdk.RemovalPolicy.DESTROY;
