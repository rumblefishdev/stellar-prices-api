import * as cdk from 'aws-cdk-lib';
import * as iam from 'aws-cdk-lib/aws-iam';
import type { Construct } from 'constructs';

const GITHUB_OIDC_THUMBPRINT = 'ffffffffffffffffffffffffffffffffffffffff';
const GITHUB_OIDC_URL = 'https://token.actions.githubusercontent.com';
/** Condition key prefix — AWS IAM strips the https:// from the issuer URL. */
const GITHUB_OIDC_ISSUER = 'token.actions.githubusercontent.com';
const GITHUB_OIDC_AUDIENCE = 'sts.amazonaws.com';

export interface CicdStackProps extends cdk.StackProps {
  /** GitHub org/repo, e.g. "rumblefishdev/stellar-prices-api". */
  readonly githubRepo: string;
  readonly awsRegion: string;
}

/**
 * CI/CD resources shared across environments.
 *
 * Creates:
 * - GitHub Actions OIDC identity provider (singleton per AWS account)
 * - Staging deploy role (scoped to GitHub Environment "staging")
 * - Production deploy role (scoped to GitHub Environment "production")
 *
 * Deploy roles trust the CDK bootstrap roles for actual CloudFormation
 * operations. The OIDC trust policy restricts which GitHub workflows
 * can assume each role based on the GitHub Environment name.
 *
 * Deployed once per AWS account via: `make deploy-cicd`
 *
 * Permission scope is intentionally narrow for the prices-api shape:
 * - sts:AssumeRole on cdk-hnb659fds-* bootstrap roles (CDK does the
 *   actual CloudFormation work)
 * - ssm:GetParameter on /platform/{env}/* (BE-published identifiers)
 *   and /prices/{env}/* (prices-api-published outputs)
 * - ssm:PutParameter on /prices/{env}/* (writeable outputs only)
 * - cloudformation:DescribeStacks for post-deploy smoke tests
 *
 * No ECR (no Docker images), no S3 SPA (no frontend), no CloudFront
 * (no public CDN) — those scopes belong to soroban-block-explorer.
 */
export class CicdStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: CicdStackProps) {
    super(scope, id, props);

    const { githubRepo, awsRegion } = props;
    const accountId = cdk.Stack.of(this).account;

    const oidcProvider = new iam.OpenIdConnectProvider(
      this,
      'GitHubOidcProvider',
      {
        url: GITHUB_OIDC_URL,
        clientIds: [GITHUB_OIDC_AUDIENCE],
        thumbprints: [GITHUB_OIDC_THUMBPRINT],
      },
    );

    for (const envName of ['staging', 'production'] as const) {
      const role = new iam.Role(this, `${capitalize(envName)}DeployRole`, {
        roleName: `stellar-prices-api-${envName}-deploy`,
        assumedBy: new iam.WebIdentityPrincipal(
          oidcProvider.openIdConnectProviderArn,
          {
            StringEquals: {
              [`${GITHUB_OIDC_ISSUER}:aud`]: GITHUB_OIDC_AUDIENCE,
              [`${GITHUB_OIDC_ISSUER}:sub`]: `repo:${githubRepo}:environment:${envName}`,
            },
          },
        ),
        maxSessionDuration: cdk.Duration.hours(1),
        description: `GitHub Actions deploy role for ${envName} environment`,
      });

      role.addToPolicy(
        new iam.PolicyStatement({
          actions: ['sts:AssumeRole'],
          resources: [
            `arn:aws:iam::${accountId}:role/cdk-hnb659fds-*-${accountId}-${awsRegion}`,
          ],
        }),
      );

      // Read both namespaces — /platform/* is BE-published, /prices/*
      // is prices-api-published. Both are needed at synth time so
      // valueForStringParameter calls can resolve during diff/deploy.
      role.addToPolicy(
        new iam.PolicyStatement({
          actions: ['ssm:GetParameter', 'ssm:GetParameters'],
          resources: [
            `arn:aws:ssm:${awsRegion}:${accountId}:parameter/platform/${envName}/*`,
            `arn:aws:ssm:${awsRegion}:${accountId}:parameter/prices/${envName}/*`,
          ],
        }),
      );

      // Write only under /prices/* — never /platform/*. The two-
      // namespace contract is the cross-team boundary; violating it
      // here would let prices-api silently overwrite BE values.
      role.addToPolicy(
        new iam.PolicyStatement({
          actions: ['ssm:PutParameter', 'ssm:DeleteParameter'],
          resources: [
            `arn:aws:ssm:${awsRegion}:${accountId}:parameter/prices/${envName}/*`,
          ],
        }),
      );

      role.addToPolicy(
        new iam.PolicyStatement({
          actions: ['cloudformation:DescribeStacks'],
          resources: [
            `arn:aws:cloudformation:${awsRegion}:${accountId}:stack/Prices-${envName}-*/*`,
          ],
        }),
      );

      new cdk.CfnOutput(this, `${capitalize(envName)}DeployRoleArn`, {
        value: role.roleArn,
        description: `Deploy role ARN for ${envName} — add as AWS_DEPLOY_ROLE_ARN in GitHub Environment "${envName}"`,
      });
    }

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
  }
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}
