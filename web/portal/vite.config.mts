import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv, type Plugin } from 'vite';

/**
 * `base` is what makes Vite emit asset URLs under `/api/`; without it
 * every `<script src>` and `<link href>` points at the domain root and the app
 * 403s on its own JavaScript the moment it is not served from `/`. The router's
 * `basename` is the other half — this one covers assets, that one covers routes,
 * and both are needed.
 *
 * Declared here rather than imported from `src/base-path.ts`, deliberately: a
 * config that imports application source trips Vite's `configLoader: 'native'`
 * warning on every build, and silencing that would hide a real future breakage.
 * So the value is duplicated and the DUPLICATION IS TESTED instead —
 * `src/base-path.spec.ts` loads this config and asserts `base` still equals
 * `BASE_PATH`. Change one and that test tells you about the other.
 *
 * It is a BUILD setting: CloudFront cannot fix it after the fact, which is why
 * task 0185 puts it in the first commit rather than discovering it on the first
 * deploy.
 *
 * The trailing slash is required. Vite treats a `base` without one as a path
 * prefix to concatenate rather than a directory, which produces
 * `/apiassets/…`.
 */
const BASE_PATH = '/api/';

/**
 * Dev-server proxy targets, mirroring `soroban-block-explorer`'s pattern.
 *
 * The browser only ever talks to localhost, so dev is same-origin exactly like
 * production is — which is the property task 0184 bought by putting the API on
 * the same distribution, and the one that lets task 0186's session cookie be
 * `SameSite=Lax`. Developing against a cross-origin API would hide every CORS
 * and cookie problem until the first deploy.
 *
 * `DEV_API_PROXY_TARGET` should be the **CloudFront distribution**, not the
 * execute-api URL: the distribution serves all four of these paths at exactly
 * the same shape this proxy forwards, whereas execute-api serves them under
 * `/{stage}` and would need a rewrite here that production does not perform.
 */
const PROXIED_PATHS = [
  // The portal's own backend (task 0183's gate, keyless by design). Covers
  // task 0186's `/auth/*` by prefix, which matters for more than reachability:
  // the sign-in round-trip only works locally if the browser sees ONE origin, so
  // that the `SameSite=Lax` session cookie set by the callback is sent back on
  // the next request. A separate backend port would break that in a way no test
  // in this repo can see.
  //
  // ⚠️ For the sign-in round-trip, `DEV_API_PROXY_TARGET` must point at a LOCAL
  // `cargo run --bin serve` (with `PORTAL_ENABLED=true` and a
  // `PORTAL_OAUTH_SECRET_FILE`), not at the CloudFront distribution — production
  // runs with the portal closed, so proxying there gets an empty 404 from the
  // gate. See docs/runbooks/portal-oauth-deploy-prep.md § 4.
  '/api/api',
  // The data routes, for when a portal page needs to show a real API response.
  '/v1',
  '/health',
  '/api-docs-json',
] as const;

/** Only `/v1` requires a key — see the note on `devApiKey` below. */
const KEYED_PATHS: readonly string[] = ['/v1'];

/**
 * Keep `index.html`'s comments out of the shipped document.
 *
 * Vite does not strip them, and this app's entry document is mostly commentary:
 * why there is no `<base href>`, why the favicon is root-relative, which task
 * puts a credential on this page, and how S3 answers a missing key. That is
 * useful to the next person editing the file and it is a free read for anyone
 * who opens view-source on a PUBLIC page whose whole job, from task 0187 on, is
 * to render an API key. None of it is a secret; all of it is unnecessary
 * disclosure of the roadmap and of how the bucket behaves on a miss.
 *
 * Build only, so the source stays readable in `nx dev`. `vite preview` serves
 * the built output, so it sees the stripped document like production does.
 */
export const stripHtmlComments = (): Plugin => ({
  name: 'portal-strip-html-comments',
  apply: 'build',
  transformIndexHtml: {
    // `post`, so this also covers anything Vite or a plugin injected.
    order: 'post',
    // The leading `\n[ \t]*` goes with the comment, or every stripped block
    // leaves a blank indented line behind.
    handler: (html: string) => html.replace(/\n?[ \t]*<!--[\s\S]*?-->/g, ''),
  },
});

export default defineConfig(({ mode }) => {
  // The `''` prefix loads ALL env vars, not just `VITE_`-prefixed ones. That is
  // deliberate and it is the security property of this file: the values below
  // are read HERE, in the Node config, and only `VITE_`-prefixed vars are
  // exposed to `import.meta.env` in the client bundle. A dev key put in
  // `VITE_API_KEY` would be compiled into the JavaScript and served to every
  // visitor — see the acceptance criterion "no API key, no secret and no
  // third-party script in the bundle".
  const env = loadEnv(mode, import.meta.dirname, '');

  const proxyTarget = env['DEV_API_PROXY_TARGET'];
  const devApiKey = env['DEV_API_KEY'];

  // No target configured → no proxy. `web/portal/.env.development` is
  // gitignored; without it the app runs against nothing, which is the correct
  // default for a repo where a checked-in target would be a production URL.
  const proxy = proxyTarget
    ? Object.fromEntries(
        PROXIED_PATHS.map((path) => [
          path,
          {
            target: proxyTarget,
            // Rewrite `Host` so CloudFront routes to the right distribution.
            changeOrigin: true,
            secure: true,
            // Injected SERVER-SIDE, per path, and only where a key is actually
            // required. The portal's own routes are keyless on purpose (a
            // visitor signing in to get a key does not have one yet), so
            // sending a key to `/api/api/*` in dev would test a
            // configuration production never runs.
            ...(devApiKey && KEYED_PATHS.includes(path)
              ? { headers: { 'x-api-key': devApiKey } }
              : {}),
          },
        ]),
      )
    : undefined;

  return {
    root: import.meta.dirname,
    base: BASE_PATH,
    cacheDir: '../../node_modules/.vite/web/portal',
    server: {
      port: 4200,
      host: 'localhost',
      proxy,
    },
    preview: {
      port: 4200,
      host: 'localhost',
      // The SAME proxy as `server`, not an oversight corrected: `vite preview`
      // is the only local way to run the actual built bundle — base-prefixed
      // asset URLs, minified chunks, no dev transforms — and it is therefore the
      // one local check that resembles what CloudFront serves. Without a proxy
      // here it always rendered the "could not reach the portal backend" branch,
      // so the closest thing to a production rehearsal could not exercise the
      // same-origin call that is this slice's whole point.
      proxy,
    },
    plugins: [react(), stripHtmlComments()],
    build: {
      emptyOutDir: true,
      outDir: './dist',
      reportCompressedSize: true,
    },
    test: {
      name: 'portal',
      watch: false,
      globals: true,
      environment: 'jsdom',
      include: ['src/**/*.{test,spec}.{js,mjs,cjs,ts,mts,cts,jsx,tsx}'],
      reporters: ['default'],
      coverage: {
        reportsDirectory: './test-output/vitest/coverage',
        provider: 'v8' as const,
      },
    },
  };
});
