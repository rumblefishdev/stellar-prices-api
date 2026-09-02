import * as cdk from 'aws-cdk-lib';
import * as apigateway from 'aws-cdk-lib/aws-apigateway';
import * as acm from 'aws-cdk-lib/aws-certificatemanager';
import type * as lambda from 'aws-cdk-lib/aws-lambda';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as route53 from 'aws-cdk-lib/aws-route53';
import * as targets from 'aws-cdk-lib/aws-route53-targets';
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
const PORTAL_API_RESOURCE_PATH = '/api/{proxy+}';

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
 * `/api/auth/login` answers the gateway's own
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
 * Method-level throttle for the portal's backend (`/api/...`).
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
 * new dependency for a threat nobody has seen yet.
 *
 * ✅ **The WAF half is now DECIDED, not deferred (task 0126, 2026-09-02): NO.**
 * This comment used to say task 0194 should cost the portal's traffic first and
 * then decide; 0194 landed on 2026-09-01 and the decision was taken. The gap
 * described above — per-caller VOLUME, which the throttles do not bound — is
 * real and is the strongest argument for one; it loses to ~$5-6/mo per web ACL
 * plus per-request charges against a §10 budget of ~$108/mo, for a threat
 * nobody has observed. Four named triggers reverse it, and if it is ever
 * reversed the rule ships in COUNT mode first so it cannot corrupt task 0121's
 * load measurement. Read the decision in task 0126 before re-opening it.
 */
const PORTAL_THROTTLE = { rate: 10, burst: 40 } as const;

/**
 * The header the bundle marks its state-changing calls with —
 * `PORTAL_REQUEST_HEADER` in `packages/prices-api/src/portal/keys/mod.rs`,
 * where it is the CSRF guard on the revoke. Non-safelisted, so the preflight
 * below has to name it or the browser never sends the revoke at all: the
 * failure is a `403` from the preflight in the network tab and a "could not
 * be reached" on the page, with nothing in any log of ours.
 */
const PORTAL_REQUEST_HEADER = 'X-Requested-With';

/**
 * How long a browser may reuse one preflight answer (task 0194).
 *
 * The bundle's calls are simple `GET`s except the revoke, which carries the
 * header above, and every one of them carries the session cookie — a
 * credentialed request, which is what makes the answer cacheable per origin
 * rather than per URL. Chromium caps this at two hours; one is the figure the
 * block explorer settled on for the same reason (its task 0317), and there is
 * no argument for a different one here.
 */
const PORTAL_PREFLIGHT_MAX_AGE = cdk.Duration.hours(1);

/**
 * The `/v1` data routes' CORS answer — task 0126.
 *
 * ⚠️ **This gateway now carries TWO CORS policies, and they disagree on
 * purpose. Neither is a mistake, and the reason is not a preference.**
 *
 * | | portal (`/api/{proxy+}`) | data routes (`/v1/*`) |
 * |---|---|---|
 * | origins | ONE (`config.portalWebOrigin`) | `*` |
 * | credentials | `true` | absent |
 *
 * The browser leaves no choice. `Access-Control-Allow-Origin: *` **cannot** be
 * combined with credentials — a browser rejects that pairing outright. The
 * portal's calls carry the session cookie ([[0186]]), so it is *forced* to name
 * exactly one origin; the data routes carry no cookie and no session, so `*` is
 * available to them and costs nothing.
 *
 * And it genuinely costs nothing, which is the part worth stating rather than
 * assuming. CORS protects a browser user's AMBIENT authority — credentials the
 * browser attaches on its own. `/v1` has none: auth is an `x-api-key` header
 * the caller supplies deliberately. So a hostile page calling `/v1` gets
 * exactly what `curl` gets, and restricting the origin would block only
 * browsers — the one client where it costs a legitimate integrator something
 * and stops no attacker, since a script or a server performs no CORS at all.
 *
 * Two alternatives were weighed and rejected (task 0126, decision 1):
 * mirroring the request `Origin` is equivalent in effect but makes the answer
 * vary per caller, needing `Vary: Origin` and splitting the stage cache into
 * one entry per origin — and this gateway's cache is ON, with a cache-key
 * mistake already on record (task 0118, cross-caller poisoning measured on
 * production 2026-08-28). An allowlist is only coherent if we intend to control
 * who builds against the API; keys are self-service and the onboarding page
 * ships example queries.
 */
