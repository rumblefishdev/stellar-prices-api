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
const USAGE_URL = '/api-tokens/api/usage';

/**
 * The usage endpoint's "no key yet" answer (task 0188) — the default for every
 * signed-in stub, because the dashboard now asks for usage on mount and a
 * stub that does not answer it would fail the request and render a failure
 * state under every other test's assertions.
 */
const usageNoKey = () => ({
  ok: false,
  status: 404,
  json: async () => ({ code: 'no_key', message: 'you have no API key yet' }),
});

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
    [USAGE_URL]: usageNoKey,
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

  /**
   * `?signin=failed` is the other landing state: any OAuth error that is not
   * the visitor declining. Rendering it as "cancelled" is what let a drifted
   * scope registration look like every visitor changing their mind.
   */
  it('distinguishes a failed sign-in from a cancelled one', async () => {
    openAndSignedOut();
    renderAt('/?signin=failed');

    expect(await screen.findByText(/could not be completed/i)).toBeTruthy();
    expect(screen.queryByText(/sign-in cancelled/i)).toBeNull();
    // Still a plain-text state with the button, not an error screen.
    expect(
      screen.getByRole('link', { name: /sign in with discord/i }),
    ).toBeTruthy();
  });

  it('does not claim a failure that did not happen', async () => {
    openAndSignedOut();
    renderAt('/?signin=cancelled');
    expect(await screen.findByText(/sign-in cancelled/i)).toBeTruthy();
    expect(screen.queryByText(/could not be completed/i)).toBeNull();
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
      [USAGE_URL]: usageNoKey,
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
   * A page that reports an error and removes the only control on it is a dead
   * end the visitor can leave only by guessing at a reload. Signing in is a
   * fresh top-level navigation, so it does not depend on the request that just
   * failed — the button should still be there.
   */
  it('still offers sign-in when the session check fails', async () => {
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({ ok: false, status: 502 }),
    });
    renderAt('/');

    await screen.findByText(/could not check your sign-in status/i);
    const link = screen.getByRole('link', { name: /sign in with discord/i });
    expect(link.getAttribute('href')).toBe('/api-tokens/api/auth/login');
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

/**
 * The API key (task 0187).
 *
 * Every test here starts signed in, because that is the only state the control
 * exists in. `fetch` is stubbed rather than the module mocked, so these also
 * cover `issueKey` in `src/api/portal.ts` — including that its URL is relative
 * and its verb is `POST`.
 */
describe('the API key', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const KEY_URL = '/api-tokens/api/key';
  const KEY_VALUE = 'aBcDeF0123456789aBcDeF0123456789aBcDeF01';

  const signedInWithKey = (
    key: Record<string, unknown> = {
      key_id: 'abc123',
      name: 'discord-308994132968210433-key',
      value: KEY_VALUE,
      created: true,
    },
  ) =>
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: () => ({ json: async () => key }),
      [USAGE_URL]: usageNoKey,
    });

  const renderApp = () =>
    render(
      <MemoryRouter initialEntries={['/']}>
        <App />
      </MemoryRouter>,
    );

  /**
   * **Nothing is fetched until the visitor presses.** The backend's `GET` and
   * `POST` on `/key` are the same operation — without a registry it cannot tell
   * "deleted by hand" from "never issued", so a reveal has to be able to create
   * — which means a page that asked on load would issue a real production API
   * key to anyone who merely opened it.
   */
  it('issues nothing until the button is pressed', async () => {
    const fetchMock = signedInWithKey();
    renderApp();

    await screen.findByRole('button', { name: /get my api key/i });
    expect(fetchMock.mock.calls.some(([url]) => url === KEY_URL)).toBe(false);
  });

  it('issues the key on a relative POST and shows it masked', async () => {
    const fetchMock = signedInWithKey();
    renderApp();

    fireEvent.click(
      await screen.findByRole('button', { name: /get my api key/i }),
    );

    const shown = await screen.findByTestId('api-key');
    // Masked on arrival — the visitor asked for a key, not for it to appear on
    // screen while they were looking at the button. This renders during
    // screen-shares.
    expect(shown.textContent).not.toContain(KEY_VALUE);
    expect(shown.textContent).toMatch(/^•+$/);

    const call = fetchMock.mock.calls.find(([url]) => url === KEY_URL) as [
      string,
      RequestInit,
    ];
    expect(call[0].startsWith('http')).toBe(false);
    expect(call[1].method).toBe('POST');
  });

  /**
   * The mask must not be a prefix-and-suffix of the real value. That habit
   * comes from card numbers, where the unmasked part is not the secret; here it
   * would leak the beginning and end of a credential for no benefit.
   */
  it('leaks no part of the value while masked', async () => {
    signedInWithKey();
    renderApp();
    fireEvent.click(
      await screen.findByRole('button', { name: /get my api key/i }),
    );
    await screen.findByTestId('api-key');

    for (const fragment of [
      KEY_VALUE.slice(0, 4),
      KEY_VALUE.slice(-4),
      KEY_VALUE,
    ]) {
      expect(document.body.textContent).not.toContain(fragment);
    }
  });

  it('reveals and re-hides the value on the toggle', async () => {
    signedInWithKey();
    renderApp();
    fireEvent.click(
      await screen.findByRole('button', { name: /get my api key/i }),
    );
    await screen.findByTestId('api-key');

    fireEvent.click(screen.getByRole('button', { name: /^reveal$/i }));
    expect(screen.getByTestId('api-key').textContent).toBe(KEY_VALUE);

    fireEvent.click(screen.getByRole('button', { name: /^hide$/i }));
    expect(screen.getByTestId('api-key').textContent).not.toContain(KEY_VALUE);
  });

  it('copies the real value even while it is masked', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    signedInWithKey();
    renderApp();
    fireEvent.click(
      await screen.findByRole('button', { name: /get my api key/i }),
    );
    await screen.findByTestId('api-key');

    fireEvent.click(screen.getByRole('button', { name: /^copy$/i }));

    // The masked display must not become what gets copied — the reason the
    // component keeps the value in state rather than reading it back out of the
    // DOM.
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(KEY_VALUE));
    expect(await screen.findByText(/^copied\.$/i)).toBeTruthy();
  });

  /**
   * `navigator.clipboard` is absent on an insecure origin. A missing API must
   * not throw past the handler and blank the page; it must say what to do
   * instead.
   */
  it('says so when copying is unavailable, rather than throwing', async () => {
    vi.stubGlobal('navigator', {});
    signedInWithKey();
    renderApp();
    fireEvent.click(
      await screen.findByRole('button', { name: /get my api key/i }),
    );
    await screen.findByTestId('api-key');

    fireEvent.click(screen.getByRole('button', { name: /^copy$/i }));

    expect(await screen.findByText(/copy it by hand/i)).toBeTruthy();
    // And the key is still there to copy by hand.
    expect(screen.getByTestId('api-key')).toBeTruthy();
  });

  it('reports a failure and leaves the button pressable', async () => {
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: () => ({ ok: false, status: 502 }),
      [USAGE_URL]: usageNoKey,
    });
    renderApp();

    fireEvent.click(
      await screen.findByRole('button', { name: /get my api key/i }),
    );

    expect(await screen.findByText(/could not get your api key/i)).toBeTruthy();
    // A dead end is worse than a failure: a `502` here is usually transient, so
    // the control the visitor would use to retry has to survive it.
    expect(
      screen.getByRole('button', { name: /get my api key/i }),
    ).toBeTruthy();
  });

  /**
   * `401` is the one failure with an answer other than "press it again": the
   * session expired while the tab sat open. `api/portal.ts` carries the status
   * through for exactly this, and without this branch that would be a promise
   * the page did not keep — the visitor would be told "answered 401" and left
   * to work out that they need to sign in again.
   */
  it('tells the visitor to sign in again when the session has expired', async () => {
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: () => ({ ok: false, status: 401 }),
      [USAGE_URL]: usageNoKey,
    });
    renderApp();

    fireEvent.click(
      await screen.findByRole('button', { name: /get my api key/i }),
    );

    expect(await screen.findByText(/session has expired/i)).toBeTruthy();
    // And not the raw status, which says nothing a visitor can act on.
    expect(document.body.textContent).not.toContain('answered 401');
  });

  /** The key belongs to the session, so signing out must take it off screen. */
  it('is not rendered while signed out', async () => {
    openAndSignedOut();
    renderApp();

    await screen.findByRole('link', { name: /sign in with discord/i });
    expect(
      screen.queryByRole('button', { name: /get my api key/i }),
    ).toBeNull();
    expect(screen.queryByTestId('api-key')).toBeNull();
  });

  /**
   * The closed portal must offer no key control at all: the route answers an
   * empty `404` while the flag is off, and until task 0189's eligibility gate
   * lands that flag is the only thing between a stranger and a real key.
   */
  it('is not rendered while the portal is closed', async () => {
    stubFetch({ json: async () => ({ enabled: false }) });
    renderApp();

    await screen.findByText(/not yet available/i);
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });
});

