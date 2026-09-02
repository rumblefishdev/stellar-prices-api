import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { OPENAPI_JSON } from '../landing/links';
import { ApiReference } from './ApiReference';
import { FIXTURE } from './openapi.fixture';

/**
 * `fetch` is stubbed, not the module: the page's one job is to show what the
 * served document says, so the tests hand it a document and read the page.
 * `FIXTURE` is one of every construct `utoipa` emits for this API.
 */
function stubSpec(response: Partial<Response> & { json?: () => unknown } = {}) {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    status: 200,
    json: async () => FIXTURE,
    ...response,
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

afterEach(() => {
  vi.unstubAllGlobals();
  // `replaceState`, not `location.hash = ''`: clearing a hash that is not
  // there is a navigation to jsdom, and jsdom does not implement those.
  window.history.replaceState(null, '', window.location.pathname);
});

describe('ApiReference', () => {
  it('fetches the live document with a simple request and renders its groups', async () => {
    const fetchMock = stubSpec();
    render(<ApiReference />);

    expect(
      await screen.findByRole('heading', { name: 'Ops', level: 2 }),
    ).toBeTruthy();
    // The served document, not a bundled copy — and a request that needs no
    // preflight: no credentials, no custom header.
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(OPENAPI_JSON);
    expect(init.credentials).toBeUndefined();
    expect(init.headers).toEqual({ accept: 'application/json' });

    // Tags in the document's order, and the one it never declared last.
    const headings = screen
      .getAllByRole('heading', { level: 2 })
      .map((h) => h.textContent);
    expect(headings).toEqual([
      'Base URL & authentication',
      'Ops',
      'Assets',
      'Prices',
      'Schemas',
    ]);
    expect(screen.getByText('v9.9.9')).toBeTruthy();
    expect(screen.getByText('https://api.example')).toBeTruthy();
    expect(screen.getByText('x-api-key: YOUR_API_KEY')).toBeTruthy();
    expect(
      screen.getByRole('link', { name: /openapi json/i }).getAttribute('href'),
    ).toBe(OPENAPI_JSON);
  });

  it('lists every operation as a collapsed row with its method and path', async () => {
    stubSpec();
    render(<ApiReference />);

    const row = await screen.findByRole('button', {
      name: /GET \/v1\/assets/,
    });
    expect(row.getAttribute('aria-expanded')).toBe('false');
    expect(
      screen.getByRole('button', { name: /POST \/v1\/prices\/batch/ }),
    ).toBeTruthy();
    // Keyed routes carry the lock; the anonymous probe does not.
    expect(
      within(row).getByTitle(/requires the x-api-key header/i),
    ).toBeTruthy();
    expect(
      within(screen.getByRole('button', { name: /GET \/health/ })).queryByTitle(
        /requires/i,
      ),
    ).toBeNull();
  });

  it('opens a row to its parameters, allowed values and responses', async () => {
    stubSpec();
    render(<ApiReference />);

    fireEvent.click(
      await screen.findByRole('button', { name: /GET \/v1\/assets/ }),
    );

    const params = screen.getByRole('table', { name: 'Parameters' });
    expect(within(params).getByText('type')).toBeTruthy();
    expect(within(params).getByText('limit')).toBeTruthy();
    // A referenced enum links to its schema and shows its members.
    expect(
      within(params)
        .getByRole('link', { name: 'TypeFilter' })
        .getAttribute('href'),
    ).toBe('#schema-TypeFilter');
    for (const value of ['classic', 'soroban', 'all']) {
      expect(within(params).getByText(value)).toBeTruthy();
    }
    expect(within(params).getByText('integer (int32)')).toBeTruthy();
    expect(within(params).getByText('1 to 200')).toBeTruthy();
    // `query` for both, no `required` chip: neither is required.
    expect(within(params).getAllByText('query')).toHaveLength(2);
    expect(within(params).queryByText('required')).toBeNull();

    // Responses, with the success example built from the schema.
    expect(screen.getByText('200')).toBeTruthy();
    expect(screen.getByText('400')).toBeTruthy();
    expect(screen.getByText('Invalid query parameter')).toBeTruthy();
    expect(
      screen.getByRole('link', { name: 'ErrorEnvelope' }).getAttribute('href'),
    ).toBe('#schema-ErrorEnvelope');
    const example = screen.getByRole('button', {
      name: /copy the 200 example/i,
    });
    expect(example).toBeTruthy();
    expect(screen.getByText(/"asset_code"/)).toBeTruthy();
    expect(screen.getByText(/"USDC"/)).toBeTruthy();
  });

  it('shows a request body with its example', async () => {
    stubSpec();
    render(<ApiReference />);

    fireEvent.click(
      await screen.findByRole('button', { name: /POST \/v1\/prices\/batch/ }),
    );

    expect(screen.getByRole('heading', { name: 'Request body' })).toBeTruthy();
    expect(screen.getByText('application/json')).toBeTruthy();
    expect(screen.getByRole('link', { name: 'BatchRequest' })).toBeTruthy();
    expect(screen.getByText(/"assets"/)).toBeTruthy();
  });

  it('opens a schema to its properties, marking the required ones', async () => {
    stubSpec();
    render(<ApiReference />);

    fireEvent.click(
      await screen.findByRole('button', { name: /^AssetListItem/ }),
    );

    const props = screen.getByRole('table', { name: 'Properties' });
    expect(within(props).getByText('asset_code')).toBeTruthy();
    expect(within(props).getAllByText('required')).toHaveLength(1);
    expect(within(props).getByText('string | null')).toBeTruthy();
    expect(
      within(props)
        .getByRole('link', { name: 'Stream | null' })
        .getAttribute('href'),
    ).toBe('#schema-Stream');
    expect(within(props).getByText('string (date-time)')).toBeTruthy();
    // The description's bullet list, with its wrapped line joined back on.
    const items = within(props)
      .getAllByRole('listitem')
      .map((li) => li.textContent);
    expect(items).toEqual([
      'native — the lumens themselves.',
      'CODE:ISSUER — anything else.',
    ]);
  });

  it('opens the row the URL names, so a link from the rail lands on content', async () => {
    stubSpec();
    window.history.replaceState(null, '', '#schema-TypeFilter');
    render(<ApiReference />);

    const row = await screen.findByRole('button', { name: /^TypeFilter/ });
    expect(row.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('heading', { name: 'Values' })).toBeTruthy();
  });

  it('lists every tag and operation in the rail', async () => {
    stubSpec();
    render(<ApiReference />);

    // The placeholder has an (empty) rail of its own; wait for the document.
    await screen.findByRole('heading', { name: 'Ops', level: 2 });
    const rail = within(
      screen.getByRole('navigation', { name: 'On this page' }),
    );
    expect(
      rail.getByRole('link', { name: 'Assets' }).getAttribute('href'),
    ).toBe('#tag-assets');
    expect(
      rail.getByRole('link', { name: 'GET /v1/assets' }).getAttribute('href'),
    ).toBe('#op-get-v1-assets');
    expect(rail.getByRole('link', { name: 'Schemas' })).toBeTruthy();
  });

  it('says which URL failed, and offers to try again', async () => {
    const fetchMock = stubSpec({ ok: false, status: 503 });
    render(<ApiReference />);

    expect(await screen.findByRole('alert')).toHaveProperty(
      'textContent',
      `${OPENAPI_JSON} answered 503`,
    );
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => FIXTURE,
    });
    fireEvent.click(screen.getByRole('button', { name: /try again/i }));
    await waitFor(() =>
      expect(
        screen.getByRole('heading', { name: 'Ops', level: 2 }),
      ).toBeTruthy(),
    );
  });

  it('treats a non-document answer as a failure, not as an empty reference', async () => {
    stubSpec({ json: async () => ({ enabled: true }) });
    render(<ApiReference />);

    expect(await screen.findByRole('alert')).toHaveProperty(
      'textContent',
      `${OPENAPI_JSON} did not return an OpenAPI document`,
    );
  });
});
