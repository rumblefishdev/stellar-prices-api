import * as cdk from 'aws-cdk-lib';
import * as apigateway from 'aws-cdk-lib/aws-apigateway';
import type * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
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
  /**
   * The api-handler's execution role, so the one control-plane grant that needs
   * the usage-plan id can be declared here (task 0187).
   *
   * The other four `apigateway:` grants live in `ComputeStack`, on resources
   * that need no id from this stack. This one cannot: `usagePlan.usagePlanId`
   * is created below, and a policy in `ComputeStack` referencing it would make
   * ComputeStack import an export of ApiGatewayStack — while ApiGatewayStack
   * already imports the Lambda from ComputeStack. That is a cycle, and
   * CloudFormation refuses it.
   *
   * Declaring the policy HERE and attaching it to the passed-in role keeps the
   * reference pointing the one way that works: ApiGateway -> Compute.
   */
  readonly apiHandlerRole: iam.IRole;
}

/**
 * Per-endpoint response-cache TTLs (overview §2.1). The cache key includes the
 * path params (automatic) plus the query params declared per method below, so
 * paginated / parameterized reads cache correctly. Caching is opt-IN: anything
 * without an entry in `methodSettings` below is uncached by the `/*` `*`
 * default, which covers `POST /prices/batch` and `GET /health`. The portal's
 * routes state it themselves — see `portalSettings`.
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
 * The portal backend's gateway resource path, in the form API Gateway uses for
 * a method setting.
 *
 * One greedy `{proxy+}`, so a later slice adds a route at any depth without
 * touching this file — the same property task 0183's prefix gate is built on.
 *
 * **`ANY` is what cannot carry a stage method setting — not `{proxy+}`.** This
 * is worth stating exactly, because the first two attempts at this got the
 * diagnosis wrong and the second one broke production for twenty minutes. API
 * Gateway addresses a stage method setting by the string path
 * `/{resourcePath}/{httpMethod}/{setting}` and rejects the whole update if it
 * cannot resolve one — and `ANY` is never a resolvable `{httpMethod}` there,
 * whatever the resource path looks like. Measured against a throwaway REST API
 * on 2026-08-14, one variable at a time:
 *
 * | setting path                              | verdict  |
 * | ----------------------------------------- | -------- |
 * | `/lit/GET/caching/enabled`                | accepted |
 * | `/pg/{p}/GET/caching/enabled`             | accepted |
 * | `/g/{proxy+}/GET/caching/enabled`         | accepted |
 * | `/g/{proxy+}/GET/throttling/rateLimit`    | accepted |
 * | `/p/{proxy+}/POST/throttling/rateLimit`   | accepted |
 * | `/lit/ANY/caching/enabled`                | REJECTED |
 * | `/pa/{p}/ANY/caching/enabled`             | REJECTED |
 * | `/deep/{p}/{s}/ANY/caching/enabled`       | REJECTED |
 * | wildcard on the verb alone, `/lit/[*]/…`  | REJECTED |
 *
 * So the `+` is fine, braces are fine, depth is fine, and the only wildcard
 * form the service accepts is the both-segment one used by the default entry
 * below. What has to go is `ANY`, which is why the verbs are enumerated in
 * `PORTAL_API_METHODS`.
 *
 * A change set accepts every one of the rejected forms, so this fails on apply
 * and only on apply. Do not read a clean `cdk diff` as evidence here.
 */
const PORTAL_API_RESOURCE_PATH = '/api/api/{proxy+}';

/**
 * The verbs mapped under the portal prefix, enumerated because `ANY` cannot
 * carry the throttle above — see `PORTAL_API_RESOURCE_PATH`.
 *
 * Covers the epic as sliced: `GET` for `/config` (task 0183), task 0186's
 * `/auth/login`, `/auth/callback` and `/auth/me`, and task 0188's `/usage`;
 * `POST` for task 0187's key issue, task 0191's rework and task 0186's
 * `/auth/logout`; `DELETE` for task 0192's revoke if it prefers that shape to a
 * `POST`.
 *
 * ⚠️ Task 0186's four routes sit at **depth 3** (`auth/login`, not `login`),
 * which the greedy `{proxy+}` above covers and the intermediate
 * `{proxy}` + `{proxy}/{sub}` pair that is CURRENTLY DEPLOYED does not — see
 * task 0205, which ships this file's committed shape. Until that deploy runs,
 * `/api/api/auth/login` answers the gateway's own
 * `403 Missing Authentication Token` rather than reaching the handler. That is a
 * deployment gap, not a defect in either task: the flag keeps the handler dark
 * regardless, and `/config` at depth 1 is unaffected.
 *
 * ⚠️ A verb that is NOT listed here gets the gateway's `403 Missing
 * Authentication Token` rather than task 0183's empty `404`, which is a smaller
 * version of the hole `ANY` was chosen to avoid: paths stay free, verbs do not.
 * Adding one is a line here and a deploy — cheap, but it is a CDK change, so a
 * slice that needs `PATCH` should notice at design time rather than in a
 * browser.
 */
