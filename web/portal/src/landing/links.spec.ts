import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it, vi } from 'vitest';

import { PUBLIC_API_BASE_URL } from './links';

/**
 * The base URL the snippets render and the base URL the OpenAPI document
 * advertises come from two files that nothing else ties together. This does.
 */
describe('PUBLIC_API_BASE_URL', () => {
  it('is the deployed apiBaseUrl, versioned', () => {
    const config = JSON.parse(
      readFileSync(
        join(import.meta.dirname, '../../../../infra/envs/production.json'),
        'utf8',
      ),
    ) as { apiBaseUrl: string };
    expect(PUBLIC_API_BASE_URL).toBe(`${config.apiBaseUrl}/v1`);
  });

  it('is an https URL with no trailing slash', () => {
    expect(PUBLIC_API_BASE_URL).toMatch(/^https:\/\/[^/]+\/v1$/);
  });
});

/**
 * The defect this guards against is invisible in dev, which is where it would
 * be tested by hand: there `API_ORIGIN` is empty and every href is
 * same-origin, so a `download` on an `API_ORIGIN`-prefixed link saves a file
 * locally and silently stops doing so on the shared host, where the browser
 * ignores `download` cross-origin and navigates instead. So the case sets
 * `API_ORIGIN` to what the shared-host build sets and re-imports.
 */
describe('OPENAPI_JSON_DOWNLOAD', () => {
  const SHARED_HOST_ORIGIN = 'https://prices-api.sorobanscan.rumblefish.dev';

  async function linksWith(apiOrigin: string) {
    vi.resetModules();
    vi.stubEnv('VITE_PORTAL_API_ORIGIN', apiOrigin);
    try {
      return await import('./links');
    } finally {
      vi.unstubAllEnvs();
    }
  }

  it('is a bundle-relative path under the app base, with a .json extension', async () => {
    const { OPENAPI_JSON_DOWNLOAD } = await linksWith('');
    expect(OPENAPI_JSON_DOWNLOAD).toBe('/api/openapi.json');
  });

  it('does not move when the build points the app at a cross-origin API', async () => {
    const { OPENAPI_JSON, OPENAPI_JSON_DOWNLOAD } =
      await linksWith(SHARED_HOST_ORIGIN);
    // The fetched document follows API_ORIGIN...
    expect(OPENAPI_JSON).toBe(`${SHARED_HOST_ORIGIN}/api-docs-json`);
    // ...the downloaded one must not, or `download` is ignored.
    expect(OPENAPI_JSON_DOWNLOAD).toBe('/api/openapi.json');
    expect(OPENAPI_JSON_DOWNLOAD).not.toContain(SHARED_HOST_ORIGIN);
    expect(OPENAPI_JSON_DOWNLOAD).not.toMatch(/^[a-z]+:\/\//);
  });

  it('names the API in the saved file', async () => {
    const { OPENAPI_JSON_FILENAME } = await linksWith('');
    expect(OPENAPI_JSON_FILENAME).toBe('stellar-prices-api-openapi.json');
  });
});

/**
 * The file the link points at is a committed copy of the extracted document.
 * CI diffs it against `npm run openapi:extract`; this only asserts it is
 * present and parses, so a bundle that ships a 404 fails here rather than in
 * a reader's Downloads folder.
 */
describe('the bundled OpenAPI document', () => {
  it('exists in public/ and is an OpenAPI document', () => {
    const doc = JSON.parse(
      readFileSync(
        join(import.meta.dirname, '../../public/openapi.json'),
        'utf8',
      ),
    ) as { openapi?: string; paths?: Record<string, unknown> };
    expect(doc.openapi).toMatch(/^3\./);
    expect(Object.keys(doc.paths ?? {}).length).toBeGreaterThan(0);
  });
});
