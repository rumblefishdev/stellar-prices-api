# CloudFront changes needed on `EA2TLS5SS5M87` to serve the Prices API portal

**For:** the `soroban-block-explorer` repo — the distribution
`EA2TLS5SS5M87` (alias `sorobanscan.rumblefish.dev`) is defined there.
**From:** `stellar-prices-api`, task 0194 (portal security and ops audit).
**Measured:** 2026-08-31, against the live distribution config and live probes.
**Status of our side:** done. Nothing below requires a change in
`stellar-prices-api`.

---

## What is already true

- The portal's static bundle **is synced** to
  `s3://production-soroban-explorer-api-spa/api/` — 13 objects, and it is the
  right bundle (`<title>Stellar Prices API — API keys</title>`, asset URLs
  `/api/assets/…`, built for the `/api/` prefix).
- The portal's backend **is deployed and open**. Directly against the API:

      GET https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/api/api/config
      → 200  {"enabled":true,"rate_limit_per_second":1}

- The bucket itself passes our security audit: `BLOCK_ALL` on all four
  public-access flags, `IsPublic false`, a single `s3:GetObject` grant to
  `cloudfront.amazonaws.com` conditioned on
  `AWS:SourceArn = …distribution/EA2TLS5SS5M87`, anonymous GET → `403`.

## The symptom

Signed in through basic auth, `https://sorobanscan.rumblefish.dev/api/index.html`
renders the portal shell but shows **no "Sign in with Discord" / "Get my API
key" control**, exactly as if the portal were switched off. It is not off.

## Why — the exact chain

1. The page loads and its JS calls `fetch('/api/api/config')`, a **relative**
   URL. It is relative on purpose: the session is an `HttpOnly`,
   `SameSite=Lax` cookie, so the call must be same-origin or the browser
   withholds it. An absolute URL to execute-api is not an option.
2. `/api/api/config` matches cache behaviour `/api/*`, whose target origin is
   the **S3 bucket**. There is no origin on this distribution pointing at the
   API.
3. S3 has no key `api/api/config`, so it answers `403`.
4. `CustomErrorResponses` maps `403 → /index.html` with response code **`200`**.
5. `/index.html` resolves on the *default* origin — the Explorer SPA bucket.

So the portal asks "are you enabled?" and receives **the Explorer SPA as
`200 text/html`**. Because the status is `200`, the client's `!response.ok`
guard passes and the JSON parse fails instead; the app lands in its
"could not reach the backend" state and hides the controls.

Confirmed independently on a path with no basic auth:
`GET /assets/nie-ma-takiego-pliku-xyz.js` → `200 text/html`, Explorer's
document with Google Tag Manager in it.

---

## Required changes

