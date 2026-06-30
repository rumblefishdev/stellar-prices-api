import * as cdk from 'aws-cdk-lib';
import * as apigateway from 'aws-cdk-lib/aws-apigateway';
import type * as lambda from 'aws-cdk-lib/aws-lambda';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface ApiGatewayStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  /**
   * The single axum api-handler Lambda (ADR 0008) every `/v1` route proxies to.
   * Passed in from `ComputeStack` (cross-stack reference).
   */
  readonly apiHandlerFunction: lambda.IFunction;
}

/**
 * Per-endpoint response-cache TTLs (overview §2.1). The cache key includes the
 * path params (automatic) plus the query params declared per method below, so
 * paginated / parameterized reads cache correctly. `POST /prices/batch` and
 * `GET /health` are uncached.
 */
const CACHE_TTL = {
  assetsList: cdk.Duration.seconds(60),
  assetDetail: cdk.Duration.seconds(60),
  price: cdk.Duration.seconds(15),
  ohlcv: cdk.Duration.seconds(60),
  oracles: cdk.Duration.seconds(30),
  backfill: cdk.Duration.seconds(30),
} as const;

/** 0.5 GB stage cache (overview §2.1). */
const CACHE_CLUSTER_SIZE = '0.5';

/**
 * Public REST API Gateway for prices-api.
 *
 * Fronts the single axum api-handler Lambda (ADR 0008): every `/v1` route is a
 * Lambda **proxy** integration onto `ComputeStack.apiHandlerFunction`, so the
 * gateway forwards the full request (path + query + headers) and the Lambda's
 * own axum router (which owns the `/v1` prefix) handles it — the gateway adds no
 * prefix of its own, so there is no `/v1/v1` double-prefix.
 *
 * - **Auth / rate limit**: data routes set `apiKeyRequired: true`; the UsagePlan
 *   enforces the §2.1/§7 per-key 100 req/s (`apiKeyRateLimit`) + a daily quota.
 *   `GET /health` stays a keyless mock (cheapest liveness probe).
 * - **Response cache**: 0.5 GB stage cache with per-endpoint TTLs (`CACHE_TTL`);
 *   each cached method declares its query params as cache keys.
 *
 * Still deferred (task 0056 / later): custom domain + ACM, WAF WebACL, CORS
 * preflight. The REST API ID is published to SSM at
 * `/prices/{env}/api-gateway-id`.
 */
export class ApiGatewayStack extends cdk.Stack {
  public readonly api: apigateway.RestApi;

