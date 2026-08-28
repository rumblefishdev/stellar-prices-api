import { describe, expect, it } from 'vitest';

// `.mts` spelled out: TypeScript's bundler resolution does not substitute that
// extension for an extensionless path, so `allowImportingTsExtensions` in
// tsconfig.spec.json is what makes this import resolve. Importing the real
// config is the point of the third test below — a copy of its value would prove
// nothing about what the build actually uses.
import viteConfig from '../vite.config.mts';
import { BASE_PATH, ROUTER_BASENAME } from './base-path';

/**
 * The base path is the one thing in this slice that is expensive to retrofit and
 * free to get right on the first commit, and until these tests existed nothing
 * exercised it: `app.spec.tsx` mounts a `MemoryRouter` with no basename, so
 * deleting `basename` from `main.tsx` or `base` from the Vite config left every
 * test green while the deployed app rendered an empty page at the only URL it is
 * served from.
 *
 * These assert the two halves and, more importantly, that they still agree.
 */
describe('base path', () => {
  it('gives Vite a trailing slash and the router none', () => {
    // Vite concatenates `base` without adding a separator, so a missing slash
    // emits `/apiassets/…`; react-router strips a trailing one and warns.
    expect(BASE_PATH).toBe('/api/');
    expect(ROUTER_BASENAME).toBe('/api');
    expect(BASE_PATH.endsWith('/')).toBe(true);
    expect(ROUTER_BASENAME.endsWith('/')).toBe(false);
  });

  it('keeps both halves describing the same prefix', () => {
    // The guard that matters: the pair may differ ONLY by the trailing slash.
    expect(`${ROUTER_BASENAME}/`).toBe(BASE_PATH);
  });

  it('is what the Vite build actually uses', () => {
    // Asserting the constant alone would not notice `base` being dropped from
    // the returned config, which is the half CloudFront cannot fix afterwards.
    // `defineConfig` with a function argument returns that function; call it the
    // way Vite does for a production build.
    const resolved =
      typeof viteConfig === 'function'
        ? viteConfig({ command: 'build', mode: 'production' })
        : viteConfig;

    expect(resolved).not.toBeInstanceOf(Promise);
    expect((resolved as { base?: string }).base).toBe(BASE_PATH);
  });
});
