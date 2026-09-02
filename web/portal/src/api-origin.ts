/**
 * Where the portal's backend is — the API's own hostname, or nothing.
 *
 * Empty by default, and then every backend URL in this app is **relative**:
 * `/api/config`, `/api/auth/login`. That is the same-origin layout — the dev
 * server's proxy and `vite preview`, which forward `/api/*` to the backend.
 * (It was also the layout of our own CloudFront distribution, where the API
 * was an origin on the same table as the bundle; task 0195 retired that
 * distribution, so production only ever runs the absolute form below.)
 *
 * On the shared host (task 0194) it is not: `sorobanscan.rumblefish.dev/api/*`
 * is a static SPA behaviour that rewrites every extensionless path to
 * `/api/index.html`, so a relative `fetch('/api/config')` there gets this
 * bundle back as `200 text/html`. That deployment is built with
 *
 *     VITE_PORTAL_API_ORIGIN=https://prices-api.sorobanscan.rumblefish.dev
 *
 * and every backend URL becomes absolute, cross-origin and same-site. The
 * three things that layout needs are all on the backend's side — one
 * allowed origin in its CORS answer, `credentials: 'include'` honoured, and a
 * sign-in that lands back on this page's origin — and it is the backend's
 * `PORTAL_WEB_ORIGIN` that names this page. The two values are a pair:
 * `infra/envs/production.json` holds both (`apiDomain.domainName`,
 * `portalWebOrigin`), and `make -C infra sync-portal-explorer` is the one
 * build that sets this variable.
 *
 * A BUILD-time value, like `BASE_PATH`: the bundle is public and cached, and
 * which backend it talks to is not something to read from the page it is on.
 * Trailing slash stripped so `${API_ORIGIN}/api` cannot become `//api`.
 */
export const API_ORIGIN: string = (
  import.meta.env.VITE_PORTAL_API_ORIGIN ?? ''
).replace(/\/+$/, '');
