import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv } from 'vite';

/**
 * `base` is what makes Vite emit asset URLs under `/api-tokens/`; without it
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
 * `/api-tokensassets/…`.
 */
const BASE_PATH = '/api-tokens/';

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
  // The portal's own backend (task 0183's gate, keyless by design).
  '/api-tokens/api',
  // The data routes, for when a portal page needs to show a real API response.
  '/v1',
  '/health',
  '/api-docs-json',
] as const;

/** Only `/v1` requires a key — see the note on `devApiKey` below. */
const KEYED_PATHS: readonly string[] = ['/v1'];

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
            // sending a key to `/api-tokens/api/*` in dev would test a
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
    },
    plugins: [react()],
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
