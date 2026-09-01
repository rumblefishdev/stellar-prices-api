> ⚠️ **Superseded 2026-08-31, later the same day — decision A in the task
> README ("The backend on its own host").** The Prices API now has a hostname
> of its own, `prices-api.sorobanscan.rumblefish.dev`, and the bundle at
> `/api/` calls it directly (cross-origin, same-site, CORS answered by the
> API). So `EA2TLS5SS5M87` needs **no API origin, no `/api/*` method change,
> no `CustomErrorResponses` change** — the static `/api/*` SPA behaviour PR
> #437 built is exactly right as it is. The one item that still belongs to the
> explorer repo is the basic-auth gate on `/api/*` (`enableApiSpaBasicAuth`),
> which has to be off before the portal is public. Kept as the record of what
> was measured and asked for.

# CloudFront changes needed on `EA2TLS5SS5M87` to serve the Prices API portal

**For:** the `soroban-block-explorer` repo — the distribution
`EA2TLS5SS5M87` (alias `sorobanscan.rumblefish.dev`) is
`Distribution830FAC52` in your stack `Explorer-production-Delivery`, and so
are the basic-auth function and the `api-spa` bucket. Nothing below can be
done from our side: CloudFormation will not let another stack touch these, and
a console edit would be reverted by your next `cdk deploy`.
**From:** `stellar-prices-api`, task 0194 (portal security and ops audit).
**Measured:** 2026-08-31, against the live distribution config and live probes.
**Layout:** revised the same day — the portal's backend no longer has a
sub-prefix. If you saw an earlier version of this document asking for
`/api/api/*`, this one replaces it.

---

## The layout, in one rule

**`/api/` is the whole self-service portal. Nothing of ours lives anywhere else
on this host.**

| path | what | origin |
|---|---|---|
| `/api/` | the portal page | S3 `production-soroban-explorer-api-spa`, key `api/index.html` |
| `/api/index.html`, `/api/favicon.ico`, `/api/assets/*` | the bundle's files | S3, same bucket |
| `/api/login`, `/api/dashboard`, `/api/quick-start` (with or without `/`) | the app's client-side routes — served as `api/index.html` | S3, same bucket |
| **everything else under `/api/*`** | the portal's backend: `/api/config`, `/api/auth/*`, `/api/key`, `/api/key/rework`, `/api/usage`, `/api/api-docs-json` | **API Gateway** `02mabge71l.execute-api.eu-central-1.amazonaws.com`, stage `/production` |

The bundle and the backend share the prefix, so the split is made by a short
fixed list of bundle paths carved out to S3 **ahead of** an `/api/*` catch-all
that goes to the API. The list is the whole first three rows; it changes only
when the app gains a page.

## What is already true

- The bundle **is synced** to `s3://production-soroban-explorer-api-spa/api/`
  and it is the right one (`<title>Stellar Prices API — API keys</title>`,
  asset URLs `/api/assets/…`). We re-sync it after our own deploy of this
  layout; the S3 keys do not change.
- The backend **is deployed and open**, at the paths above. Directly against
  the API:

      GET https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/api/config
      → 200  {"enabled":true,"rate_limit_per_second":1}

- The bucket passes our security audit: `BLOCK_ALL`, `IsPublic false`, a single
  `s3:GetObject` grant to `cloudfront.amazonaws.com` conditioned on
  `AWS:SourceArn = …distribution/EA2TLS5SS5M87`, anonymous GET → `403`.

## The symptom this fixes

Signed in through basic auth, `https://sorobanscan.rumblefish.dev/api/index.html`
renders the portal shell with **no "Sign in with Discord" / "Get my API key"
control**, exactly as if the portal were switched off. It is not off: the
page's `fetch('/api/config')` matches your `/api/*` row, which targets **S3**;
S3 has no such key and answers `403`; `CustomErrorResponses` turns that into
`/index.html` at **`200`** from the *default* origin — the Explorer SPA. The
portal asks "are you enabled?" and receives the block explorer as
`200 text/html`. Because the status is `200` the client's `!response.ok` guard
passes and the JSON parse fails; the app hides its controls.