const DATA_CORS_ALLOW_ORIGINS = ['*'];

/**
 * `x-api-key` is the one entry here that is load-bearing rather than
 * conventional: it is how every data route authenticates, and a header absent
 * from this list fails preflight in the browser before the request is ever
 * sent. It is also in `apigateway.Cors.DEFAULT_HEADERS` — listed explicitly
 * anyway, because a default that silently stops including it would take every
 * browser integrator down with it and nothing here would show why.
 *
 * `Content-Type` is needed by `POST /v1/prices/batch`; `Accept` is harmless and
 * conventional. Nothing else is used, and a longer list is a longer answer on
 * every preflight.
 */
const DATA_CORS_ALLOW_HEADERS = ['Content-Type', 'Accept', 'x-api-key'];

/**
 * Preflight cache lifetime for the data routes.
 *
 * ⚠️ A browser keys its preflight cache on (origin, URL, method, headers,
 * credentials mode) NO MATTER what the response says — a `*` answer is not
 * shared between origins, and an earlier draft of this comment claimed it was.
 * So the saving is per origin per route: one round trip avoided on every
 * repeat call an integrator's page makes, which is most of them.
 *
 * Chromium caps the header at two hours regardless. One hour matches
 * `PORTAL_PREFLIGHT_MAX_AGE`, so the two policies differ only where they must.
 */
const DATA_PREFLIGHT_MAX_AGE = cdk.Duration.hours(1);

/**
 * Every verb the stage carries a portal method setting for: the three Lambda
 * verbs plus the `OPTIONS` preflight, which is a MOCK — no invocation, but a
 * billed gateway request, and the one verb an anonymous loop can drive without
 * even a session. Same throttle as the rest of the prefix, for the same reason.
 */
