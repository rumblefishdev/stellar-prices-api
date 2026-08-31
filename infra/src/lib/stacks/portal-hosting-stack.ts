import * as cdk from 'aws-cdk-lib';
import * as cloudfront from 'aws-cdk-lib/aws-cloudfront';
import * as origins from 'aws-cdk-lib/aws-cloudfront-origins';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as s3deploy from 'aws-cdk-lib/aws-s3-deployment';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

/**
 * Vite build output for the portal app (`web/portal`), relative to `infra/`
 * (cdk's CWD — same convention as `API_HANDLER_ASSET_DIR` in
 * `compute-stack.ts`).
 *
 * ⚠️ **Build the app before synthesizing.** `Code.fromAsset`'s S3 equivalent
 * packages whatever is on disk with no freshness check against the source, so
 * an unbuilt tree fails synth outright and a STALE one deploys silently and
 * reports success — the failure task 0141 records for the Lambda bootstraps,
 * arriving here for the frontend. `make -C infra deploy-production-portal`
 * builds it first for exactly this reason; if you are running `cdk` by hand,
 * run `npx nx build portal` first.
 *
 *     npx nx build portal        # → web/portal/dist
 *
 * This replaced task 0184's throwaway `assets/portal-placeholder/index.html`,
 * which said the portal was not yet available. The app now says that itself,
 * and says it on the authority of `GET /api/config` rather than of a
 * hardcoded sentence that nobody would remember to delete.
 */
const PORTAL_ASSET_DIR =
  process.env['PORTAL_ASSET_DIR'] ?? '../web/portal/dist';

/**
 * The portal's path layout, and the rule that produced it.
 *
 * **`/api/` is the whole self-service portal**, bundle and backend alike, with
 * no sub-prefix for either (task 0194, 2026-08-31 — decided by Adam). On the
 * shared host the root belongs to the block explorer, so everything of ours
 * lives under this one prefix: `/api/login` is a page, `/api/auth/login` is
 * the backend, `/api/api-docs-json` is the OpenAPI document.
 *
 * That replaces task 0161's `<app>/*` + `<app>/api/*` convention, which for an
 * app called "api" produced `/api/api/…`, and it inverts which side of the
 * split is enumerated. CloudFront evaluates cache behaviours **in order** and
 * the first match wins, so:
 *
 * - the bundle's paths — `PORTAL_BUNDLE_PATHS`, a short FIXED list — are carved
 *   out to S3 by rows listed **first**;
 * - `PORTAL_BACKEND` (`/api/*`) is the catch-all and goes to the API.
 *
 * The backend is the open-ended side (five slices added routes; none touched
 * this file), so it gets the catch-all. And the failure modes are asymmetric:
 * a bundle path missing from the list reaches the API and gets a loud JSON
 * `404`, whereas the old shape's failure — a backend call reaching S3 — was a
 * `200` full of HTML that only surfaced as a JSON parse error in a browser.
 *
 * `PORTAL_BACKEND` mirrors `PORTAL_API_PREFIX` in
 * `packages/prices-api/src/portal/mod.rs` and is the prefix the Discord
 * redirect URI (task 0186) is registered under; `PORTAL_APP_ROUTES` mirrors
 * the router in `web/portal/src/landing/links.ts`. `tools/scripts/
 * verify-openapi-routes.mjs` asserts the carve-outs and the catch-all against
 * the synthesized template, so a reorder cannot pass CI.
 */
const PORTAL_BACKEND = '/api/*';

/**
 * The SPA's client-side routes, without the prefix. Adding a page means adding
 * it here AND to `web/portal/src/landing/links.ts` — this list produces both
 * the CloudFront carve-out rows and the `DirectoryIndexFn` allow-list below.
 */
const PORTAL_APP_ROUTES = ['login', 'dashboard', 'quick-start'] as const;

/**
 * Every path under `/api/` that S3 serves. Both slash forms of each route,
 * because CloudFront path patterns are literal and the function that rewrites
 * `/api/login/` runs only AFTER a behaviour has matched.
 */
const PORTAL_BUNDLE_PATHS: readonly string[] = [
  '/api/',
  '/api/index.html',
  '/api/favicon.ico',
  '/api/assets/*',
  ...PORTAL_APP_ROUTES.flatMap((r) => [`/api/${r}`, `/api/${r}/`]),
];