Items 1–3 are what make the portal work at all. Items 4–6 are required before
we can sign the portal off, and 5 is a distribution-wide correctness issue that
affects the Explorer SPA too.

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
OriginPath:      /production            ← load-bearing, see below
ProtocolPolicy:  https-only
```

**`OriginPath` is not optional.** API Gateway serves a REST API only under
`/{stage}`. Without it every proxied request arrives one path segment short and
the gateway answers `403`. This is the most common way this setup is broken.

### 2. Add a cache behaviour for `/api/api/*`, ORDERED BEFORE `/api/*`

```
PathPattern:          /api/api/*
TargetOriginId:       <the new API origin>
AllowedMethods:       ALL   (GET, HEAD, OPTIONS, PUT, POST, PATCH, DELETE)
CachePolicy:          Managed-CachingDisabled
OriginRequestPolicy:  Managed-AllViewerExceptHostHeader
ViewerProtocolPolicy: redirect-to-https
```

Precedence matters: `/api/api/*` must win over `/api/*`, or backend calls keep
hitting S3. All four settings are load-bearing and each fails silently:

- **`AllViewerExceptHostHeader`** does two jobs. (a) An execute-api endpoint
  authenticates against its own hostname, so forwarding the viewer's `Host`
  (which `AllViewer` does) turns every API call into a `403`. (b) **It forwards
  cookies.** The managed defaults — and every origin-request policy other than
  the two `AllViewer*` ones — strip them, so the session cookie never reaches
  the origin and every signed-in visitor reads as signed out. That failure
  appears only in a browser, never in `curl`.
- **`CachingDisabled`** is a requirement, not a default. A cached `/auth/me` is
  one visitor's identity served to the next. `CachingOptimized` additionally
  forwards no request headers, which strips `x-api-key`.
- **`ALL` methods**: the default `GET`/`HEAD` makes CloudFront answer `POST`
  with a `403` of its own, never reaching the API. Key issue, rework, revoke and
  sign-out are all `POST`.

This mirrors the behaviour already proven on our own distribution
(`dojr4epgxo2qp.cloudfront.net`), where 13 of 13 probe requests reached the
origin and nothing was served from the edge.

### 3. Exempt the portal's backend prefix from basic auth

`production-soroban-explorer-basic-auth` is attached to `/api/*` and currently
answers `401` to everything underneath, including `/api/api/*`. A `fetch` that
receives `401` cannot prompt for credentials, so the portal breaks even for a
visitor who authenticated for the page.

Either do not associate the function with the new `/api/api/*` behaviour, or
have it pass that prefix through. Note the portal has its own authentication
(Discord OAuth + guild-membership gate) and its own per-method throttle at the
gateway, so it is not relying on basic auth for protection.

### 4. Per-prefix SPA fallback for `/api/*`

The portal's routes are `/api/login`, `/api/dashboard`, `/api/quick-start`. A
refresh or deep link on one resolves to a missing S3 key, which today becomes
Explorer's `index.html` at `200` (item 5) — the visitor gets the block explorer.

Also `/api/` alone does not resolve: the bucket holds `api/index.html`, but the
key `api/` is a zero-byte placeholder, and `DefaultRootObject` is a property of
the distribution, not of a behaviour, so it cannot help here.

Our distribution solves this with a small CloudFront Function that appends
`index.html` to a path ending in `/` and redirects the canonical forms. Two
warnings from our review, both found the hard way:

- **Keep the redirect list fixed.** The obvious generalisation — redirect any
  path whose last segment has no file extension — is an **open redirect**:
  `request.uri` is attacker-controlled and CloudFront does not collapse a
  leading `//`, so `//evil.com/x` yields `Location: //evil.com/x/`, a different
  origin. A backslash form reaches the same place via browser normalisation.
  This distribution will host an OAuth callback, where that is the standard
  first link in a code-interception chain. Interpolate nothing into `Location`.
- **Attach it to S3 behaviours only.** On an API behaviour it would rewrite
  backend calls the moment one ended in a slash.

### 5. `CustomErrorResponses` must not rewrite the portal's responses

`403 → /index.html (200)` and `404 → /index.html (200)` are distribution-wide.
For the portal they are actively harmful, and this is no longer hypothetical:

    POST /api/api/key/rework  → 403   (a real refusal, on a live route)

On this distribution that reaches the browser as **Explorer's SPA with status
`200`** — a refusal rendered as success. Every portal error response is JSON and
carries `Cache-Control: no-store`; we verified all of them at the origin on
2026-08-31.

Scoping the error responses so they do not apply to the portal's prefixes is the
ask. It is worth reviewing for the Explorer SPA independently: turning every
origin `403`/`404` into a `200` also hides genuine failures from monitoring.

### 6. Redirect `/api` → `/api/`

`/api` does not match `/api/*`, so it falls to the default behaviour and serves
the Explorer SPA at `200`. A visitor who trims one character off the documented
URL silently gets the wrong application — which is how this whole investigation
started.

---

## How to verify, from outside

With basic-auth credentials, from a browser or `curl -u`:

| probe | expected |
|---|---|
| `GET /api/api/config` | `200`, `application/json`, `{"enabled":true,…}`, `Cache-Control: no-store` |
| `GET /api/api/auth/login` | `303` to `discord.com/oauth2/authorize` |
| `POST /api/api/auth/logout` | `204` (not `403` — proves `ALL` methods) |
| `GET /api/api/key` | `401` JSON (not `200 text/html` — proves error responses are not rewritten) |
| `GET /api/` and `GET /api` | the portal's `index.html`, not Explorer's |
| `GET /api/dashboard` | the portal's `index.html` (SPA fallback) |

The single most diagnostic one is the first: if it returns `text/html`, the
request is still reaching S3 rather than the API.

## Contact

Questions to Adam (this repo). The portal's own configuration — gateway
throttles, IAM, secrets, the `PORTAL_ENABLED` flag — is all deployed and
audited; the evidence lives in
`lore/1-tasks/active/0194_TEST_portal-security-and-ops-audit/`.