const PORTAL_API_METHODS = ['GET', 'POST', 'DELETE'] as const;

/**
 * Method-level throttle for the portal's backend (`/api/api/...`).
 *
 * Same argument as `API_DOCS_THROTTLE` above, and the same reason it cannot be
 * left to the default. These routes are anonymous by design — a visitor signing
 * in to get a key does not have one yet (task 0186) — so they sit outside the
 * usage plan and get neither of the two limits that protect every `/v1` route:
 * the per-key rate (`pricingApiFreePlanRateLimit`, 1 req/s) and the monthly
 * quota (100 000). Without an entry here they inherit the `/*` `*` default of
 * `apiGatewayThrottleRate` — 200 req/s, i.e. 200x the free tier, with no key
 * required to draw it. Task 0186 makes this a written acceptance criterion and
 * task 0194 audits the assembled array for it.
 *
 * They are also uncached at BOTH layers by requirement rather than by omission
 * (task 0187 forbids caching a key reveal; task 0186 needs the session cookie at
 * the origin), so every request is a billed gateway request AND a billed Lambda
 * invocation. At the old ceiling that is ~518M requests/month — on the epic's
 * own ~$3.50/million, four figures a month from an unauthenticated loop, against
 * an epic that cut the *keyed* limit from 100 req/s to 1 req/s because 100 was
 * ~$900/month.
 *
 * Note the exposure does not wait for `PORTAL_ENABLED`: task 0183's gate is axum
 * middleware INSIDE the handler, so a request to a closed portal still costs a
 * full invocation to be told `404`.
 *
 * Sized at the same rate as `API_DOCS_THROTTLE` so the two anonymous routes are
 * consistent, with a wider burst because the unit differs: one portal page load
 * is several calls (`/config`, `/auth/me`, `/key`, `/usage`), so 40 absorbs ~10
 * simultaneous loads while 10/s sustained is several times any realistic peak
 * for a portal gated behind Stellar Discord membership.
 *
 * What this buys is COST CONTROL, not availability: a method throttle is a
 * global cap, not per-caller, so under a flood legitimate users see `429` too.
 * Two limits it does NOT provide, worth knowing before treating this as the end
 * of the subject:
 *
 * - It bounds the RATE, not the VOLUME — the same gap `types.ts` records for the
 *   free plan, where the monthly quota "binds ~26x harder" than the rate. A
 *   keyless route has no quota to reach for, so a caller sitting at this ceiling
 *   still accumulates. This bounds the blast radius by ~20x; it does not make it
 *   zero, and nothing here notices if it happens.
 * - It is not per-caller, so it does not distinguish one loop from a crowd.
 *
 * Both are answerable — a volume alarm for the first, a WAF rate-based rule for
 * the second — and both are deliberately NOT built here: a standing cost and a
 * new dependency for a threat nobody has seen yet. Task 0194 costs the portal's
 * traffic before the flag is flipped, which is the right moment to decide
 * whether either is worth buying.
 */