export interface PortalHostingStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  /**
   * REST API id of `ApiGatewayStack`, used to build the execute-api origin
   * hostname. Passed as the id rather than the `RestApi` object because that is
   * all this stack needs and it keeps the cross-stack export to one string.
   */
  readonly restApiId: string;
}

/**
 * Portal hosting — private S3 bucket, CloudFront distribution, routing table
 * (task 0184).
 *
 * The walking skeleton for the self-service onboarding portal: a URL that
 * serves a page we own over TLS, plus the behaviour table every later slice
 * depends on. No app, no auth, no API calls from the page.
 *
 * **This distribution is production.** There is no staging environment — see
 * `packages/prices-api/src/portal/mod.rs` — so the portal's backend routes
 * shipped behind task 0183's `PORTAL_ENABLED` flag, answering with an empty
 * `404` until task 0194 flipped it. The gate existed before the door.
 *
 * Two things live here even though nothing yet uses them, because retrofitting
 * either would invalidate a decision made downstream:
 *
 * - **The API is a second origin on this distribution.** That is what makes
 *   every later portal request same-origin, which is what lets task 0186's
 *   session cookie be `SameSite=Lax` and keeps CORS out of portal traffic
 *   entirely. Both properties fail in a browser rather than in `curl`, so they
 *   are cheap now and expensive to discover later.
 * - **The bundle carve-outs ordered before the `/api/*` API row** — see the
 *   constants above.
 *
 * Deferred to task 0195 on purpose: Swagger UI at `/docs/*`, the per-prefix SPA
 * fallback (a real CloudFront Function, and pointless while the app has one
 * route), the custom domain and its `us-east-1` certificate. Deferred to task
 * 0186: the cookie-forwarding policy for the portal prefix, which is only
 * correct to write once there is a session to forward.
 */
export class PortalHostingStack extends cdk.Stack {
  public readonly bucket: s3.Bucket;
  public readonly distribution: cloudfront.Distribution;

