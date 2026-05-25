import * as cdk from 'aws-cdk-lib';
import * as apigateway from 'aws-cdk-lib/aws-apigateway';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface ApiGatewayStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * Public REST API Gateway shell.
 *
 * This is a skeleton: it stands up the `RestApi` itself, the
 * environment-specific throttling on the stage, and the UsagePlan
 * + ApiKey for non-browser consumers — but no real routes. The
 * only method is a `GET /health` mock integration returning
 * `{ "status": "ok", "stack": "prices-{env}" }`, which:
 *
 * 1. Makes the stage deployable (API Gateway refuses to deploy a
 *    stage with zero methods).
 * 2. Provides a synthetic probe target for downstream alarms
 *    (task 0056) without needing a real Lambda.
 *
 * Task 0040 will attach the `/v1/prices/...` routes as Lambda
 * proxy integrations onto the `ComputeStack.apiHandlerRole`-backed
 * function. The `/health` route stays — it's still useful as the
 * cheapest "is the stage alive" check.
 *
 * Deferred to 0040 / 0056:
 * - Custom domain wiring (Route 53 A-record + ACM cert).
 * - WAF WebACL (REGIONAL-scoped) for IP rate-limiting and managed
 *   rule sets.
 * - CORS preflight (waits for browser-consumer requirements).
 * - Response caching (no real traffic shape to size it against
 *   yet).
 *
 * The REST API ID is published to SSM at `/prices/{env}/api-gateway-id`
 * so downstream consumers (custom domain wiring, CloudWatch dashboards)
 * can resolve it without cross-stack imports.
 */
export class ApiGatewayStack extends cdk.Stack {
  public readonly api: apigateway.RestApi;

  constructor(scope: Construct, id: string, props: ApiGatewayStackProps) {
    super(scope, id, props);

    const { config } = props;

    this.api = new apigateway.RestApi(this, 'Api', {
      restApiName: `prices-${config.envName}-api`,
      description: `prices-api public REST API (${config.envName})`,
      deployOptions: {
        stageName: config.envName,
        tracingEnabled: true,
        throttlingRateLimit: config.apiGatewayThrottleRate,
        throttlingBurstLimit: config.apiGatewayThrottleBurst,
      },
      endpointTypes: [apigateway.EndpointType.REGIONAL],
    });

    // Skeleton GET /health route — mock integration returns a static
    // 200 with the env name. Replace / supplement with real routes
    // in task 0040.
    const health = this.api.root.addResource('health');
    health.addMethod(
      'GET',
      new apigateway.MockIntegration({
        integrationResponses: [
          {
            statusCode: '200',
            responseTemplates: {
              'application/json': JSON.stringify({
                status: 'ok',
                stack: `prices-${config.envName}`,
              }),
            },
          },
        ],
        passthroughBehavior: apigateway.PassthroughBehavior.NEVER,
        requestTemplates: {
          'application/json': '{ "statusCode": 200 }',
        },
      }),
      {
        methodResponses: [{ statusCode: '200' }],
      },
    );

    // Usage plan + API key. In skeleton mode no methods require an
    // API key — the plan attaches to the stage so requests sent
    // with `x-api-key` are tracked and throttled, but the key is
    // not yet a gate. Task 0040 marks specific routes with
    // `apiKeyRequired: true` when partner endpoints arrive.
    const usagePlan = this.api.addUsagePlan('UsagePlan', {
      name: `prices-${config.envName}-partner-plan`,
      throttle: {
        rateLimit: config.apiGatewayThrottleRate,
        burstLimit: config.apiGatewayThrottleBurst,
      },
      quota: {
        limit: config.apiGatewayPartnerDailyQuota,
        period: apigateway.Period.DAY,
      },
    });
    usagePlan.addApiStage({ stage: this.api.deploymentStage });

    const apiKey = this.api.addApiKey('PartnerApiKey', {
      apiKeyName: `prices-${config.envName}-partner-key`,
    });
    usagePlan.addApiKey(apiKey);

    new ssm.StringParameter(this, 'ApiGatewayIdParam', {
      parameterName: `/prices/${config.envName}/api-gateway-id`,
      stringValue: this.api.restApiId,
      description: `REST API ID for prices-${config.envName}-api`,
    });

    new cdk.CfnOutput(this, 'ApiUrl', {
      value: this.api.url,
      description: `Invoke URL for prices-${config.envName}-api stage`,
    });
    new cdk.CfnOutput(this, 'ApiKeyId', {
      value: apiKey.keyId,
      description: `API key ID — retrieve secret value via 'aws apigateway get-api-key --include-value'`,
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', config.envName);
  }
}
