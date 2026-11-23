import * as cdk from 'aws-cdk-lib';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

/**
 * Condition key prefix — AWS IAM strips the https:// from the issuer URL. It
 * is also the provider's name in its ARN, see `oidcProviderArn` below.
 */
const GITHUB_OIDC_ISSUER = 'token.actions.githubusercontent.com';
const GITHUB_OIDC_AUDIENCE = 'sts.amazonaws.com';

export interface CicdStackProps extends cdk.StackProps {
  /** GitHub org/repo, e.g. "rumblefishdev/stellar-prices-api". */
  readonly githubRepo: string;
  readonly awsRegion: string;
}

/**
 * CI/CD resources for the production AWS environment.
 *
 * Both soroban-block-explorer and stellar-prices-api share a single
 * AWS account. This stack provisions the prices-api-specific OIDC
 * plumbing (the BE equivalent lives in the block-explorer repo).
 *
 * Creates:
 * - Production deploy role (scoped to GitHub Environment "production"),
 *   assumed by `.github/workflows/deploy-production.yml`
 *
 * Imports, never creates, the GitHub Actions OIDC identity provider. An
 * account holds one provider per URL, and this account's belongs to
 * soroban-block-explorer's `Explorer-Cicd` stack (measured 2026-11-20, task
 * 0343). Importing by ARN changes nothing there, but deleting that stack or
 * removing the provider from its code deletes the provider and stops this
 * role from being assumed. A re-created provider has the same ARN, so this
 * stack then works again unchanged. Moving the provider to
 * soroban-infrastructure is task 0351.
 *
 * The deploy role trusts the CDK bootstrap roles for actual CloudFormation
 * operations. The OIDC trust policy restricts which GitHub workflows can
 * assume the role based on the GitHub Environment name.
 *
 * Staging deploy role removed — staging was retired during the
 * eu-central-1 cutover (mirrors BE task 0239). The single
 * "production" environment is initially deployed with conservative
 * test-sized parameters; values are swapped to true production
 * sizing once the service is exercised in anger.
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
 * - apigateway:DELETE on the production stage's cache data only (the
 *   post-deploy cache flush)
 *
 * No ECR (no Docker images), no S3 SPA (no frontend), no CloudFront
 * (no public CDN) — those scopes belong to soroban-block-explorer.
 */
export class CicdStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: CicdStackProps) {
    super(scope, id, props);

    const { githubRepo, awsRegion } = props;
    const accountId = cdk.Stack.of(this).account;

    // The provider's audience is `sts.amazonaws.com`, the one the trust
    // policy below requires (read back 2026-11-20).
    const oidcProvider = iam.OpenIdConnectProvider.fromOpenIdConnectProviderArn(
      this,
      'GitHubOidcProvider',
      `arn:aws:iam::${accountId}:oidc-provider/${GITHUB_OIDC_ISSUER}`,
    );

    const envName = 'production' as const;
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

    // `make flush-production-cache` after the deploy (task 0124): drops the
    // stage's cached GET responses so `/api-docs-json`, cached for an hour,
    // is the new deployment's at once. Only the cache data of this API's
    // stage: not the stage, not the API, not the authorizer cache. The API id
    // is the one ApiGateway publishes; a replaced RestApi needs
    // `make deploy-cicd` again, and until then the flush step fails loudly
    // and `master` does not move (task 0343, decision 10).
    const restApiId = ssm.StringParameter.valueForStringParameter(
      this,
      `/prices/${envName}/api-gateway-id`,
    );
    role.addToPolicy(
      new iam.PolicyStatement({
        actions: ['apigateway:DELETE'],
        resources: [
          `arn:aws:apigateway:${awsRegion}::/restapis/${restApiId}/stages/${envName}/cache/data`,
        ],
      }),
    );

    new cdk.CfnOutput(this, `${capitalize(envName)}DeployRoleArn`, {
      value: role.roleArn,
      description: `Deploy role ARN for ${envName} — add as AWS_DEPLOY_ROLE_ARN in GitHub Environment "${envName}"`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
  }
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}