/**
 * Usage against quota (task 0188).
 *
 * Every test starts signed in — the section exists only there. `fetch` is
 * stubbed rather than the module mocked, so these also cover `fetchUsage` in
 * `src/api/portal.ts` — including that its URL is relative (the same-origin
 * property task 0184 provides) and that a `404 no_key` is a renderable state,
 * not a failure.
 */
describe('usage against quota', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const KEY_URL = '/api-tokens/api/key';

  const USAGE = {
    used: 121,
    remaining: 99879,
    limit: 100000,
    period_start: '2026-08-01',
    period_end: '2026-08-31',
    resets_at: '2026-09-01T00:00:00Z',
    as_of: '2026-08-19T10:15:00Z',
  };

  const signedInWithUsage = (
    usage: () => Partial<Response> & { json?: () => unknown } = () => ({
      json: async () => USAGE,
    }),
  ) =>
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [USAGE_URL]: usage,
    });

  const renderApp = () =>
    render(
      <MemoryRouter initialEntries={['/']}>
        <App />
      </MemoryRouter>,
    );

  /**
   * The acceptance criterion, rendered: used, remaining and the limit as
   * numbers, plus the 1 req/s rate limit and the reset date — all visible on
   * one screen.
   */
  it('shows used, remaining, limit, the rate limit and the reset date', async () => {
    const fetchMock = signedInWithUsage();
    renderApp();

    expect((await screen.findByTestId('usage-used')).textContent).toBe('121');
    expect(screen.getByTestId('usage-remaining').textContent).toBe('99879');
    expect(screen.getByTestId('usage-limit').textContent).toBe('100000');

    // The limits as numbers, not prose (task 0157's figures).
    expect(screen.getByText(/request per second/i)).toBeTruthy();
    // The reset rule is OURS — the 1st of the month, 00:00 UTC — and the next
    // date is rendered from the response, not computed in the page.
    expect(screen.getByText(/1st of each month, 00:00 UTC/i)).toBeTruthy();
    expect(screen.getByText(/2026-09-01/)).toBeTruthy();

    // And the URL is relative: same-origin, cookie attached by the browser.
    const call = fetchMock.mock.calls.find(([url]) => url === USAGE_URL) as [
      string,
    ];
    expect(call[0].startsWith('http')).toBe(false);
  });

  /**
   * **The wording this task decides once** (task 0193 restyles it without
   * re-deciding): every rendered figure carries when it was last refreshed and
   * that AWS reports with a delay. Without this line, a visitor who just made
   * requests reads the dashboard as broken.
   */
  it('states when the figure was last refreshed, and that AWS lags', async () => {
    signedInWithUsage();
    renderApp();

    await screen.findByTestId('usage-used');
    expect(screen.getByText(/last updated/i).textContent).toMatch(
      /AWS reports usage with a delay/i,
    );
    // The timestamp is the backend's `as_of` — the moment of the GetUsage —
    // rendered in UTC (the decided wording says UTC, not toUTCString's
    // "GMT"), not the moment of the page load.
    expect(screen.getByText(/last updated/i).textContent).toContain(
      new Date(USAGE.as_of).toUTCString().replace(/GMT$/, 'UTC'),
    );
    expect(screen.getByText(/last updated/i).textContent).not.toContain('GMT');
  });

  /** Usage is read-only, so — unlike the key — it may and does load on mount. */
  it('fetches usage on mount, without any button press', async () => {
    const fetchMock = signedInWithUsage();
    renderApp();

    await screen.findByTestId('usage-used');
    expect(fetchMock.mock.calls.some(([url]) => url === USAGE_URL)).toBe(true);
    // And still nothing touched the key route — reading usage must never
    // issue.
    expect(fetchMock.mock.calls.some(([url]) => url === KEY_URL)).toBe(false);
  });

  /** A signed-in visitor with no key is told so, in words they can act on. */
  it('renders the no-key state rather than an error', async () => {
    signedInWithUsage(usageNoKey);
    renderApp();

    expect(await screen.findByText(/no API key yet/i)).toBeTruthy();
    expect(document.body.textContent).not.toContain('Could not load');
    // The limits still render — they belong to the plan, not to a key, and a
    // visitor deciding whether to issue one is exactly who they inform.
    expect(screen.getByText(/request per second/i)).toBeTruthy();
    expect(screen.getByText(/1st of each month, 00:00 UTC/i)).toBeTruthy();
  });

  /**
   * Only the backend's own `no_key` envelope means "no key". A 404 with no
   * such code — task 0183's empty gate answer, reachable when the portal is
   * closed under a still-open tab — must NOT render "you have no API key",
   * which would be a false statement about a key that may well exist.
   */
  it('does not read the gate’s empty 404 as "no key"', async () => {
    signedInWithUsage(() => ({
      ok: false,
      status: 404,
      json: async () => {
        throw new SyntaxError('Unexpected end of JSON input');
      },
    }));
    renderApp();

    expect(await screen.findByText(/could not load your usage/i)).toBeTruthy();
    expect(screen.queryByText(/no API key yet/i)).toBeNull();
  });

  /**
   * AWS has no rows for the key yet — `used`/`remaining`/`limit` are `null`
   * together. Not rendered as zeros: "0 used of 100000" would be an invented
   * figure, and the honest state for a fresh key is "nothing recorded yet".
   * The reset rule and rate limit still render — they are ours, not AWS's.
   */
  it('says nothing is recorded yet instead of inventing zeros', async () => {
    signedInWithUsage(() => ({
      json: async () => ({
        ...USAGE,
        used: null,
        remaining: null,
        limit: null,
      }),
    }));
    renderApp();

    expect(await screen.findByText(/not recorded any usage/i)).toBeTruthy();
    expect(screen.queryByTestId('usage-used')).toBeNull();
    expect(screen.getByText(/request per second/i)).toBeTruthy();
    expect(screen.getByText(/last updated/i)).toBeTruthy();
  });

  /**
   * Straight after an issue, the backend's short cache can still answer
   * "no key" about a key the page is displaying. "You have no API key yet"
   * would be false at that moment, so the page says what is actually
   * happening instead.
   */
  it('does not claim "no key" while a freshly issued key is on screen', async () => {
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: () => ({
        json: async () => ({
          key_id: 'abc123',
          name: 'discord-308994132968210433-key',
          value: 'aBcDeF0123456789aBcDeF0123456789aBcDeF01',
          created: true,
        }),
      }),
      [USAGE_URL]: usageNoKey,
    });
    renderApp();

    await screen.findByText(/no API key yet/i);
    fireEvent.click(
      await screen.findByRole('button', { name: /get my api key/i }),
    );
    await screen.findByTestId('api-key');

    expect(await screen.findByText(/your key is new/i)).toBeTruthy();
    expect(screen.queryByText(/no API key yet/i)).toBeNull();
  });

  /** The refresh control re-asks; the backend's cache bounds what that costs. */
  it('refreshes on the button', async () => {
    const fetchMock = signedInWithUsage();
    renderApp();
    await screen.findByTestId('usage-used');
    const before = fetchMock.mock.calls.filter(
      ([url]) => url === USAGE_URL,
    ).length;

    fireEvent.click(screen.getByRole('button', { name: /refresh/i }));

    await waitFor(() =>
      expect(
        fetchMock.mock.calls.filter(([url]) => url === USAGE_URL).length,
      ).toBe(before + 1),
    );
    await screen.findByTestId('usage-used');
  });

  /** A backend failure is a stated failure, not a blank section. */
  it('reports a failure and keeps the refresh control', async () => {
    signedInWithUsage(() => ({ ok: false, status: 502 }));
    renderApp();

    expect(await screen.findByText(/could not load your usage/i)).toBeTruthy();
    expect(screen.getByRole('button', { name: /refresh/i })).toBeTruthy();
  });

  /**
   * An expired session reads the same here as in the key section: "sign out
   * and sign in again", through the shared `describeFailure`. Without it the
   * two sections describe one cause in two vocabularies — the key section in
   * words, this one as a raw "answered 401" — and that reads as two bugs.
   */
  it('tells the visitor to sign in again when the session has expired', async () => {
    signedInWithUsage(() => ({ ok: false, status: 401 }));
    renderApp();

    expect(await screen.findByText(/session has expired/i)).toBeTruthy();
    expect(document.body.textContent).not.toContain('answered 401');
  });
});