  constructor(scope: Construct, id: string, props: ApiGatewayStackProps) {
    super(scope, id, props);

    const { config, apiHandlerFunction } = props;
    const cacheEnabled = config.apiGatewayCacheEnabled;

    this.api = new apigateway.RestApi(this, 'Api', {
      restApiName: `prices-${config.envName}-api`,
      description: `prices-api public REST API (${config.envName})`,
      deployOptions: {
        stageName: config.envName,
        tracingEnabled: true,
        throttlingRateLimit: config.apiGatewayThrottleRate,
        throttlingBurstLimit: config.apiGatewayThrottleBurst,
        // 0.5 GB stage response cache; per-method TTLs set on each method below.
        cachingEnabled: cacheEnabled,
        ...(cacheEnabled
          ? { cacheClusterEnabled: true, cacheClusterSize: CACHE_CLUSTER_SIZE }
          : {}),
      },
      endpointTypes: [apigateway.EndpointType.REGIONAL],
    });

    // ---------------------------------------------------------------
    // GET /health — keyless mock (liveness; no Lambda invocation).
    // ---------------------------------------------------------------
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
        requestTemplates: { 'application/json': '{ "statusCode": 200 }' },
      }),
      {
        methodResponses: [{ statusCode: '200' }],
        // health stays uncached even when the stage cache is on.
        ...(cacheEnabled ? { cachingEnabled: false } : {}),
      },
    );

    // ---------------------------------------------------------------
    // /v1/* — Lambda proxy routes to the single api-handler.
    // ---------------------------------------------------------------
    /** Lambda proxy integration with the given gateway cache-key params. */
    const proxy = (cacheKeyParameters: string[]) =>
      new apigateway.LambdaIntegration(apiHandlerFunction, {
        proxy: true,
        ...(cacheKeyParameters.length ? { cacheKeyParameters } : {}),
      });
    /** Declare cache-key params on the method (path → required, query → optional). */
    const declare = (keys: string[]): Record<string, boolean> =>
      Object.fromEntries(keys.map((k) => [k, k.includes('.path.')]));
    /** Add a key-gated GET with a cached integration. */
    const addGet = (resource: apigateway.IResource, cacheKeys: string[]) =>
      resource.addMethod('GET', proxy(cacheKeys), {
        apiKeyRequired: true,
        requestParameters: declare(cacheKeys),
      });

    const PATH_ID = 'method.request.path.asset_identifier';
    const qs = (name: string) => `method.request.querystring.${name}`;

    const v1 = this.api.root.addResource('v1');

    // /v1/assets (list) + /v1/assets/{asset_identifier} (+ /price, /ohlcv)
    const assets = v1.addResource('assets');
    addGet(assets, [
      qs('type'),
      qs('search'),
      qs('sort'),
      qs('order'),
      qs('cursor'),
      qs('limit'),
    ]);
    const assetId = assets.addResource('{asset_identifier}');
    addGet(assetId, [PATH_ID]);
    addGet(assetId.addResource('price'), [PATH_ID]);
    addGet(assetId.addResource('ohlcv'), [
      PATH_ID,
      qs('timeframe'),
      qs('granularity'),
      qs('start'),
      qs('end'),
      qs('base_currency'),
    ]);

    // /v1/oracles/{asset_identifier}
    const oracles = v1.addResource('oracles');
    addGet(oracles.addResource('{asset_identifier}'), [PATH_ID]);

    // /v1/backfill/status
    const backfill = v1.addResource('backfill');
    addGet(backfill.addResource('status'), []);

    // /v1/prices/batch (POST, uncached)
    const prices = v1.addResource('prices');
    prices.addResource('batch').addMethod('POST', proxy([]), {
      apiKeyRequired: true,
    });

    // ---------------------------------------------------------------
    // Per-method cache TTLs (only meaningful when the stage cache is on).
    // ---------------------------------------------------------------
    if (cacheEnabled) {
      // Per-method caching expressed as CfnStage method settings (resourcePath +
      // httpMethod). The high-level `deployOptions.methodOptions` is fixed at
      // construction; setting the L1 `methodSettings` here keeps the per-route
      // TTL table colocated with the routes for readability.
      const cfnStage = this.api.deploymentStage.node
        .defaultChild as apigateway.CfnStage;
      // Assigning `methodSettings` wholesale REPLACES the `/*/*` entry CDK
      // renders from `deployOptions.throttlingRateLimit/Burst`, which would drop
      // the stage-wide throttle (only the per-key usage-plan limit would remain).
      // Re-declare it here so the §2.1 aggregate stage ceiling survives.
      cfnStage.methodSettings = [
        {
          resourcePath: '/*',
          httpMethod: '*',
          throttlingRateLimit: config.apiGatewayThrottleRate,
          throttlingBurstLimit: config.apiGatewayThrottleBurst,
        },
        {
          resourcePath: '/v1/assets',
          httpMethod: 'GET',
          cachingEnabled: true,
          cacheTtlInSeconds: CACHE_TTL.assetsList.toSeconds(),
        },
        {
          resourcePath: '/v1/assets/{asset_identifier}',
          httpMethod: 'GET',
          cachingEnabled: true,
          cacheTtlInSeconds: CACHE_TTL.assetDetail.toSeconds(),
        },
        {
          resourcePath: '/v1/assets/{asset_identifier}/price',
          httpMethod: 'GET',
          cachingEnabled: true,
          cacheTtlInSeconds: CACHE_TTL.price.toSeconds(),
        },
        {
          resourcePath: '/v1/assets/{asset_identifier}/ohlcv',
          httpMethod: 'GET',
          cachingEnabled: true,
          cacheTtlInSeconds: CACHE_TTL.ohlcv.toSeconds(),
        },
        {
          resourcePath: '/v1/oracles/{asset_identifier}',
          httpMethod: 'GET',
          cachingEnabled: true,
          cacheTtlInSeconds: CACHE_TTL.oracles.toSeconds(),
        },
        {
          resourcePath: '/v1/backfill/status',
          httpMethod: 'GET',
          cachingEnabled: true,
          cacheTtlInSeconds: CACHE_TTL.backfill.toSeconds(),
        },
        // Explicitly uncached:
        {
          resourcePath: '/v1/prices/batch',
          httpMethod: 'POST',
          cachingEnabled: false,
        },
        { resourcePath: '/health', httpMethod: 'GET', cachingEnabled: false },
      ];
    }

    // ---------------------------------------------------------------
    // UsagePlan + API key — per-key 100 req/s (§2.1/§7) + daily quota.
    // ---------------------------------------------------------------
    const usagePlan = this.api.addUsagePlan('UsagePlan', {
      name: `prices-${config.envName}-partner-plan`,
      throttle: {
        rateLimit: config.apiKeyRateLimit,
        burstLimit: config.apiKeyBurstLimit,
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
