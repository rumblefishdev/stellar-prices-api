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
 * paginated / parameterized reads cache correctly. Caching is opt-IN: anything
 * without an entry in `methodSettings` below is uncached by the `/*` `*`
 * default, which covers `POST /prices/batch`, `GET /health` and the portal's
 * `ANY /api-tokens/api/{proxy+}`.
 *
 * SINGLE SOURCE OF TRUTH: these MUST mirror the handler `Cache-Control` tiers in
 * `packages/prices-api/src/common/cache_control.rs` (the gateway stage cache and
 * the client/CDN max-age must agree, or one serves staler data than the other).
 * Mapping: SHORT=10s → price; MEDIUM=60s → assetsList / assetDetail / ohlcv /
 * oracles / backfill. `apiDocs` is the one deliberate mismatch: 3600s here,
 * `DEPLOY_STATIC`=300s on the handler — see the comment on that constant and on
 * `apiDocs` below.
 */
const CACHE_TTL = {
  assetsList: cdk.Duration.seconds(60), // MEDIUM
  assetDetail: cdk.Duration.seconds(60), // MEDIUM
  price: cdk.Duration.seconds(10), // SHORT
  ohlcv: cdk.Duration.seconds(60), // MEDIUM
  oracles: cdk.Duration.seconds(60), // MEDIUM
  backfill: cdk.Duration.seconds(60), // MEDIUM
  // The spec is byte-identical for the life of a deployment, so the longest TTL
  // API Gateway allows keeps the document off the Lambda entirely (task 0124).
  //
  // It is only free because this entry is flushed when a deployment ships —
  // `make -C infra deploy-production` runs `flush-production-cache` after the
  // deploy. Without that the cache outlives the build that filled it and the
  // gateway serves the previous deployment's document for up to an hour. The
  // handler's own `Cache-Control` is 300s, not 3600s, for the same reason
  // applied to caches we do NOT control (see cache_control.rs).
  apiDocs: cdk.Duration.seconds(3600), // DEPLOY_STATIC (client side: 300s)
} as const;

/** 0.5 GB stage cache (overview §2.1). */
const CACHE_CLUSTER_SIZE = '0.5';

/**
 * Method-level throttle for `GET /api-docs-json`.
 *
 * Every other Lambda-backed route is `apiKeyRequired: true` and therefore
 * carries two limits from the usage plan: the per-key rate and the monthly quota.
 * This one is anonymous by design, so it has neither — without an entry here it
 * would fall back to the default method entry (`resourcePath: '/*'`), which is
 * `apiGatewayThrottleRate` (200 req/s). That is a lot of unauthenticated traffic
 * to leave available on a route nobody has to hold a key to call.
 *
 * What this buys is COST CONTROL ON THIS ROUTE, not protection of the others.
 * Per-API per-stage limits are applied per method — each method gets its own
 * bucket from that default — so an anonymous loop here cannot draw down
 * `/v1/...` and cannot make a key holder see 429s on a route they never called.
 * It can only exhaust its own bucket. The exposure is the bill: API Gateway
 * charges per request, and with the cache off every one is also a billed Lambda
 * invocation.
 *
 * Sized for what the route is: a static ~40 KB document that a reader fetches
 * occasionally and that is cached for an hour at the edge. 10 req/s is far above
 * any legitimate use and a twentieth of what the route would otherwise get.
 *
 * A local constant rather than a config key because it is a property of this
 * route's shape (anonymous, cached, static), not of an environment — unlike
 * `apiGatewayThrottleRate`, which encodes a per-deployment capacity decision.
 */
