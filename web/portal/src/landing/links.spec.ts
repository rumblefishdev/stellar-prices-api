import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

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
