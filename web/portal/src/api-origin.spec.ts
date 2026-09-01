import { describe, expect, it, vi } from 'vitest';

/** The module reads the variable at import, so each case re-imports it. */
async function apiOriginWith(value: string | undefined): Promise<string> {
  vi.resetModules();
  if (value === undefined) {
    vi.stubEnv('VITE_PORTAL_API_ORIGIN', '');
  } else {
    vi.stubEnv('VITE_PORTAL_API_ORIGIN', value);
  }
  try {
    return (await import('./api-origin')).API_ORIGIN;
  } finally {
    vi.unstubAllEnvs();
  }
}

describe('API_ORIGIN', () => {
  it('is empty — relative, same-origin URLs — unless the build says otherwise', async () => {
    expect(await apiOriginWith(undefined)).toBe('');
  });

  it('is the configured origin with any trailing slash removed', async () => {
    expect(
      await apiOriginWith('https://prices-api.sorobanscan.rumblefish.dev'),
    ).toBe('https://prices-api.sorobanscan.rumblefish.dev');
    expect(
      await apiOriginWith('https://prices-api.sorobanscan.rumblefish.dev/'),
    ).toBe('https://prices-api.sorobanscan.rumblefish.dev');
  });
});
