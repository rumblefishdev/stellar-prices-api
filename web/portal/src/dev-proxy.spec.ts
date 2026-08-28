import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import viteConfig from '../vite.config.mts';

/**
 * The dev proxy carries this app's one security property and had no coverage:
 * the API key is read in the Node config and injected server-side, per path, so
 * it never reaches the bundle. A refactor that moved it into `define`, or into a
 * `VITE_`-prefixed variable, would compile a credential into the JavaScript
 * served to every visitor and no test would have noticed.
 *
 * `loadEnv(mode, dir, '')` reads `process.env` as well as the `.env` files, which
 * is what lets these tests set a target without writing one to disk.
 */
type ProxyEntry = { target?: string; headers?: Record<string, string> };
type ResolvedConfig = {
  server?: { proxy?: Record<string, ProxyEntry> };
  preview?: { proxy?: Record<string, ProxyEntry> };
};

const PROXY_TARGET = 'https://portal.example.invalid';

const resolve = (): ResolvedConfig => {
  const config =
    typeof viteConfig === 'function'
      ? viteConfig({ command: 'serve', mode: 'development' })
      : viteConfig;
  expect(config).not.toBeInstanceOf(Promise);
  return config as ResolvedConfig;
};

describe('dev proxy', () => {
  beforeEach(() => {
    process.env['DEV_API_PROXY_TARGET'] = PROXY_TARGET;
    process.env['DEV_API_KEY'] = 'test-key-must-not-leak';
  });
  afterEach(() => {
    delete process.env['DEV_API_PROXY_TARGET'];
    delete process.env['DEV_API_KEY'];
  });

  it('proxies the portal backend from both the dev server and preview', () => {
    const { server, preview } = resolve();

    // `preview` is the only local way to run the BUILT bundle, so it is the
    // closest thing to a production rehearsal; without a proxy it could only
    // ever render the "could not reach the portal backend" branch.
    expect(server?.proxy?.['/api/api']?.target).toBe(PROXY_TARGET);
    expect(preview?.proxy?.['/api/api']?.target).toBe(PROXY_TARGET);
    expect(preview?.proxy).toEqual(server?.proxy);
  });

  it('sends the key to /v1 only, never to the portal routes', () => {
    const proxy = resolve().server?.proxy ?? {};

    // A visitor signing in to obtain a key does not have one yet, so keying the
    // portal's own routes in dev would exercise a configuration production never
    // runs — and would hide a 403 that only appears for real users.
    expect(proxy['/v1']?.headers?.['x-api-key']).toBe('test-key-must-not-leak');
    expect(proxy['/api/api']?.headers).toBeUndefined();
  });

  // NOT tested here: "no target configured → no proxy". `loadEnv` reads
  // `.env.development` from disk as well as `process.env`, and that file is
  // gitignored — so the assertion passes in CI, where it does not exist, and
  // fails on the machine of anyone who has actually set up a dev proxy. A test
  // whose result depends on untracked local files is worse than no test. The
  // `process.env` values above win over the file, which is why the two tests
  // that do run are deterministic either way.
});
