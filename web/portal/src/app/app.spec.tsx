import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ROUTER_BASENAME } from '../base-path';
import App from './app';

/**
 * The gate is the only behaviour in this slice worth testing, and both of its
 * branches matter for different reasons: the closed one is what every visitor
 * sees until task 0194 flips the flag, and the open one is the assertion that
 * flipping it is not a no-op.
 *
 * `fetch` is stubbed rather than the module mocked, so these tests also cover
 * `src/api/portal.ts` — including that the URL is relative, which is the
 * same-origin property task 0184 exists to provide and an absolute base URL
 * would silently undo.
 */
function stubFetch(response: Partial<Response> & { json?: () => unknown }) {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    status: 200,
    json: async () => ({ enabled: false }),
    ...response,
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

const renderApp = () =>
  render(
    <MemoryRouter initialEntries={['/']}>
      <App />
    </MemoryRouter>,
  );

describe('portal home', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('says the portal is unavailable and offers nothing to click while the flag is off', async () => {
    stubFetch({ json: async () => ({ enabled: false }) });
    renderApp();

    expect(await screen.findByText(/not yet available/i)).toBeTruthy();
    // The acceptance criterion is "no sign-in button", not "no button that
    // happens to say sign in" — assert on the role, so any control added here
    // fails this rather than sneaking past a string match.
    expect(screen.queryAllByRole('button')).toHaveLength(0);
    expect(screen.queryAllByRole('link')).toHaveLength(0);
  });

  it('renders the open state when the flag is on', async () => {
    stubFetch({ json: async () => ({ enabled: true }) });
    renderApp();

    expect(await screen.findByText(/portal is open/i)).toBeTruthy();
    expect(screen.queryByText(/not yet available/i)).toBeNull();
  });

  it('calls the backend same-origin, with a relative URL and no API key', async () => {
    const fetchMock = stubFetch({});
    renderApp();

    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api-tokens/api/config');
    // An absolute URL here would reintroduce CORS and break task 0186's
    // SameSite=Lax cookie — both of which fail in a browser, not in curl.
    expect(url.startsWith('http')).toBe(false);
    expect(JSON.stringify(init?.headers ?? {})).not.toMatch(/api-key/i);
  });

  it('surfaces a backend failure instead of spinning forever', async () => {
    stubFetch({ ok: false, status: 404 });
    renderApp();

    expect(
      await screen.findByText(/could not reach the portal backend/i),
    ).toBeTruthy();
    expect(screen.queryByText(/Checking whether/i)).toBeNull();
  });

  // A `200` carrying HTML is what CloudFront returns if the `/api-tokens/api/*`
  // behaviour ever stops winning over `/api-tokens/*` — the regression
  // `portal-hosting-stack.ts` fails CI to prevent. Unwrapped, `response.json()`
  // throws a bare SyntaxError about an unexpected `<`, which names neither the
  // URL nor the status and reads like a bug in this app.
  it('reports a 200 that is not JSON as a backend failure, with the status', async () => {
    stubFetch({
      json: async () => {
        throw new SyntaxError("Unexpected token '<'");
      },
    });
    renderApp();

    expect(
      await screen.findByText(/could not reach the portal backend/i),
    ).toBeTruthy();
    // The reason must name the URL and carry the status, which a bare
    // SyntaxError does neither of.
    expect(
      await screen.findByText(
        /\/api-tokens\/api\/config answered 200, not JSON/i,
      ),
    ).toBeTruthy();
  });

  // `fetch` has no default timeout, and nothing else bounds a connection that
  // never reaches an origin — the gateway's 29s cap only applies once a request
  // gets there. Without a signal a stalled handshake leaves the page on
  // "Checking whether the portal is open…" forever, which is the spinner the
  // failure branch exists to avoid.
  it('gives the probe a timeout rather than waiting on the network forever', async () => {
    const fetchMock = stubFetch({});
    renderApp();

    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(init?.signal).toBeInstanceOf(AbortSignal);
  });

  it('reports a hung backend as a timeout, naming the URL', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockRejectedValue(
          new DOMException('The operation was aborted.', 'TimeoutError'),
        ),
    );
    renderApp();

    expect(
      await screen.findByText(/could not reach the portal backend/i),
    ).toBeTruthy();
    // Distinct from "could not be reached": a timeout means something accepted
    // the connection and then said nothing, which points at the gateway or the
    // origin rather than at the viewer's own network.
    expect(
      await screen.findByText(
        /\/api-tokens\/api\/config did not answer within 10s/i,
      ),
    ).toBeTruthy();
  });

  it('says nothing about the outcome while the probe is still in flight', async () => {
    stubFetch({});
    renderApp();

    // The evidence paragraph must not claim failure before there is an answer.
    expect(
      screen.getByText(/Checking whether the portal is open/i),
    ).toBeTruthy();
    expect(screen.queryByText(/unsuccessfully/i)).toBeNull();

    // …and it must still report the outcome once one arrives.
    expect(await screen.findByText(/successfully/i)).toBeTruthy();
  });

  // `renderApp` above mounts at `/` with no basename, so it cannot notice
  // `ROUTER_BASENAME` being dropped from `main.tsx` — and that would render an
  // empty page on the one URL this app is served from. Mount it the way
  // production does instead.
  it('renders its route when mounted under the production basename', async () => {
    stubFetch({});
    render(
      <MemoryRouter
        basename={ROUTER_BASENAME}
        initialEntries={[`${ROUTER_BASENAME}/`]}
      >
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText(/not yet available/i)).toBeTruthy();
    expect(screen.getByRole('heading', { level: 1 })).toBeTruthy();
  });
});