  constructor(scope: Construct, id: string, props: PortalHostingStackProps) {
    super(scope, id, props);

    const { config, restApiId } = props;

    // ---------------------------------------------------------------
    // Origin 1 — the bundle bucket. Private, reachable only through OAC.
    // ---------------------------------------------------------------
    // A public bucket is the default way static hosting ships wrong, and
    // Tranche 3 AC 6 audits exactly this. `BLOCK_ALL` plus origin access
    // control means the only principal that can read an object is this
    // distribution, and only for the objects it is asked for.
    //
    // RETAIN, not DESTROY: the content is disposable but the bucket is
    // referenced by a live distribution, and a stack delete that also empties
    // the origin turns a rollback into an outage.
    this.bucket = new s3.Bucket(this, 'PortalBucket', {
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      encryption: s3.BucketEncryption.S3_MANAGED,
      enforceSSL: true,
      removalPolicy: cdk.RemovalPolicy.RETAIN,
    });

    // ---------------------------------------------------------------
    // Access logs.
    // ---------------------------------------------------------------
    // On, because CloudFront cannot backfill: a distribution that was not
    // logging at the time of an incident has nothing to reconstruct it from,
    // and the two things most likely to need reconstructing both arrive here.
    // `/api/*` is an anonymous entry point, and its throttle
    // (`PORTAL_THROTTLE`, 10 req/s) is a global rate cap rather than a
    // per-caller one — it bounds the bill, not the behaviour, so what a flood
    // looked like is a question only the logs can answer. Task 0186 then puts
    // Discord OAuth callbacks and session cookies through these same
    // behaviours.
    //
    // `logIncludesCookies` stays FALSE for that second reason: from task 0186
    // the portal's cookie IS the session, so including cookies would write a
    // usable credential into an S3 bucket in plaintext, for 90 days, to answer
    // a question the request line already answers.
    //
    // `BUCKET_OWNER_PREFERRED` is required, not stylistic — CloudFront standard
    // logging delivers via ACL grants, and S3's current default of
    // `BucketOwnerEnforced` disables ACLs, which makes delivery fail silently.
    const logBucket = new s3.Bucket(this, 'PortalLogBucket', {
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      encryption: s3.BucketEncryption.S3_MANAGED,
      enforceSSL: true,
      objectOwnership: s3.ObjectOwnership.BUCKET_OWNER_PREFERRED,
      lifecycleRules: [{ expiration: cdk.Duration.days(90) }],
      removalPolicy: cdk.RemovalPolicy.RETAIN,
    });

    // ---------------------------------------------------------------
    // Origin 2 — the API, as a second origin on the same distribution.
    // ---------------------------------------------------------------
    // `originPath` is load-bearing. API Gateway serves a REST API only under
    // `/{stage}`, so without it every proxied request arrives at the gateway
    // one path segment short and 403s — the same stage-prefix trap documented
    // on `apiBaseUrl` in `types.ts`.
    const apiOrigin = new origins.HttpOrigin(
      `${restApiId}.execute-api.${config.awsRegion}.amazonaws.com`,
      {
        originPath: `/${config.envName}`,
        protocolPolicy: cloudfront.OriginProtocolPolicy.HTTPS_ONLY,
      },
    );

    const s3Origin = origins.S3BucketOrigin.withOriginAccessControl(
      this.bucket,
    );

    // ---------------------------------------------------------------
    // Directory index rewrite (viewer request, S3 behaviours only).
    // ---------------------------------------------------------------
    // `defaultRootObject` applies to `/` alone — it is a property of the
    // distribution, not of a behaviour — so it cannot make `/api/`
    // resolve to `/api/index.html`. The other way to get per-directory
    // index documents is an S3 *website* endpoint, which requires a public
    // bucket and would trade away the AC above. Hence six lines of function.
    //
    // This is NOT task 0195's SPA fallback, which rewrites *unknown sub-paths*
    // to the owning app's `index.html` and has to be written per prefix. This
    // only appends `index.html` to a path that already ends in a slash, and
    // sends a path that is missing that slash to its canonical form.
    //
    // The trailing-slash redirects are not cosmetic. `/api/*` does NOT
    // match `/api`, so the bare prefix falls through to `defaultBehavior`
    // (S3) and — because the OAC policy grants `s3:GetObject` and not
    // `s3:ListBucket` — S3 masks the missing key as `403 AccessDenied` XML
    // rather than a 404. That is the same bare AccessDenied page the `/`
    // redirect exists to prevent, at a URL a reviewer reaches by trimming one
    // character off the documented one.
    //
    // ⚠️ **The redirect targets are a fixed list, and must stay one.** The
    // obvious generalisation — redirect any path whose last segment has no file
    // extension — was written first and rejected on review, twice over:
    //
    // - It is an **open redirect**. `request.uri` is attacker-controlled and
    //   CloudFront does not collapse a leading `//`, so `//evil.com/x` would
    //   have produced `Location: //evil.com/x/` — protocol-relative, i.e. a
    //   different origin. A backslash form (`/\evil.com/x`) reaches the same
    //   place through browser normalisation. On the origin that will host task
    //   0186's OAuth callback, that is the standard first link in a
    //   code-interception chain.
    // - It would **fight task 0185's router**. Once the SPA has real routes,
    //   `/api/keys` would 302 to `/api/keys/`, resolve to a key
    //   that does not exist, and leave the address bar permanently rewritten —
    //   so task 0195's per-prefix SPA fallback would have to undo this branch
    //   instead of sitting alongside it.
    //
    // Interpolating nothing into `Location` makes the first impossible by
    // construction rather than by validation, and keeps the function to the job
    // its name claims. A new frontend adds its prefix to the list.
    //
    // Associated with the S3 behaviours only. Attaching it to an API behaviour
    // would rewrite backend calls the moment one ended in a slash — which is
    // the failure mode 0161 flagged, arriving early.
    const directoryIndex = new cloudfront.Function(this, 'DirectoryIndexFn', {
      functionName: `prices-${config.envName}-portal-directory-index`,
      comment: 'Append index.html to directory paths; redirect / to the portal',
      runtime: cloudfront.FunctionRuntime.JS_2_0,
      code: cloudfront.FunctionCode.fromInline(`
function redirect(location) {
  return {
    statusCode: 302,
    statusDescription: 'Found',
    headers: { location: { value: location } },
  };
}

// Every value here is a literal. Nothing from the request is ever interpolated
// into a Location — see the note in the stack above.
var REDIRECTS = {
  // The distribution root has no app of its own yet. Task 0195 gives it one
  // when a second frontend joins; until then the only page here is the portal.
  '/': '/api/',
  '/api': '/api/'
};

// The portal's client-side routes, served by the portal's own index.html.
//
// Without this, a hard refresh or a pasted link to one of them resolves
// against S3, which grants s3:GetObject and NOT s3:ListBucket — so the missing
// key comes back as 403 AccessDenied XML rather than a 404, and the visitor
// gets a bare AWS error page instead of the app. The router cannot help,
// because the bundle never loads.
//
// An ALLOW-LIST of literals, not a catch-all rewrite of every extension-less
// path. A catch-all would answer 200-with-index.html for genuinely missing
// objects too, which turns a broken deploy — a hashed chunk that did not
// upload — into an app that silently renders the wrong thing. It also keeps
// the open-redirect property the stack note above insists on: nothing from the
// request is interpolated into a URI.
//
// Both slash forms, because the trailing-slash branch below would otherwise
// rewrite '/api/login/' to '/api/login/index.html' and 403.
//
// WARNING: add a route to web/portal/src/landing/links.ts and you must add
// it here too. Task 0195 is where this stops being a hand-maintained list and
// becomes the per-prefix SPA fallback.
var APP_ROUTES = ${JSON.stringify(
        Object.fromEntries(
          PORTAL_APP_ROUTES.flatMap((r) => [
            [`/api/${r}`, '/api/index.html'],
            [`/api/${r}/`, '/api/index.html'],
          ]),
        ),
        null,
        2,
      )};

function handler(event) {
  var request = event.request;
  var uri = request.uri;
  // typeof, not truthiness: a bare lookup would also find inherited members,
  // and 'constructor' is truthy.
  if (typeof REDIRECTS[uri] === 'string') {
    return redirect(REDIRECTS[uri]);
  }
  // Before the trailing-slash branch, which would otherwise append index.html
  // to the directory form and miss.
  if (typeof APP_ROUTES[uri] === 'string') {
    request.uri = APP_ROUTES[uri];
    return request;
  }
  if (uri.slice(-1) === '/') {
    request.uri = uri + 'index.html';
    return request;
  }
  return request;
}
`),
    });

    const staticBehaviour = {
      origin: s3Origin,
      viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS,
      functionAssociations: [
        {
          function: directoryIndex,
          eventType: cloudfront.FunctionEventType.VIEWER_REQUEST,
        },
      ],
    };

    // ---------------------------------------------------------------
    // Shared shape for every behaviour that proxies to the API.
    // ---------------------------------------------------------------
    // Three settings here are each load-bearing and each fail silently:
    //
    // - `ALL_VIEWER_EXCEPT_HOST_HEADER`. Two jobs, and task 0186 added the
    //   second. (a) An execute-api endpoint authenticates the request against
    //   its own hostname, so forwarding the viewer's `Host` (which `ALL_VIEWER`
    //   does) turns every API call into a 403 — the single most common way an
    //   API-behind-CloudFront setup is broken. (b) **It forwards cookies**, and
    //   from task 0186 the portal's session IS a cookie. The managed default
    //   (`CachePolicy`'s cookie behaviour, and every origin-request policy other
    //   than the two `ALL_VIEWER*` ones) strips them, so the session would never
    //   reach the origin and `/auth/me` would answer "signed out" to a visitor
    //   who had just signed in — a failure that appears only in a browser, never
    //   in `curl -H`. Task 0184 left this policy alone deliberately, noting it
    //   was only correct to write once there was a session to forward; this is
    //   that moment, and the answer turned out to be that the policy already
    //   chosen for `Host` does the job. `tools/scripts/verify-openapi-routes.mjs`
    //   asserts it against the synthesized template so a "tidy-up" to
    //   `CORS_S3_ORIGIN` or `ALL_VIEWER` cannot pass CI.
    // - `CACHING_DISABLED`. The obvious-looking `CACHING_OPTIMIZED` forwards no
    //   request headers, so `x-api-key` would be stripped — every keyed route
    //   403s — and every caller would collapse onto one cache entry. Edge
    //   caching for the data routes is a separate decision with its own
    //   correctness argument; the gateway already has a stage cache and the
    //   handler already sends `Cache-Control`. For the portal prefix it must
    //   stay off **as a requirement, not as a default**: task 0186 puts a
    //   session cookie through here, and a cached `/auth/me` is one visitor's
    //   identity served to the next. Also asserted in CI.
    // - `ALLOW_ALL` methods. The default (`GET`/`HEAD`) makes CloudFront answer
    //   `POST /v1/prices/batch` with a 403 of its own, never reaching the API.
    //   Task 0186's `POST /auth/logout` needs it too.
    const apiBehaviour = {
      origin: apiOrigin,
      viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS,
      allowedMethods: cloudfront.AllowedMethods.ALLOW_ALL,
      cachePolicy: cloudfront.CachePolicy.CACHING_DISABLED,
      originRequestPolicy:
        cloudfront.OriginRequestPolicy.ALL_VIEWER_EXCEPT_HOST_HEADER,
    };

    // ---------------------------------------------------------------
    // The routing table.
    // ---------------------------------------------------------------
    // Insertion order IS precedence — see the constants at the top of this
    // file: the bundle carve-outs first, then the `/api/*` catch-all to the
    // API. `/api-docs-json` and `/health` are listed individually because both
    // are mounted on the API *root*, not under `/v1`: a table that routes only
    // `/v1/*` sends them to S3, and the Swagger UI that task 0195 adds then
    // loads with a 404 on its spec fetch.
    this.distribution = new cloudfront.Distribution(this, 'Distribution', {
      comment: `prices-${config.envName} portal + API`,
      defaultBehavior: staticBehaviour,
      additionalBehaviors: {
        ...Object.fromEntries(
          PORTAL_BUNDLE_PATHS.map((p) => [p, staticBehaviour]),
        ),
        [PORTAL_BACKEND]: apiBehaviour,
        '/v1/*': apiBehaviour,
        '/api-docs-json': apiBehaviour,
        '/health': apiBehaviour,
      },
      enableLogging: true,
      logBucket,
      logIncludesCookies: false,
    });

    // ---------------------------------------------------------------
    // Upload + invalidate, inside `cdk deploy`.
    // ---------------------------------------------------------------
    // A stale bundle behind a CDN is indistinguishable from a broken deploy, so
    // the invalidation is part of the same operation that uploads rather than a
    // step an operator has to remember. `distributionPaths` is scoped to the
    // portal prefix — the API behaviours are uncached and there is nothing else
    // in the bucket.
    //
    // `cacheControl` is the half `distributionPaths` cannot do. An invalidation
    // clears the EDGE; it cannot reach a viewer's browser. Without a header the
    // objects carry none, the S3 behaviours fall to CDK's default
    // `CACHING_OPTIMIZED` (24 h) and browsers apply heuristic caching on top —
    // so a visitor who opens today's placeholder keeps being told the portal is
    // not available for a day after task 0185 ships the real app, with nothing
    // on screen to explain why. That is the hazard the portal's own `/config`
    // route sets `no-store` to avoid, arriving through the front door instead.
    //
    // **Two deployments, because one `cacheControl` cannot describe both halves
    // of a real bundle** (task 0185 split what task 0184 shipped as one).
    //
    // Vite emits content-hashed assets under `assets/` — `index-DgY606Y7.js` —
    // whose name changes whenever the bytes do. A new build is a new URL, so
    // those can be cached for a year and never revalidated; the entry document
    // is the opposite, an unhashed `index.html` that must be re-fetched or the
    // browser keeps booting last week's app from a URL that still resolves.
    //
    // The `exclude`/`include` pair is what keeps `prune` safe here. Pruning runs
    // as `aws s3 sync --delete` under the same filters, so the entry-document
    // deployment cannot delete the hashed assets it excludes. The asset
    // deployment sets `prune: false` deliberately: a viewer already holding an
    // `index.html` from the previous deploy still requests the old chunk names,
    // and deleting them the moment a new build lands turns an open tab into a
    // blank page. They accumulate; if that ever matters, an S3 lifecycle rule on
    // `api/assets/` is the answer, not pruning.
    //
    // Only the entry-document deployment invalidates. New hashed assets are new
    // URLs and were never in the cache, so invalidating them costs money to
    // clear entries that do not exist.
    const HASHED_ASSETS = 'assets/*';

    const portalBundleAssets = new s3deploy.BucketDeployment(
      this,
      'PortalBundleAssets',
      {
        sources: [s3deploy.Source.asset(PORTAL_ASSET_DIR)],
        destinationBucket: this.bucket,
        destinationKeyPrefix: 'api',
        exclude: ['*'],
        include: [HASHED_ASSETS],
        prune: false,
        cacheControl: [
          s3deploy.CacheControl.setPublic(),
          s3deploy.CacheControl.maxAge(cdk.Duration.days(365)),
          s3deploy.CacheControl.immutable(),
        ],
      },
    );

    // A stale bundle behind a CDN is indistinguishable from a broken deploy, so
    // the invalidation is part of the same operation that uploads rather than a
    // step an operator has to remember.
    //
    // `cacheControl` is the half `distributionPaths` cannot do. An invalidation
    // clears the EDGE; it cannot reach a viewer's browser. Without a header the
    // objects carry none, the S3 behaviours fall to CDK's default
    // `CACHING_OPTIMIZED` (24 h) and browsers apply heuristic caching on top —
    // so a visitor who opened the portal keeps being told it is unavailable for
    // a day after the flag is flipped, with nothing on screen to explain why.
    // That is the hazard the portal's own `/config` route sets `no-store` to
    // avoid, arriving through the front door instead.
    const portalBundle = new s3deploy.BucketDeployment(this, 'PortalBundle', {
      sources: [s3deploy.Source.asset(PORTAL_ASSET_DIR)],
      destinationBucket: this.bucket,
      destinationKeyPrefix: 'api',
      exclude: [HASHED_ASSETS],
      distribution: this.distribution,
      // Enumerated, NOT `/api/*`. An invalidation path list is matched
      // against the cache key and knows nothing about this deployment's
      // `exclude`, so a wildcard here would purge the year-cached hashed assets
      // on every deploy — the exact cost the comment above says this design
      // avoids, and it would have been avoided in the upload only.
      //
      // `/api/` is listed for belt and braces, not because it is known
      // to hold an entry. `DirectoryIndexFn` rewrites it to
      // `/api/index.html` on VIEWER_REQUEST, which is BEFORE the cache
      // lookup, so on the documented behaviour the cache key is only ever the
      // rewritten URI and this path clears nothing. It costs nothing to keep —
      // invalidation is billed per path and the first 1,000 a month are free —
      // and it is the one entry whose absence would be invisible until a
      // viewer was served a stale document. Drop it if task 0205's live deploy
      // confirms the rewrite behaves as documented.
      //
      // Only unhashed objects belong on this list, because only they change
      // bytes while keeping a URL. If a future build emits another one into
      // `web/portal/public/`, add it here — the miss is self-correcting rather
      // than silent (everything this deployment uploads carries
      // `max-age=0, must-revalidate`, so the edge revalidates anyway; the
      // invalidation is what makes the switch immediate instead of merely
      // prompt).
      distributionPaths: ['/api/', '/api/index.html', '/api/favicon.ico'],
      prune: true,
      cacheControl: [
        s3deploy.CacheControl.setPublic(),
        s3deploy.CacheControl.maxAge(cdk.Duration.seconds(0)),
        s3deploy.CacheControl.mustRevalidate(),
      ],
    });

    // ORDERING, not tidiness. `BucketDeployment` creates no implicit dependency
    // between siblings, so without this CloudFormation runs the two custom
    // resources CONCURRENTLY and the entry document can land first.
    //
    // That ordering is a broken deploy, not a slow one: the new `index.html`
    // carries `max-age=0, must-revalidate`, so every viewer refetches it at
    // once, and it references `assets/index-<newhash>.js` which is not in the
    // bucket yet. The bucket policy grants `s3:GetObject` and NOT
    // `s3:ListBucket`, so the miss comes back as `403 AccessDenied` rather than
    // a 404 — the app fails on its own JavaScript and renders nothing. The
    // invalidation fires inside the same window, so the edge is guaranteed to
    // go and fetch the document that cannot work.
    //
    // The reverse order has no such window: the hashed assets are new URLs
    // nobody is asking for yet, so publishing them early is invisible.
    portalBundle.node.addDependency(portalBundleAssets);

    // Task 0186 registers the OAuth redirect URI against this hostname and task
    // 0195 replaces it with the custom domain. Published so neither has to read
    // it out of a CfnOutput by hand.
    new ssm.StringParameter(this, 'PortalDistributionDomainParam', {
      parameterName: `/prices/${config.envName}/portal-distribution-domain`,
      stringValue: this.distribution.distributionDomainName,
      description: `CloudFront domain serving the onboarding portal (${config.envName})`,
    });

    new cdk.CfnOutput(this, 'PortalUrl', {
      value: `https://${this.distribution.distributionDomainName}/api/`,
      description: 'Onboarding portal URL',
    });
    new cdk.CfnOutput(this, 'DistributionDomainName', {
      value: this.distribution.distributionDomainName,
      description: 'CloudFront distribution domain (portal + API)',
    });
    new cdk.CfnOutput(this, 'DistributionId', {
      value: this.distribution.distributionId,
      description: 'CloudFront distribution ID',
    });

    cdk.Tags.of(this).add('Project', 'stellar-prices-api');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
    cdk.Tags.of(this).add('Environment', config.envName);
  }
}