The fetch is a relative URL on purpose: the session is an `HttpOnly`,
`SameSite=Lax` cookie, so the backend must be same-origin with the page. That
is why the API has to be an origin on *this* distribution rather than called
directly.

---

## Required changes

### Current configuration, for reference

| | |
|---|---|
| Origin 1 (default) | `production-soroban-explorer-spa.s3.eu-central-1.amazonaws.com` |
| Origin 2 | `production-soroban-explorer-api-spa.s3.eu-central-1.amazonaws.com` |
| Behaviour 0 | `/assets/*` → Origin 1, `GET`/`HEAD` |
| Behaviour 1 | `/static/*` → Origin 1, `GET`/`HEAD` |
| Behaviour 2 | `/api/*` → Origin 2, `GET`/`HEAD`, function `production-soroban-explorer-basic-auth` |
| Default | → Origin 1 |
| `CustomErrorResponses` | `403 → /index.html` as `200`; `404 → /index.html` as `200` |

### 1. Add a third origin — the API

```
DomainName:      02mabge71l.execute-api.eu-central-1.amazonaws.com
OriginPath:      /production            ← load-bearing
ProtocolPolicy:  https-only
```

**`OriginPath` is not optional.** API Gateway serves a REST API only under
`/{stage}`. Without it every proxied request arrives one path segment short and
the gateway answers `403`.

### 2. Carve the bundle out to S3 — ten rows, listed BEFORE `/api/*`

All to Origin 2 (the `api-spa` bucket), `GET`/`HEAD`, cached as you cache the
Explorer SPA. Keep basic auth on these if you want the *page* gated; the
backend has its own authentication.

```
/api/
/api/index.html
/api/favicon.ico
/api/assets/*
/api/login        /api/login/
/api/dashboard    /api/dashboard/
/api/quick-start  /api/quick-start/
```

Both slash forms, because path patterns are literal and a viewer-request
function runs only *after* a behaviour has matched — it cannot re-route.

These rows need one small **viewer-request CloudFront Function** to resolve to
a key S3 actually holds (`api/` is a zero-byte placeholder, and
`DefaultRootObject` applies to `/` only):

```js
// Every value is a literal. Nothing from the request is ever interpolated
// into a Location or a URI — see the warning below.
var REDIRECTS = { '/api': '/api/' };
var INDEX = '/api/index.html';
var APP_ROUTES = {
  '/api/login': INDEX, '/api/login/': INDEX,
  '/api/dashboard': INDEX, '/api/dashboard/': INDEX,
  '/api/quick-start': INDEX, '/api/quick-start/': INDEX
};
function handler(event) {
  var request = event.request;
  var uri = request.uri;
  if (typeof REDIRECTS[uri] === 'string') {
    return { statusCode: 302, statusDescription: 'Found',
             headers: { location: { value: REDIRECTS[uri] } } };
  }
  if (typeof APP_ROUTES[uri] === 'string') { request.uri = APP_ROUTES[uri]; return request; }
  if (uri.slice(-1) === '/') { request.uri = uri + 'index.html'; return request; }
  return request;
}
```

(`typeof … === 'string'` rather than truthiness: a bare lookup also finds
inherited members, and `'constructor'` is truthy.)

⚠️ **Keep the lists fixed; do not generalise to "any path without an
extension".** That generalisation is an **open redirect**: `request.uri` is
attacker-controlled and CloudFront does not collapse a leading `//`, so
`//evil.com/x` yields `Location: //evil.com/x/` — a different origin — and a
backslash form reaches the same place through browser normalisation. This
distribution hosts an OAuth callback, where that is the standard first link in
a code-interception chain. Our own function was rewritten twice on review for
exactly this; the version above interpolates nothing.