const API_DOCS_THROTTLE = { rate: 10, burst: 20 } as const;

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
 *   enforces the self-service per-key rate (`pricingApiFreePlanRateLimit`) + a
 *   monthly quota (task 0157, overriding the design doc's §2.1/§7 100 req/s).
 *   `GET /health` stays a keyless mock (cheapest liveness probe), and
 *   `GET /api-docs-json` is a keyless proxy to the handler (task 0124 — public
 *   documentation). `ANY /api-tokens/api/{proxy+}` is the onboarding portal's
 *   backend (task 0184), keyless for the same reason and gated in the handler
 *   by `PORTAL_ENABLED` (task 0183).
 * - **Response cache**: 0.5 GB stage cache with per-endpoint TTLs (`CACHE_TTL`),
 *   opt-in per method; each cached method declares its query params as cache
 *   keys.
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

    // ---------------------------------------------------------------
    // GET /api-docs-json — the OpenAPI spec, anonymous (task 0124).
    // ---------------------------------------------------------------
    // Proxies to the same axum handler as the data routes (one integration
    // mechanism, one source of truth) rather than serving a second, separately
    // maintained copy of the document.
    //
    // Keyless on purpose: an API description is public documentation, and
    // gating it behind a key the reader does not have yet is a self-service
    // dead end. `/health` already establishes the anonymous-route precedent.
    // The in-app gate exempts this path too (`auth::is_exempt`), so the posture
    // holds even when `API_KEYS` is armed. Safe to cache for everyone because
    // the document contains nothing key-specific.
    //
    // The `/health` precedent covers the *posture*, not the cost profile:
    // `/health` is a MockIntegration and can never invoke anything, whereas a
    // cache miss here reaches the Lambda, and an anonymous route sits outside
    // the usage plan, so the only limiter it has is the stage-wide throttle it
    // shares with paying traffic. Two things keep the residual small: the
    // 3600s TTL below with **no** cache-key parameters, so all callers collapse
    // onto one entry, and API Gateway's default
    // `requireAuthorizationForCacheControl: true`, which stops an anonymous
    // caller busting that entry with `Cache-Control: max-age=0`. If this ever
    // needs a harder bound, the lever is a method-level throttle here, not a
    // key requirement.
    this.api.root.addResource('api-docs-json').addMethod('GET', proxy([]), {
      apiKeyRequired: false,
    });

    // ---------------------------------------------------------------
    // ANY /api-tokens/api/{proxy+} — the onboarding portal's backend (0184).
    // ---------------------------------------------------------------
    // Without this resource the portal's routes are unreachable in production
    // no matter what the handler does: CloudFront forwards the request, the
    // gateway maps nothing, and the caller gets the gateway's own
    // `403 Missing Authentication Token` instead of the empty `404` task 0183's
    // gate is careful to produce. This is the "door" that task's note says
    // arrives here.
    //
    // **Greedy, and ANY.** The point of task 0183's prefix gate is that a later
    // slice adds a route without editing the gate; the same has to hold at the
    // gateway, or every slice pays for a CDK change and a deploy. `{proxy+}`
    // plus `ANY` covers task 0186's `GET /auth/*`, task 0187's `POST /key` and
    // task 0192's `DELETE /key` with no further work here. The axum router
    // decides what actually exists — and while `PORTAL_ENABLED` is false, that
    // answer is "nothing", byte-identical to a path that was never deployed.
    //
    // **Keyless**, matching `auth::is_exempt`: a visitor signing in to get a
    // key does not have one yet, so requiring one is a self-service dead end —
    // the same argument that makes `/api-docs-json` anonymous. This is not a
    // hole: the flag decides whether these routes answer at all, and once they
    // do, task 0186's session is what authenticates them. The route stays
    // outside the usage plan, so its only limiter is the default per-method
    // stage throttle.
    //
    // **Uncached**, but not by an entry of its own — a greedy `{proxy+}` cannot
    // carry one. See `defaultCachingOff` below, which is what actually holds
    // that guarantee.
    const portalApi = this.api.root
      .addResource('api-tokens')
      .addResource('api');
    portalApi.addResource('{proxy+}').addMethod('ANY', proxy([]), {
      apiKeyRequired: false,
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
    // Per-method stage settings: throttles (always) + cache TTLs (when the
    // stage cache is on).
    // ---------------------------------------------------------------
    // Per-method settings expressed as CfnStage method settings (resourcePath +
    // httpMethod). The high-level `deployOptions.methodOptions` is fixed at
    // construction; setting the L1 `methodSettings` here keeps the per-route
    // table colocated with the routes for readability.
    const cfnStage = this.api.deploymentStage.node
      .defaultChild as apigateway.CfnStage;

    // Assigning `methodSettings` wholesale REPLACES the default entry CDK
    // renders from `deployOptions.throttlingRateLimit/Burst`, which would drop
    // it entirely — leaving the per-key usage-plan limit, and above it the
    // account-level limit, as the only throttles on every method. Re-declare it
    // here so the §2.1 figure survives. It drops `deployOptions.cachingEnabled`
    // from that default entry too, which is why `defaultCachingOff` below
    // restates the caching half rather than leaving it to be inferred.
    //
    // Despite the name, this is NOT an aggregate ceiling across the stage: a
    // per-API per-stage limit is applied per method, so this grants every method
    // its OWN bucket of `apiGatewayThrottleRate`. It bounds what any one route
    // can draw, not what the stage can draw in total. The only genuinely shared
    // pool above the usage plan is the account-level limit (10 000 RPS / 5 000
    // burst in eu-central-1).
    const defaultMethodThrottle = {
      resourcePath: '/*',
      httpMethod: '*',
      throttlingRateLimit: config.apiGatewayThrottleRate,
      throttlingBurstLimit: config.apiGatewayThrottleBurst,
    };

    // Caching OFF for every method that does not opt in below (task 0184).
    //
    // This was already the effective behaviour — the default entry above never
    // declared `cachingEnabled`, and API Gateway treats an undeclared method as
    // uncached — but it was an accident of what the entry happened to omit, not
    // a stated rule. Adding `CachingEnabled: true` here would have silently
    // switched on the cache for every route without one, including the portal's
    // session traffic (task 0186). More specific entries still win, so the six
    // routes that enable it below are unaffected.
    //
    // It is a wildcard rather than one entry per uncached route because the
    // portal's route CANNOT have one: API Gateway assembles the setting path as
    // `/{resourcePath}/{httpMethod}/caching/enabled`, and the `+` in a greedy
    // `{proxy+}` segment makes that unparseable — `/api-tokens/api/{proxy+}/ANY/
    // caching/enabled` is rejected with `Invalid method setting path` at deploy
    // time, though a change set accepts it. (Multi-segment paths are otherwise
    // fine; `/v1/assets/{asset_identifier}/price` below is deployed and works.)
    // So the guarantee has to be expressed by the wildcard, which is the form
    // AWS documents as `/*/*/caching/enabled`.
    const defaultCachingOff = {
      ...defaultMethodThrottle,
      cachingEnabled: false,
    };

    // One entry, not two: method settings are keyed by resourcePath+httpMethod,
    // so a separate throttle entry for this route would collide with its cache
    // entry. The throttle is declared OUTSIDE the `cacheEnabled` branch on
    // purpose — with the cache off, every anonymous request is a billed Lambda
    // invocation, which is precisely when an unthrottled keyless route costs
    // the most.
    const apiDocsSettings = {
      resourcePath: '/api-docs-json',
      httpMethod: 'GET',
      throttlingRateLimit: API_DOCS_THROTTLE.rate,
      throttlingBurstLimit: API_DOCS_THROTTLE.burst,
      ...(cacheEnabled
        ? {
            cachingEnabled: true,
            cacheTtlInSeconds: CACHE_TTL.apiDocs.toSeconds(),
          }
        : {}),
    };

    if (cacheEnabled) {
      cfnStage.methodSettings = [
        defaultCachingOff,
        apiDocsSettings,
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
        // Redundant against `defaultCachingOff`, kept as documentation: these
        // two are uncached by intent, not merely by omission.
        {
          resourcePath: '/v1/prices/batch',
          httpMethod: 'POST',
          cachingEnabled: false,
        },
        { resourcePath: '/health', httpMethod: 'GET', cachingEnabled: false },
        // `/api-tokens/api/{proxy+}` is covered by `defaultCachingOff` above,
        // and cannot be listed here — see the comment on that constant.
      ];
    } else {
      // No cache cluster, so no TTLs to declare — but the throttles still
      // apply, and this is the configuration in which the anonymous route is
      // most expensive to leave unbounded.
      cfnStage.methodSettings = [defaultMethodThrottle, apiDocsSettings];
    }

    // ---------------------------------------------------------------
    // UsagePlan + API key — the `pricing-api-free` tier (task 0157).
    //
    // One plan, because a key belongs to exactly one plan per stage and
    // self-service is the default (and currently only) way to hold a key.
    // Higher limits are a manual, out-of-band arrangement made by hand in the
    // console — see docs/runbooks/manual-api-key-tier.md.
    //
    // The construct id stays `UsagePlan` so this updates the deployed plan in
    // place rather than creating a second one: every property of
    // AWS::ApiGateway::UsagePlan, including UsagePlanName, is "no interruption".
    // ---------------------------------------------------------------
    const usagePlan = this.api.addUsagePlan('UsagePlan', {
      name: `pricing-api-free-${config.envName}`,
      throttle: {
        rateLimit: config.pricingApiFreePlanRateLimit,
        burstLimit: config.pricingApiFreePlanBurstLimit,
      },
      quota: {
        limit: config.pricingApiFreePlanMonthlyQuota,
        period: apigateway.Period.MONTH,
      },
    });
    usagePlan.addApiStage({ stage: this.api.deploymentStage });

    // Two separate lines here can rotate this key, by two different mechanisms:
    //
    // 1. Changing the CONSTRUCT ID changes the logical id, so CloudFormation sees
    //    a removal and an unrelated addition. That is what task 0157 did
    //    (`PartnerApiKey` -> `PricingApiFreeApiKey`).
    // 2. Changing `apiKeyName` alone is a Replacement — AWS::ApiGateway::ApiKey
    //    .Name is "update requires replacement".
    //
    // The distinction is bookkeeping, not safety: CloudFormation "usually creates
    // the replacement resource first, changes references ... and then deletes the
    // old resource", so under BOTH paths the old key stays valid for the whole
    // update, dies only in the post-success cleanup phase, and survives a
    // mid-update rollback untouched.
    //
    // What matters is the part that is the same either way: the key gets a new
    // value and every holder is cut off. Deliberate here; touch neither line
    // casually.
    //
    // This is the ONLY key on the plan that CloudFormation manages, and it is
    // ours — the one verification curls authenticate with. Task 0160 mints a key
    // per Discord user onto the same plan via the SDK at runtime; those never
    // appear in this template and no deploy can touch them. So "a key on
    // pricing-api-free" is not the same thing as "this key", and only this one
    // has no owning row in 0158's registry.
    const apiKey = this.api.addApiKey('PricingApiFreeApiKey', {
      apiKeyName: `pricing-api-free-${config.envName}-key`,
    });
    usagePlan.addApiKey(apiKey);

    new ssm.StringParameter(this, 'ApiGatewayIdParam', {
      parameterName: `/prices/${config.envName}/api-gateway-id`,
      stringValue: this.api.restApiId,
      description: `REST API ID for prices-${config.envName}-api`,
    });

    // The onboarding backend (task 0160) issues keys and reads per-key usage,
    // both of which need the plan id. It lives in ComputeStack, which this stack
    // depends on, so it cannot read the plan object without closing the cycle —
    // same shape as the apiBaseUrl problem in task 0124. Publish via SSM instead.
    new ssm.StringParameter(this, 'PricingApiFreePlanIdParam', {
      parameterName: `/prices/${config.envName}/pricing-api-free-plan-id`,
      stringValue: usagePlan.usagePlanId,
      description: `Usage plan ID for pricing-api-free-${config.envName} (key issuance + GetUsage)`,
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
