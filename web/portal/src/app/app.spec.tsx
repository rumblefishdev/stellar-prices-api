import { fireEvent, render, screen, waitFor } from '@testing-library/react';
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

/**
 * Route-aware stub, for the open-portal states (task 0186).
 *
 * The page makes two calls once the flag is on — `/config` and `/auth/me` — so
 * a single blanket response would answer one of them with the other's body and
 * the test would be asserting on a coincidence. Everything is keyed on the URL
 * instead, which also means each test states which endpoint it is exercising.
 */
function stubRoutes(
  routes: Record<string, () => Partial<Response> & { json?: () => unknown }>,
) {
  // `init` is declared even though the stub ignores it: the tests assert on it
  // (that sign-out is a POST, that nothing carries a key), and without the
  // parameter the recorded call tuple has length 1 and `call[1]` will not
  // typecheck.
  const fetchMock = vi.fn((url: string, init?: RequestInit) => {
    void init;
    const handler = routes[url];
    if (!handler) {
      return Promise.reject(new Error(`unexpected request to ${url}`));
    }
    const { ok = true, status = 200, json = async () => ({}) } = handler();
    return Promise.resolve({ ok, status, json });
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

const CONFIG_URL = '/api-tokens/api/config';
const ME_URL = '/api-tokens/api/auth/me';
const LOGOUT_URL = '/api-tokens/api/auth/logout';

/** The portal open, and nobody signed in. */
const openAndSignedOut = () =>
  stubRoutes({
    [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
    [ME_URL]: () => ({ json: async () => ({ authenticated: false }) }),
  });

/** The portal open, with a completed round-trip behind it. */
const openAndSignedIn = () =>
  stubRoutes({
    [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
    [ME_URL]: () => ({
      json: async () => ({
        authenticated: true,
        user_id: '308994132968210433',
        username: 'adam',
      }),
    }),
  });

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
    openAndSignedOut();
    renderApp();

    // Task 0185's "sign-in arrives with the next slice" placeholder is gone —
    // this slice IS that sign-in, so the open state is now the real control.
    expect(
      await screen.findByRole('link', { name: /sign in with discord/i }),
    ).toBeTruthy();
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

/**
 * Sign in with Discord (task 0186).
 *
 * The OAuth round-trip itself is the backend's (`tests/portal_auth.rs` drives it
 * end to end against a mock Discord). What can only be asserted here is what the
 * page does with the answer: that the control is a link and not a `fetch`, that
 * the signed-in state shows both the username and the ID, and that "signed out"
 * and "cancelled" are plain text rather than screens.
 */
describe('sign in with Discord', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const renderAt = (entry: string) =>
    render(
      <MemoryRouter initialEntries={[entry]}>
        <App />
      </MemoryRouter>,
    );

  /**
   * The control has to be an anchor. A button with an `onClick` that `fetch`es
   * `/auth/login` would follow the 303 to discord.com as a cross-origin
   * request — no consent screen the visitor can see, and no way back. This is
   * also why the session cookie is `SameSite=Lax` and not `Strict`.
   */
  it('offers sign-in as a same-origin link, not a fetch', async () => {
    openAndSignedOut();
    renderAt('/');

    const link = await screen.findByRole('link', {
      name: /sign in with discord/i,
    });
    expect(link.getAttribute('href')).toBe('/api-tokens/api/auth/login');
    // Relative. An absolute URL would make the whole flow cross-site and the
    // Lax cookie would never be sent back to the callback.
    expect(link.getAttribute('href')?.startsWith('http')).toBe(false);
  });

  it('asks who the visitor is with a relative, credential-free request', async () => {
    const fetchMock = openAndSignedOut();
    renderAt('/');

    await waitFor(() =>
      expect(fetchMock.mock.calls.some(([url]) => url === ME_URL)).toBe(true),
    );
    // The session is an HttpOnly cookie, so this code neither reads nor sends
    // one — the browser attaches it because the request is same-origin.
    const call = fetchMock.mock.calls.find(([url]) => url === ME_URL);
    expect(JSON.stringify(call?.[1] ?? {})).not.toMatch(
      /api-key|authorization/i,
    );
  });

  /** The acceptance criterion: the page shows their Discord username and ID. */
  it('shows the username and the Discord ID once signed in', async () => {
    openAndSignedIn();
    renderAt('/');

    expect(await screen.findByText(/adam/)).toBeTruthy();
    expect(await screen.findByText('308994132968210433')).toBeTruthy();
    // And the sign-in control is gone.
    expect(
      screen.queryByRole('link', { name: /sign in with discord/i }),
    ).toBeNull();
  });

  it('renders signed-out as plain text with the button still there', async () => {
    openAndSignedOut();
    renderAt('/');

    expect(await screen.findByText(/you are not signed in/i)).toBeTruthy();
    // "Plain text, not a screen" — the heading and the same-origin evidence
    // paragraph are still on the page.
    expect(screen.getByRole('heading', { level: 1 })).toBeTruthy();
  });

  /**
   * The visitor pressed Cancel at Discord's consent screen; the callback
   * redirected to `/api-tokens/?signin=cancelled`. Plain text, and the button
   * stays where it was — this is not an error state.
   */
  it('says sign-in was cancelled, in plain text, and still offers the button', async () => {
    openAndSignedOut();
    renderAt('/?signin=cancelled');

    expect(await screen.findByText(/sign-in cancelled/i)).toBeTruthy();
    expect(
      screen.getByRole('link', { name: /sign in with discord/i }),
    ).toBeTruthy();
    // Not dressed up as a failure.
    expect(screen.queryByText(/could not/i)).toBeNull();
  });

  it('does not claim a cancellation that did not happen', async () => {
    openAndSignedOut();
    renderAt('/');
    await screen.findByText(/you are not signed in/i);
    expect(screen.queryByText(/cancelled/i)).toBeNull();
  });

  /** Sign-out is a POST, and the page re-asks the server rather than assuming. */
  it('signs out with a POST and re-reads the session', async () => {
    let authenticated = true;
    const fetchMock = stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({
        json: async () =>
          authenticated
            ? { authenticated: true, user_id: '1', username: 'adam' }
            : { authenticated: false },
      }),
      [LOGOUT_URL]: () => {
        authenticated = false;
        return { status: 204, json: async () => ({}) };
      },
    });

    renderAt('/');
    const button = await screen.findByRole('button', { name: /sign out/i });
    // `fireEvent` rather than `button.click()`: it wraps the dispatch in
    // `act(...)`, so the state updates it triggers are flushed before the
    // assertions rather than warned about after them.
    fireEvent.click(button);

    expect(await screen.findByText(/you are not signed in/i)).toBeTruthy();
    const logout = fetchMock.mock.calls.find(([url]) => url === LOGOUT_URL);
    expect(logout).toBeTruthy();
    // A GET sign-out is triggerable by any third-party page; the backend only
    // accepts POST and the client must not depend on that being lenient.
    expect((logout?.[1] as RequestInit | undefined)?.method).toBe('POST');
  });

  /**
   * A backend that cannot answer "who am I" must not leave the page on a
   * spinner — the same rule the config probe follows, applied to the second
   * call this page makes.
   */
  it('surfaces a failed session check instead of spinning forever', async () => {
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({ ok: false, status: 502 }),
    });
    renderAt('/');

    expect(
      await screen.findByText(/could not check your sign-in status/i),
    ).toBeTruthy();
    expect(
      screen.queryByText(/Checking whether you are signed in/i),
    ).toBeNull();
  });

  /**
   * The closed portal must ask nothing about sessions. The gate answers
   * `/auth/me` with an empty 404 while the flag is off, so a call here would
   * render a failure on the page every visitor currently sees.
   */
  it('does not call the sign-in routes while the portal is closed', async () => {
    const fetchMock = stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: false }) }),
    });
    renderAt('/');

    await screen.findByText(/not yet available/i);
    expect(fetchMock.mock.calls.every(([url]) => url === CONFIG_URL)).toBe(
      true,
    );
  });
});