### 3. Re-point `/api/*` at the API — the catch-all

```
PathPattern:          /api/*             (unchanged pattern, new target)
TargetOriginId:       <the new API origin>
AllowedMethods:       ALL
CachePolicy:          Managed-CachingDisabled
OriginRequestPolicy:  Managed-AllViewerExceptHostHeader
ViewerProtocolPolicy: redirect-to-https
FunctionAssociations: NONE — remove the basic-auth function from this row
```

Each setting is load-bearing and fails silently when wrong:

- **`AllViewerExceptHostHeader`** — (a) execute-api authenticates against its
  own hostname, so forwarding the viewer's `Host` turns every call into a
  `403`; (b) **it forwards cookies** — every other managed policy strips them,
  the session is a cookie, and without it every signed-in visitor reads as
  signed out. Fails only in a browser, never in `curl`.
- **`CachingDisabled`** — a cached `/auth/me` is one visitor's identity served
  to the next.
- **`ALL` methods** — the default `GET`/`HEAD` makes CloudFront answer `POST`
  with its own `403`. Key issue, rework and sign-out are `POST`s.
- **No basic-auth function** — a `fetch()` that receives `401` cannot prompt
  for credentials, so the portal breaks even for a visitor who authenticated
  for the page. The backend has Discord OAuth, a guild-membership gate and its
  own 10 req/s throttle at the gateway.

This mirrors the configuration proven on our own distribution
(`dojr4epgxo2qp.cloudfront.net`), where the same layout serves the portal
today.

### 4. `CustomErrorResponses` must not rewrite the portal's responses

`403 → /index.html (200)` and `404 → /index.html (200)` are distribution-wide,
and they now apply to the API row too. Concretely:

    POST /api/key/rework  → 403   (a real refusal, on a live route)

reaches the browser as **Explorer's SPA with status `200`** — a refusal
rendered as success. Every portal error response is JSON with
`Cache-Control: no-store`.

Custom error responses cannot be scoped per behaviour, so the ask is to remove
the `403`/`404` mappings and give the Explorer SPA its own fallback the way
item 2 does for ours (a viewer-request function on its S3 rows). That is worth
doing for the Explorer SPA independently: turning every origin `403`/`404` into
a `200` also hides genuine failures from monitoring.

---

## Ordering, and what goes wrong when it is wrong

CloudFront takes the **first** matching behaviour. The ten carve-out rows must
be listed before `/api/*`. If one is missing or listed after, that path reaches
the API and gets a JSON `404` — loud, and visible in the browser's network tab.
That asymmetry is deliberate: the old shape (backend rows ahead of a bundle
catch-all) failed the other way round, with the silent `200 text/html` this
document opened with.

## How to verify, from outside

With basic-auth credentials where you keep them:

| probe | expected |
|---|---|
| `GET /api/config` | `200`, `application/json`, `{"enabled":true,…}`, `Cache-Control: no-store` |
| `GET /api/api-docs-json` | `200`, `application/json`, the OpenAPI document |
| `GET /api/auth/login` | `303` to `discord.com/oauth2/authorize` |
| `POST /api/auth/logout` | `204` — proves `ALL` methods |
| `GET /api/key` | `401` **JSON** — proves error responses are not rewritten |
| `GET /api/`, `GET /api` | the portal's `index.html` (`/api` via a `302`) |
| `GET /api/dashboard` | the portal's `index.html` — the carve-out |
| `GET /api/assets/<any built file>` | `200` from S3 |

The single most diagnostic one is the first: `text/html` means the request is
still reaching S3.

```
curl -u <basic-auth> https://sorobanscan.rumblefish.dev/api/config
```

- `200` + JSON → done, the button appears.
- `200` + `text/html` → `/api/*` still targets S3.
- `401` → the basic-auth function is still on the `/api/*` row.
- `403` → `originPath` missing or wrong.
- `404` JSON → the API row works; a carve-out is missing for that path.