const PORTAL_STAGE_METHODS = [...PORTAL_API_METHODS, 'OPTIONS'] as const;

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
 *   documentation). `GET`/`POST`/`DELETE` on `/api/{proxy+}` are the
 *   onboarding portal's backend (task 0184), keyless for the same reason, gated
 *   in the handler by `PORTAL_ENABLED` (task 0183) and carrying their own
 *   method-level throttle since they sit outside the usage plan.
 * - **Response cache**: 0.5 GB stage cache with per-endpoint TTLs (`CACHE_TTL`),
 *   opt-in per method; each cached method declares its query params as cache
 *   keys.
 *
 * - **Hostname**: `config.apiDomain` (task 0194) — a REGIONAL custom domain
 *   with a DNS-validated certificate and Route 53 aliases, mapped at the root
 *   to this stage. The portal's bundle lives on another distribution and
 *   calls the backend here, cross-origin; the `OPTIONS` preflight on
 *   `/api/{proxy+}` and the CORS headers on the gateway's own error
 *   responses exist for that one caller, `config.portalWebOrigin`.
 *
 * WAF WebACL: **decided against, not deferred** — task 0126, 2026-09-02, with
 * reasoning and four reversal triggers. The REST API ID is still published to
 * SSM at `/prices/{env}/api-gateway-id`, which is what a web ACL would attach
 * to if that decision is ever reversed.
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
    /**
     * The `/v1` preflight, added to a resource that carries data methods
     * (task 0126). See `DATA_CORS_ALLOW_ORIGINS` for why this policy differs
     * from the portal's on the same gateway.
     *
     * Three properties are each load-bearing and each fail in a way that looks
     * like something else:
     *
     * - **MOCK, not the Lambda.** `addCorsPreflight` emits a MockIntegration,
     *   so an `OPTIONS` never invokes the handler, never costs an invocation
     *   and never touches ClickHouse. A per-handler CORS layer would answer the
     *   same headers and bill every preflight — and a browser sends one before
     *   nearly every cross-origin call.
     * - **No API key.** CDK does not set `apiKeyRequired` on the method it
     *   emits, which is the only correct answer: a preflight is sent by the
     *   browser BEFORE the caller's code runs and cannot carry one. Requiring a
     *   key here would 403 the preflight and take every browser integrator
     *   down while `curl` kept working perfectly.
     * - **Per resource, not per prefix.** API Gateway has no wildcard for this;
     *   a resource without its own `OPTIONS` answers 403, and the browser
     *   reports that as a network failure rather than as a missing route. So
     *   every resource carrying a data method gets one — which is why this is
     *   folded into `addGet` rather than left as a call to remember.
     *
     * These methods get NO stage entry of their own, so they inherit the
     * wildcard default below: uncached, and throttled at
     * `apiGatewayThrottleRate/Burst` like every other data route. Deliberate,
     * and different from the portal's `OPTIONS`, which carries the tighter
     * `PORTAL_THROTTLE`. That tighter figure is there because the portal's
     * verbs are keyless AND reach the Lambda; these reach a MOCK — no
     * invocation, no ClickHouse — so the exposure is a gateway request, the
     * same shape `/health` has carried on the stage default since it shipped.
     * Caching them would be worse than pointless: a preflight answer is already
     * cached by the BROWSER for `DATA_PREFLIGHT_MAX_AGE`.
     */
    const addDataCors = (
      resource: apigateway.IResource,
      allowMethods: string[],
    ) =>
      resource.addCorsPreflight({
        allowOrigins: DATA_CORS_ALLOW_ORIGINS,
        allowMethods: [...allowMethods, 'OPTIONS'],
        allowHeaders: DATA_CORS_ALLOW_HEADERS,
        // NO `allowCredentials`. Setting it true alongside `*` is refused by
        // every browser, and nothing on these routes has a credential to send.
        maxAge: DATA_PREFLIGHT_MAX_AGE,
      });

    /**
     * Add a key-gated GET with a cached integration, and the preflight that
     * lets a browser reach it.
     *
     * The two are declared together deliberately: a data route added later
     * without a preflight is invisible to `curl` and to every test here, and
     * shows up only as "the API cannot be called from a browser" — which is the
     * defect task 0126 exists to close. Coupling them makes forgetting it
     * require an edit rather than an omission.
     */
    const addGet = (resource: apigateway.IResource, cacheKeys: string[]) => {
      const method = resource.addMethod('GET', proxy(cacheKeys), {
        apiKeyRequired: true,
        requestParameters: declare(cacheKeys),
      });
      addDataCors(resource, ['GET']);
      return method;
    };

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
    // /api/{proxy+} — the onboarding portal's backend (0184).
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
    // `/api/{proxy+}` directly under the root: the bundle's paths share the
    // prefix but never reach the gateway — the page lives on another host
    // (task 0194) and only its backend calls come here, on the API's own
    // hostname. The OpenAPI alias `/api/api-docs-json` rides on this same
    // proxy.
    const portalProxy = this.api.root
      .addResource('api')
      .addResource('{proxy+}');
    for (const httpMethod of PORTAL_API_METHODS) {
      portalProxy.addMethod(httpMethod, proxy([]), { apiKeyRequired: false });
    }

    // The preflight for the bundle's cross-origin calls (task 0194).
    //
    // The bundle is served from `config.portalWebOrigin` — another
    // application's distribution, whose `/api/*` behaviour is a static SPA
    // that rewrites every extensionless path to its `index.html`. A
    // same-origin `fetch('/api/config')` from there gets the bundle back as
    // `200 text/html`, so the bundle calls this API on `config.apiDomain`
    // instead, and a browser asks before a non-simple request crosses an
    // origin: the revoke's marker header (`PORTAL_REQUEST_HEADER`) is one, and
    // so is any `DELETE`. Answered here by a MOCK integration rather than by
    // the Lambda — no invocation, and the `max-age` has to be set by whoever
    // answers the preflight, which is the gateway — with the handler adding
    // `Access-Control-Allow-Origin` on the actual responses (`portal::apply`
    // in the Rust crate names the same one origin).
    //
    // ONE origin, exactly, and `allowCredentials: true`: the calls carry the
    // session cookie, and a credentialed answer cannot use `*`. The same
    // value reaches the handler as `PORTAL_WEB_ORIGIN`, so the two answers
    // cannot drift apart — `validateConfig` checks its shape once.
    portalProxy.addCorsPreflight({
      allowOrigins: [config.portalWebOrigin],
      allowMethods: [...PORTAL_API_METHODS],
      allowHeaders: ['Content-Type', 'Accept', PORTAL_REQUEST_HEADER],
      allowCredentials: true,
      maxAge: PORTAL_PREFLIGHT_MAX_AGE,
    });

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
    const batch = prices.addResource('batch');
    batch.addMethod('POST', proxy([]), {
      apiKeyRequired: true,
    });
    // The one data route that is not a GET, so it needs its preflight named
    // rather than inherited from `addGet`. It is also the route that most needs
    // one: a JSON `POST` is never a "simple" request, so a browser preflights
    // it unconditionally — `Content-Type: application/json` alone is enough.
    addDataCors(batch, ['POST']);

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
    const portalSettings = PORTAL_STAGE_METHODS.map((httpMethod) => ({
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
    // This is the ONLY key on the plan that CloudFormation manages. Task 0160
    // mints a key per Discord user onto the same plan via the SDK at runtime;
    // those never appear in this template and no deploy can touch them. So "a
    // key on pricing-api-free" is not the same thing as "this key", and only
    // this one has no owning row in 0158's registry.
    //
    // ⚠️ An earlier revision added "and it is ours — the one verification curls
    // authenticate with". **That was false, and it cost an hour on 2026-08-28.**
    // The key our tooling actually carried (`.env.local`, the 0120 conformance
    // suite, the 0121 load test) was `smdesqkg5j`, a key created outside this
    // template; the CloudFormation-managed one was `t61phbbhhj`. When
    // `smdesqkg5j` was deleted by hand at 14:00 UTC, every keyed route began
    // answering 403 and this comment sent the investigation at the deploy that
    // had run 30 minutes earlier — which had touched no key at all (CloudTrail
    // settled it; `cdk diff` had shown only method and deployment changes).
    //
    // So do not assume the key in anyone's environment is this one. The
    // authority is the stack's own `ApiKeyId` output:
    //   aws cloudformation describe-stacks --stack-name Prices-<env>-ApiGateway \
    //     --query "Stacks[0].Outputs[?OutputKey=='ApiKeyId'].OutputValue"
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

    // CORS on the gateway's OWN error answers (task 0194): a `429` from the
    // throttle above, a `504` when the Lambda runs out of time. Neither
    // reaches the handler, so neither carries its `Access-Control-Allow-Origin`
    // — and to a browser on `portalWebOrigin` a cross-origin response without
    // one is not a `429`, it is a network error. The bundle's `getJson` would
    // then report "could not be reached", which points at the visitor's
    // network when the cause is a throttle that has a status and a body. A
    // static value rather than the request's `Origin`, because a gateway
    // response cannot read one: this is the one origin the API serves a page
    // to, and a browser on any other origin rejects the mismatch and shows the
    // same opaque error it would have shown anyway.
    //
    // ⚠️ **A gateway response is scopable by `ResponseType` and by nothing
    // else** — there is no path, resource or stage dimension. So every entry
    // here is API-WIDE, `/v1` included, and the header goes out even on a
    // request that carried no `Origin` at all. That is why the 4xx half is
    // `THROTTLED` and not `DEFAULT_4XX`, which 0194's review found stamping
    // the portal's origin onto every keyless `/v1` `403` — a set of answers
    // the portal never reads, on the far larger half of the API, for no gain.
    // `THROTTLED` is the one 4xx the bundle has to be able to tell apart from
    // a dead network, and it is the same `429` on `/v1`, so widening it there
    // costs nothing: the body is API Gateway's own generic text and the header
    // lets a page on OUR origin read a status it could already read by
    // sending the request itself.
    //
    // `DEFAULT_5XX` stays broad on the same reasoning and cannot be narrowed
    // usefully anyway — every 5xx here is ours (integration failure, timeout),
    // there is no per-path form of it, and the alternative is enumerating
    // response types that AWS may add to.
    for (const [id, type] of [
      ['PortalCors4xx', apigateway.ResponseType.THROTTLED],
      ['PortalCors5xx', apigateway.ResponseType.DEFAULT_5XX],
    ] as const) {
      this.api.addGatewayResponse(id, {
        type,
        responseHeaders: {
          'Access-Control-Allow-Origin': `'${config.portalWebOrigin}'`,
          'Access-Control-Allow-Credentials': "'true'",
          Vary: "'Origin'",
        },
      });
    }

    // ---------------------------------------------------------------
    // The API's own hostname — `config.apiDomain` (task 0194).
    // ---------------------------------------------------------------
    // What the block explorer's `api-gateway-stack.ts` does for
    // `api.sorobanscan.rumblefish.dev`, in the same zone: a REGIONAL domain,
    // TLS 1.2, alias A + AAAA records. Two choices differ from theirs and are
    // worth stating:
    //
    // - **The certificate is created here**, DNS-validated in the zone,
    //   rather than referenced by ARN. The wildcard `*.sorobanscan…` cert in
    //   this region belongs to no stack, is attached to nothing
    //   (`InUseBy: []`), and ACM reports it `INELIGIBLE` for renewal — a
    //   domain hung on it would go dark on its expiry with nobody's deploy to
    //   blame. One we own, in use, renews.
    // - **The base path is the root.** `addDomainName` maps the domain to
    //   this deployment stage, so `https://{domainName}/v1/…` is the data API
    //   and `https://{domainName}/api/…` is the portal's backend, with no
    //   stage segment in either — which is what a bundle built with
    //   `VITE_PORTAL_API_ORIGIN` assumes, and what `AWS_LAMBDA_HTTP_IGNORE_
    //   STAGE_IN_PATH` already made the handler indifferent to.
    //
    // The execute-api endpoint stays enabled. Nothing advertises it any more
    // (`apiBaseUrl` names this hostname, and the distribution that fronted it
    // as an origin was retired by task 0195), but it still answers; whether
    // it is announced as retired or kept as a permanent alias is task 0126's
    // decision.
    const hostedZone = route53.HostedZone.fromHostedZoneAttributes(
      this,
      'HostedZone',
      {
        hostedZoneId: config.apiDomain.hostedZoneId,
        zoneName: config.apiDomain.hostedZoneName,
      },
    );
    const certificate = new acm.Certificate(this, 'ApiCertificate', {
      domainName: config.apiDomain.domainName,
      validation: acm.CertificateValidation.fromDns(hostedZone),
    });
    const apiDomain = this.api.addDomainName('ApiDomain', {
      domainName: config.apiDomain.domainName,
      certificate,
      endpointType: apigateway.EndpointType.REGIONAL,
      securityPolicy: apigateway.SecurityPolicy.TLS_1_2,
    });
    // Relative to the zone — `validateConfig` guarantees the suffix.
    const recordName = config.apiDomain.domainName.slice(
      0,
      -(config.apiDomain.hostedZoneName.length + 1),
    );
    const alias = route53.RecordTarget.fromAlias(
      new targets.ApiGatewayDomain(apiDomain),
    );
    new route53.ARecord(this, 'ApiARecord', {
      zone: hostedZone,
      recordName,
      target: alias,
    });
    new route53.AaaaRecord(this, 'ApiAaaaRecord', {
      zone: hostedZone,
      recordName,
      target: alias,
    });

    new cdk.CfnOutput(this, 'ApiCustomDomain', {
      value: `https://${config.apiDomain.domainName}`,
      description: 'The API on its own hostname (root base path → this stage)',
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