const PORTAL_THROTTLE = { rate: 10, burst: 40 } as const;

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
 *   documentation). `GET`/`POST`/`DELETE` on `/api/api/{proxy+}` are the
 *   onboarding portal's backend (task 0184), keyless for the same reason, gated
 *   in the handler by `PORTAL_ENABLED` (task 0183) and carrying their own
 *   method-level throttle since they sit outside the usage plan.
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

    const { config, apiHandlerFunction, apiHandlerRole } = props;
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
    // the usage plan, so it has neither of the two limits a keyed route carries.
    // The lever that replaces them is a method-level throttle, not a key
    // requirement — `API_DOCS_THROTTLE` above, which is what keeps this off the
    // stage-wide default it would otherwise share with paying traffic. Two more
    // things keep the residual small: the 3600s TTL below with **no** cache-key
    // parameters, so all callers collapse onto one entry, and API Gateway's
    // default `requireAuthorizationForCacheControl: true`, which stops an
    // anonymous caller busting that entry with `Cache-Control: max-age=0`.
    this.api.root.addResource('api-docs-json').addMethod('GET', proxy([]), {
      apiKeyRequired: false,
    });

    // ---------------------------------------------------------------
    // /api/api/{proxy+} — the onboarding portal's backend (0184).
    // ---------------------------------------------------------------
    // Without these resources the portal's routes are unreachable in production
    // no matter what the handler does: CloudFront forwards the request, the
    // gateway maps nothing, and the caller gets the gateway's own
    // `403 Missing Authentication Token` instead of the empty `404` task 0183's
    // gate is careful to produce. This is the "door" that task's note says
    // arrives here.
    //
    // **One greedy resource, enumerated verbs.** The point of task 0183's prefix
    // gate is that a later slice adds a route without editing the gate; the same
    // has to hold at the gateway, or every slice pays for a CDK change and a
    // deploy. `{proxy+}` covers task 0186's `/auth/*`, task 0187's `/key`, task
    // 0188's `/usage` and task 0192's `/key/revoke` at any depth with no further
    // work here. The axum router decides what actually exists — which while
    // `PORTAL_ENABLED` was false meant "nothing", byte-identical to a path that
    // was never deployed. Task 0194 has since flipped the flag, so these verbs
    // now reach real handlers.
    //
    // The verbs are listed rather than collapsed into `ANY` because `ANY` cannot
    // carry the throttle below, and that throttle is task 0186's acceptance
    // criterion. It is `ANY` that is the obstacle and not the `+` — the evidence
    // is tabulated on `PORTAL_API_RESOURCE_PATH` at the top of this file, and
    // the cost of believing otherwise is recorded in task 0184.
    //
    // **Keyless**, matching `auth::is_exempt`: a visitor signing in to get a
    // key does not have one yet, so requiring one is a self-service dead end —
    // the same argument that makes `/api-docs-json` anonymous. This is not a
    // hole: the flag decides whether these routes answer at all, and once they
    // do, task 0186's session is what authenticates them.
    //
    // **Throttled and uncached by entries of their own**, in `portalSettings`
    // below — declared outside the `cacheEnabled` branch, because with the stage
    // cache off is exactly when an unbounded anonymous route costs the most.
    const portalProxy = this.api.root
      .addResource('api')
      .addResource('api')
      .addResource('{proxy+}');
    for (const httpMethod of PORTAL_API_METHODS) {
      portalProxy.addMethod(httpMethod, proxy([]), { apiKeyRequired: false });
    }

    const PATH_ID = 'method.request.path.asset_identifier';
    const qs = (name: string) => `method.request.querystring.${name}`;

    const v1 = this.api.root.addResource('v1');

    // ⚠️ EVERY query parameter that changes the response body must be listed
    // here. API Gateway does NOT key the cache on the query string — it keys on
    // the parameters declared as `cacheKeyParameters`, and collapses every value
    // of an undeclared one onto a single entry. The failure is not a diluted hit
    // rate, it is cross-caller poisoning: measured on production 2026-08-28,
    // one `GET /v1/assets/native/price?min_volume_usd=200000` made the *next*
    // param-less request serve that caller's narrowed `sources` and reweighted
    // `vwap_24h` for the whole TTL. Task 0118 shipped the parameter believing
    // the gateway keyed on query params automatically; it does not.
    //
    // /v1/assets (list) + /v1/assets/{asset_identifier} (+ /price, /ohlcv)
    const assets = v1.addResource('assets');
    addGet(assets, [
      qs('type'),
      qs('search'),
      qs('sort'),
      qs('order'),
      qs('cursor'),
      qs('limit'),
      qs('min_volume_usd'),
    ]);
    const assetId = assets.addResource('{asset_identifier}');
    addGet(assetId, [PATH_ID]);
    addGet(assetId.addResource('price'), [PATH_ID, qs('min_volume_usd')]);
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
    // session traffic (task 0186). More specific entries still win, so the
    // seven routes that enable it below — `/api-docs-json` and the six `/v1`
    // reads — are unaffected.
    //
    // A wildcard rather than one entry per uncached route so the rule holds for
    // routes that do not exist yet, in the form AWS documents as
    // `/*/*/caching/enabled`. Both segments must be wildcards: the service
    // rejects a wildcard on the verb alone. It originally had a second reason —
    // the portal's greedy `{proxy+}` could not carry an entry of its own —
    // which turned out to be false (it can; `ANY` is what cannot), and the
    // portal now states its own caching in `portalSettings` below. The wildcard
    // stays because the stated default is worth having on its own.
    const defaultCachingOff = {
      ...defaultMethodThrottle,
      cachingEnabled: false,
    };

    // The portal backend: its own throttle, and caching explicitly off. Both are
    // declared OUTSIDE the `cacheEnabled` branch and spread into BOTH arms below
    // — the trap task 0186 warns about and task 0194 audits is that the full
    // array is assembled only inside `if (cacheEnabled)`, so an entry added to
    // that arm alone silently vanishes wherever the stage cache is off, which is
    // precisely the configuration where every anonymous request is a billed
    // Lambda invocation.
    const portalSettings = PORTAL_API_METHODS.map((httpMethod) => ({
      resourcePath: PORTAL_API_RESOURCE_PATH,
      httpMethod,
      throttlingRateLimit: PORTAL_THROTTLE.rate,
      throttlingBurstLimit: PORTAL_THROTTLE.burst,
      cachingEnabled: false,
    }));

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
        ...portalSettings,
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
      ];
    } else {
      // No cache cluster, so no TTLs to declare — but the throttles still
      // apply, and this is the configuration in which the anonymous routes are
      // most expensive to leave unbounded. `portalSettings` MUST appear here as
      // well as in the arm above; that symmetry is the whole point of declaring
      // it outside the branch, and task 0194 verifies it by flipping
      // `apiGatewayCacheEnabled` off in a synth and diffing.
      //
      // `defaultCachingOff` rather than the bare throttle for the same reason it
      // is used above: without a cache cluster the caching half is a no-op, but
      // the two arms should state the same rule, or the arm that omits it is
      // once again relying on an omission to mean something.
      cfnStage.methodSettings = [
        defaultCachingOff,
        apiDocsSettings,
        ...portalSettings,
      ];
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

    // The control-plane grants that need the plan id (tasks 0187 and 0188).
    // Declared here rather than in `ComputeStack` for the cycle reason on
    // `apiHandlerRole` in the props above; their four siblings are declared
    // there.
    //
    // `iam.Policy` rather than `apiHandlerRole.addToPrincipalPolicy`, and the
    // distinction is the whole point: `addToPrincipalPolicy` would append to
    // the role's default policy, which is a resource of ComputeStack, so the
    // plan id would travel as an export of THIS stack imported by that one —
    // the cycle again, just written differently. A standalone `Policy` is a
    // resource of this stack that names the role, so the reference runs
    // ApiGateway -> Compute like every other one here.
    //
    // Two statements, one sub-resource each, on THIS plan alone:
    //
    // - `POST …/keys` (task 0187) attaches a self-service key to the plan.
    // - `GET …/usage` (task 0188) is `GetUsage` — reading per-key consumption
    //   for the dashboard. `GET` on the usage sub-resource does NOT permit
    //   reading the plan itself (`GET /usageplans/{id}`), listing its keys
    //   (`GET …/keys`), or changing its limits — the resource path is the
    //   scope, and `/usage` is the narrowest form this call has.
    //
    // Deliberately NOT granted, though task 0187's review suggested deciding it
    // here: `GET /usageplans/{id}` to validate the plan at cold start. It would
    // turn a stale plan id into an init failure instead of a runtime one — but
    // 0187's decision 22 already rejected cold-start validation (a warm
    // container still misses a plan that changes under it, and the attach path
    // disambiguates a dead plan id into `PlanNotFound` loudly), and `GetUsage`
    // against a wrong plan id fails visibly on the first dashboard load. An
    // extra standing grant to move one failure earlier is not worth it.
    // The construct id and policyName predate the second statement (task 0187
    // named them for the attach, then task 0188 added the usage read) and are
    // KEPT: renaming an AWS::IAM::Policy is a resource replacement bought for
    // a cosmetic gain, on the policy whose absence breaks key issuance. Task
    // 0194's audit should read this policy as "the portal grants that need
    // the plan id", whatever the name says.
    new iam.Policy(this, 'PortalAttachKeyToFreePlan', {
      policyName: `prices-${config.envName}-portal-attach-key`,
      roles: [apiHandlerRole],
      statements: [
        new iam.PolicyStatement({
          sid: 'PortalAttachKeyToFreePlan',
          actions: ['apigateway:POST'],
          resources: [
            `arn:aws:apigateway:${config.awsRegion}::/usageplans/${usagePlan.usagePlanId}/keys`,
          ],
        }),
        new iam.PolicyStatement({
          sid: 'PortalReadFreePlanUsage',
          actions: ['apigateway:GET'],
          resources: [
            `arn:aws:apigateway:${config.awsRegion}::/usageplans/${usagePlan.usagePlanId}/usage`,
          ],
        }),
      ],
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