## Contact

Questions to Adam (this repo). The portal's own side — gateway resource
`/api/{proxy+}` with its throttles, IAM, secrets, `PORTAL_ENABLED` — is
deployed and audited; evidence in
`lore/1-tasks/active/0194_TEST_portal-security-and-ops-audit/`.

---

## CDK patch, in your idiom

Your distribution is the L2 `cloudfront.Distribution` (`Distribution830FAC52`
in `Explorer-production-Delivery`, with `BasicAuthFunction3DE306AB` beside it).
Names of your existing variables are guesses — `apiSpaOrigin` is whatever holds
the `production-soroban-explorer-api-spa` origin, `basicAuthFn` the function.

```ts
import * as cloudfront from 'aws-cdk-lib/aws-cloudfront';
import * as origins from 'aws-cdk-lib/aws-cloudfront-origins';

// 1. The Prices API as an origin. `originPath` is load-bearing.
const pricesApiOrigin = new origins.HttpOrigin(
  '02mabge71l.execute-api.eu-central-1.amazonaws.com',
  { originPath: '/production', protocolPolicy: cloudfront.OriginProtocolPolicy.HTTPS_ONLY },
);

// 2. The index/route rewrite for the bundle rows — the function body is in
//    the section above. Viewer request, S3 rows only.
const portalIndexFn = new cloudfront.Function(this, 'PortalIndexFn', {
  runtime: cloudfront.FunctionRuntime.JS_2_0,
  code: cloudfront.FunctionCode.fromInline(PORTAL_INDEX_FN_SOURCE),
});

// Bundle rows: S3, GET/HEAD, cached like the Explorer SPA. Keep basicAuthFn
// here if you want the PAGE gated; it must not be on the API row.
const portalBundle: cloudfront.BehaviorOptions = {
  origin: apiSpaOrigin,
  viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS,
  functionAssociations: [
    { function: portalIndexFn, eventType: cloudfront.FunctionEventType.VIEWER_REQUEST },
    // { function: basicAuthFn, eventType: cloudfront.FunctionEventType.VIEWER_REQUEST },
  ],
};

// 3. The API row. All four settings load-bearing; NO basic-auth function.
const portalBackend: cloudfront.BehaviorOptions = {
  origin: pricesApiOrigin,
  viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS,
  allowedMethods: cloudfront.AllowedMethods.ALLOW_ALL,
  cachePolicy: cloudfront.CachePolicy.CACHING_DISABLED,
  originRequestPolicy: cloudfront.OriginRequestPolicy.ALL_VIEWER_EXCEPT_HOST_HEADER,
};

const PORTAL_APP_ROUTES = ['login', 'dashboard', 'quick-start'];
const PORTAL_BUNDLE_PATHS = [
  '/api/', '/api/index.html', '/api/favicon.ico', '/api/assets/*',
  ...PORTAL_APP_ROUTES.flatMap((r) => [`/api/${r}`, `/api/${r}/`]),
];

// Insertion order IS precedence: bundle rows first, then the /api/* catch-all.
additionalBehaviors: {
  '/assets/*': /* unchanged */,
  '/static/*': /* unchanged */,
  ...Object.fromEntries(PORTAL_BUNDLE_PATHS.map((p) => [p, portalBundle])),
  '/api/*': portalBackend,          // ← was: api-spa origin + basic auth
},
```

`/api` (no slash) is handled by the same function IF it runs on the row that
`/api` falls into — which is your default behaviour. Either attach
`portalIndexFn` to the default behaviour too (it only acts on `/api`; every
other URI passes through untouched), or add a literal `/api` row to the bundle
set. The former is what we do.

Then `cdk diff` — expect: one new origin, eleven behaviour changes, one new
function, and NO change to `/assets/*`, `/static/*` or the default — and
deploy. Verify with the table above; the first probe (`/api/config` as JSON)
is the one that matters.
