import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ROUTER_BASENAME } from '../base-path';
import App from './app';

/**
 * Records the router's current query string, so the one-shot landing-param
 * tests (task 0189, closing 0186's O10) can assert the URL was cleaned while
 * the banner stayed — `MemoryRouter` has no `window.location` to inspect.
 */
let lastSearch = '';
let lastPath = '';
function LocationSpy() {
  const location = useLocation();
  lastSearch = location.search;
  lastPath = location.pathname;
  return null;
}

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
const KEY_URL = '/api-tokens/api/key';
/** The revocation — "Replace my key" (task 0191). */
const REWORK_URL = '/api-tokens/api/key/rework';
/** Where both "get my API key" and every retry link point (task 0189). */
const ISSUE_HREF = '/api-tokens/api/auth/login?action=issue';

/**
 * `/config` for an open portal (task 0183) carrying the free plan's rate limit
 * (task 0188).
 *
 * The limit is part of every open-portal stub because it is part of every real
 * `/config`: `compute-stack.ts` sets `PORTAL_RATE_LIMIT` from
 * `pricingApiFreePlanRateLimit` unconditionally. `1` is what
 * `infra/envs/production.json` holds today — and the point of the field is that
 * changing that file changes the page, so the tests below assert the rendered
 * figure against THIS value rather than against a literal of their own.
 */
const openConfig = () => ({
  json: async () => ({ enabled: true, rate_limit_per_second: 1 }),
});

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

/**
 * The key route's "no key" answer — the same envelope discipline as
 * `usageNoKey`, and the default for signed-in stubs since task 0189 made the
 * route read-only and the page fetch it on mount.
 */
const keyNoKey = () => ({
  ok: false,
  status: 404,
  json: async () => ({ code: 'no_key', message: 'you have no API key yet' }),
});

/** The portal open, and nobody signed in. */
const openAndSignedOut = () =>
  stubRoutes({
    [CONFIG_URL]: openConfig,
    [ME_URL]: () => ({ json: async () => ({ authenticated: false }) }),
  });

/** The portal open, with a completed round-trip behind it. */
const openAndSignedIn = () =>
  stubRoutes({
    [CONFIG_URL]: openConfig,
    [ME_URL]: () => ({
      json: async () => ({
        authenticated: true,
        user_id: '308994132968210433',
        username: 'adam',
      }),
    }),
    [KEY_URL]: keyNoKey,
    [USAGE_URL]: usageNoKey,
  });

const renderApp = () =>
  render(
    <MemoryRouter initialEntries={['/']}>
      <App />
    </MemoryRouter>,
  );

/**
 * The login card alone — the one place on the page a sign-in control can be.
 *
 * Scoping exists because of task 0193: this app used to BE the panel, so
 * "offers nothing to click" could be asserted against the whole document.
 * Since the landing page arrived, the document also carries a navbar, a hero,
 * a footer and a "Back to landing" link whose targets are `#features`,
 * `#use-cases`, `#top` and the OpenAPI document — navigation that is correct
 * whether the portal is open or shut, and that a closed-portal assertion has
 * no business counting.
 *
 * The assertion itself is NOT relaxed. Inside this panel the rule is still
 * zero controls while the flag is off, and the two controls that could promise
 * a key from outside it — the hero's and the navbar's "Get API Key" — are
 * rendered only on a confirmed-open probe and are covered by their own test
 * below. Widening the scope back out would fail on the footer, not on a
 * regression.
 */
function portalPanel() {
  return within(screen.getByTestId('login-card'));
}

/**
 * The three routes, and the two redirects between them (task 0193).
 *
 * These exist because the routes are a CONTRACT with the backend, not a
 * cosmetic split. `portal/auth/mod.rs` sends every OAuth outcome to
 * `/api-tokens/` and says so deliberately — "when the portal grows a second
 * page, the page it lands on decides where to go next; this handler still will
 * not". `/` is that page. If these forwards break, a completed sign-in ends on
 * the marketing page and the visitor never reaches the key they just proved
 * they are entitled to, with nothing on screen to say why.
 *
 * They also pin the guard the brief asks for in the other direction: a visitor
 * with no session who arrives at `/dashboard` goes to `/api-tokens/`.
 */
describe('routes', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    lastPath = '';
    lastSearch = '';
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const renderAt = (entry: string) =>
    render(
      <MemoryRouter initialEntries={[entry]}>
        <App />
        <LocationSpy />
      </MemoryRouter>,
    );

  it('sends a signed-in visitor from the landing to the dashboard', async () => {
    openAndSignedIn();
    renderAt('/');

    // The OAuth callback lands here; the dashboard is where it must end up.
    await waitFor(() => expect(lastPath).toBe('/dashboard'));
    expect(
      await screen.findByRole('heading', { name: /^api key$/i }),
    ).toBeTruthy();
  });

  it('carries the issue outcome through to the dashboard', async () => {
    openAndSignedIn();
    renderAt('/?issue=not_member');

    // The forward must not eat the query. `?issue=…` is a one-shot landing
    // state owned by task 0189 and the dashboard is what renders it — dropping
    // it here would swallow an eligibility refusal the visitor is owed.
    await waitFor(() => expect(lastPath).toBe('/dashboard'));
    expect(await screen.findByTestId('issue-not-member')).toBeTruthy();
  });

  it('sends a returning visitor with a signin outcome to the login screen', async () => {
    openAndSignedOut();
    renderAt('/?signin=cancelled');

    await waitFor(() => expect(lastPath).toBe('/login'));
    expect(await screen.findByTestId('signin-failed')).toBeTruthy();
  });

  /**
   * ⚠️ **"Back to landing" stays ABOVE the card, top-left** (Adam,
   * 2026-08-26, Figma `824:140`). It briefly moved into the foot of the card
   * and moved straight back; pinned here because the trap is the same in both
   * directions — rendered in both places it is two links with one name and one
   * target, and rendered unconditionally it appears on the landing page's own
   * status panel, where it points at the page you are already reading.
   */
  it('puts exactly one back link above the login card, and none on the landing', async () => {
    openAndSignedOut();
    renderAt('/login');

    await screen.findByRole('link', { name: /sign in with discord/i });
    const back = screen.getAllByRole('link', { name: /back to landing/i });
    expect(back).toHaveLength(1);
    // Above the card, not inside it.
    expect(screen.getByTestId('login-card').contains(back[0])).toBe(false);
  });

  it('offers no back link on the landing page itself', async () => {
    openAndSignedOut();
    renderAt('/');

    await screen.findAllByRole('link', { name: /get api key/i });
    expect(screen.queryByRole('link', { name: /back to landing/i })).toBeNull();
  });

  it('leaves a signed-out visitor on the landing page', async () => {
    openAndSignedOut();
    renderAt('/');

    expect(
      (await screen.findAllByRole('link', { name: /get api key/i })).length,
    ).toBeGreaterThan(0);
    expect(lastPath).toBe('/');
    // And the login card is not on the landing page — it is a route of its own.
    expect(screen.queryByTestId('login-card')).toBeNull();
  });

  it('shows the login screen on its own, with none of the landing page', async () => {
    openAndSignedOut();
    renderAt('/login');

    expect(await screen.findByTestId('login-card')).toBeTruthy();
    expect(lastPath).toBe('/login');
    // The marketing sections belong to `/`. "Only this one view" is the brief.
    expect(document.getElementById('features')).toBeNull();
    expect(document.getElementById('use-cases')).toBeNull();
  });

  it('sends a signed-in visitor away from the login screen', async () => {
    openAndSignedIn();
    renderAt('/login');

    await waitFor(() => expect(lastPath).toBe('/dashboard'));
  });

  it('sends a visitor with no session away from the dashboard', async () => {
    openAndSignedOut();
    renderAt('/dashboard');

    await waitFor(() => expect(lastPath).toBe('/'));
    // By heading, not by text: the landing page's Self-Service section says
    // "…your API key is ready immediately", which a loose text match hits.
    expect(screen.queryByRole('heading', { name: /^api key$/i })).toBeNull();
  });

  it('waits for the session before deciding about the dashboard', async () => {
    // The redirect must not fire while `/auth/me` is still in flight: that is
    // exactly the moment an arrival from the OAuth callback passes through,
    // and bouncing it would break the one journey these routes exist for.
    let answer: (value: unknown) => void = () => undefined;
    const pending = new Promise((resolve) => {
      answer = resolve;
    });
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => {
        if (url === CONFIG_URL)
          return {
            ok: true,
            status: 200,
            json: async () => ({ enabled: true }),
          };
        if (url === ME_URL) {
          await pending;
          return {
            ok: true,
            status: 200,
            json: async () => ({
              authenticated: true,
              user_id: '1',
              username: 'adam',
            }),
          };
        }
        return {
          ok: false,
          status: 404,
          json: async () => ({ code: 'no_key' }),
        };
      }),
    );

    renderAt('/dashboard');
    expect(
      await screen.findByText(/checking whether you are signed in/i),
    ).toBeTruthy();
    expect(lastPath).toBe('/dashboard');

    answer(undefined);
    expect(
      await screen.findByRole('heading', { name: /^api key$/i }),
    ).toBeTruthy();
    expect(lastPath).toBe('/dashboard');
  });

  it('sends an unknown path back to the landing page', async () => {
    openAndSignedOut();
    renderAt('/nonsense');

    await waitFor(() => expect(lastPath).toBe('/'));
  });

  it('serves the quick start to a signed-in visitor under the dashboard bar', async () => {
    openAndSignedIn();
    renderAt('/quick-start');

    expect(
      await screen.findByRole('heading', {
        name: /get your first response in under 5 minutes/i,
      }),
    ).toBeTruthy();
    // The signed-in bar — once `/auth/me` has answered — with THIS page
    // underlined rather than the dashboard.
    const bar = within(
      await screen.findByRole('navigation', { name: 'Dashboard' }),
    );
    expect(
      bar
        .getByRole('link', { name: 'Quick start' })
        .getAttribute('aria-current'),
    ).toBe('page');
    expect(
      bar.getByRole('link', { name: 'Dashboard' }).getAttribute('aria-current'),
    ).toBeNull();
    expect(bar.getByText('adam')).toBeTruthy();
    expect(lastPath).toBe('/quick-start');
  });

  it('points the footer dashboard link at the prefix the app is served from', async () => {
    openAndSignedIn();
    // WITH the basename, unlike every other test here: the bug this pins only
    // exists under one — a bare `href="/dashboard"` and a router link resolve
    // to the same string when the app is mounted at the root, and to
    // different ones on the deployment, where the bundle lives under
    // `/api-tokens/`.
    render(
      <MemoryRouter
        basename={ROUTER_BASENAME}
        initialEntries={[`${ROUTER_BASENAME}/quick-start`]}
      >
        <App />
      </MemoryRouter>,
    );

    // Wait for the session to settle: the footer offers the dashboard only
    // to somebody who has one, and the signed-in bar is the proof it has.
    await screen.findByRole('navigation', { name: 'Dashboard' });
    const footer = within(screen.getByRole('navigation', { name: 'Footer' }));
    expect(
      footer.getByRole('link', { name: 'Dashboard' }).getAttribute('href'),
    ).toBe(`${ROUTER_BASENAME}/dashboard`);
  });

  it('serves the quick start to a signed-out visitor too, under the landing bar', async () => {
    openAndSignedOut();
    renderAt('/quick-start');

    expect(
      await screen.findByRole('heading', {
        name: /get your first response in under 5 minutes/i,
      }),
    ).toBeTruthy();
    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeTruthy();
    expect(screen.queryByRole('navigation', { name: 'Dashboard' })).toBeNull();
  });

  it('marks the section the quick start opens on in its rail', async () => {
    openAndSignedOut();
    renderAt('/quick-start');
    await screen.findByRole('heading', { name: /^prerequisites$/i });

    const rail = within(
      screen.getByRole('navigation', { name: 'On this page' }),
    );
    const entries = rail.getAllByRole('link');
    expect(entries).toHaveLength(10);
    // Unscrolled, the rail points at the first section rather than at
    // nothing — the frame underlines `Prerequisites` for the same reason.
    expect(entries[0].getAttribute('aria-current')).toBe('location');
    expect(entries.filter((e) => e.getAttribute('aria-current'))).toHaveLength(
      1,
    );
  });

  it('switches the first-request snippet by language and copies it', async () => {
    openAndSignedIn();
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
    renderAt('/quick-start');
    await screen.findByRole('heading', { name: /first request/i });

    const [tabs] = screen.getAllByRole('tablist', { name: 'Language' });
    fireEvent.click(within(tabs).getByRole('tab', { name: 'Python' }));
    expect(
      within(tabs)
        .getByRole('tab', { name: 'Python' })
        .getAttribute('aria-selected'),
    ).toBe('true');
    expect(screen.getByRole('heading', { name: 'python' })).toBeTruthy();

    fireEvent.click(
      screen.getByRole('button', { name: 'Copy python example' }),
    );
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(writeText.mock.calls[0][0]).toContain('requests.get(');
    expect(writeText.mock.calls[0][0]).toContain('x-api-key');
    expect(await screen.findByText('Copied')).toBeTruthy();
  });
});

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
    expect(portalPanel().queryAllByRole('button')).toHaveLength(0);
    expect(portalPanel().queryAllByRole('link')).toHaveLength(0);
    // And nothing above the fold offers the key either (task 0193).
    expect(screen.queryByRole('link', { name: /get api key/i })).toBeNull();
  });

  it('renders the open state when the flag is on', async () => {
    openAndSignedOut();
    renderApp();

    // Task 0185's "sign-in arrives with the next slice" placeholder is gone —
    // this slice IS that sign-in, so the open state is now the real control.
    //
    // Since task 0193 gave the portal routes, the control the LANDING offers is
    // the one that leads to sign-in rather than the Discord button itself; the
    // button is asserted on `/login`, where it now lives. What this test still
    // pins is the gate: flag on, a way in appears and the closed sentence does
    // not.
    // `findAll`: the landing offers the same control in the navbar, the hero
    // and the footer, and a `findBy` throws on more than one match.
    expect(
      (await screen.findAllByRole('link', { name: /get api key/i })).length,
    ).toBeGreaterThan(0);
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
    stubFetch({ json: async () => ({ enabled: true }) });
    renderApp();

    // Task 0185's `Reached /api-tokens/api/config successfully — same-origin…`
    // line is gone (task 0193: the page must not read as a debug harness), so
    // what this test guards has moved to the control that ACTS on the answer.
    // The property is the same one and it is the one that matters: the page
    // must not commit to an outcome it does not have yet.
    expect(
      screen.getByText(/Checking whether the portal is open/i),
    ).toBeTruthy();
    // No offer of a key while nobody knows whether the portal is open…
    expect(screen.queryByRole('link', { name: /get api key/i })).toBeNull();
    // …and no claim that it is shut, either.
    expect(screen.queryByText(/not yet available/i)).toBeNull();

    // …and the answer, once it arrives, is acted on.
    expect(
      (await screen.findAllByRole('link', { name: /get api key/i })).length,
    ).toBeGreaterThan(0);
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
    renderAt('/login');

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

  /**
   * The acceptance criterion: the page shows their Discord username and ID.
   *
   * ⚠️ **Task 0186's criterion has eroded twice and this test records both.**
   *
   * 1. The numeric ID left the screen on 2026-08-25, when Adam had the account
   *    column cut down to the handle to match the frame. It survived as the
   *    column's `title` — one hover away — and the assertion followed it there.
   * 2. The column itself leaves the EMPTY-KEY card on 2026-08-26, with the
   *    `Dashboard - no key` frame. So for a signed-in visitor with no key, the
   *    id is nowhere at all and the username is only in the navbar.
   *
   * The test is split rather than weakened: the keyless half asserts what that
   * visitor actually gets, and the half with a key still pins the id. If 0186's
   * criterion is to be met again it needs somewhere on the empty dashboard to
   * live, and that is a change to 0186.
   */
  it('names the signed-in account, even with no key to show', async () => {
    openAndSignedIn();
    renderAt('/');

    // The navbar. `findAll` because the dashboard can name the account more
    // than once, and `findBy` throws on more than one match.
    expect((await screen.findAllByText(/adam/)).length).toBeGreaterThan(0);
    // And the sign-in control is gone.
    expect(
      screen.queryByRole('link', { name: /sign in with discord/i }),
    ).toBeNull();
  });

  it('carries the Discord ID on the account column, once there is a key', async () => {
    stubRoutes({
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: () => ({
        json: async () => ({
          key_id: 'identity-key',
          name: 'discord-identity-key',
          value: 'IDENTITYKEY00000000000000000000000000000',
        }),
      }),
      [USAGE_URL]: usageNoKey,
    });
    renderAt('/');

    const account = await screen.findByTitle('308994132968210433');
    expect(account.textContent).toContain('adam');
  });

  /**
   * ⚠️ "You are not signed in." was removed on 2026-08-26 (Adam, Figma
   * `824:140`). The state is now identified by what it OFFERS rather than by
   * a sentence about what the visitor is not, so that is what this asserts.
   */
  it('renders the signed-out card with its heading and the button', async () => {
    openAndSignedOut();
    renderAt('/login');

    expect(
      await screen.findByRole('link', { name: /sign in with discord/i }),
    ).toBeTruthy();
    expect(screen.getByRole('heading', { level: 1 })).toBeTruthy();
    expect(screen.queryByText(/you are not signed in/i)).toBeNull();
  });

  /**
   * The acceptance criterion (task 0189): both prerequisites are stated
   * **before** the visitor authenticates — learning about the membership
   * requirement after the consent screen means they authorised an app for
   * nothing. The invite is the registered vanity code; the account-age line
   * names no number, because the threshold is operator configuration the
   * backend reports when it matters.
   *
   * ⚠️ **Asserted on the LANDING page since 2026-08-26.** The sign-in card
   * used to carry the same sentence and Adam removed it to match Figma
   * `824:140`; the copy survives in the FAQ, which is what the criterion
   * actually names ("the landing page states both prerequisites"). Moved
   * rather than deleted, because a criterion with no test is a criterion
   * nobody will notice breaking.
   */
  it('states both prerequisites on the landing page, before the visitor authenticates', async () => {
    openAndSignedOut();
    renderAt('/');

    // ⚠️ Behind an accordion, and the click is part of the assertion. This is
    // WEAKER than the paragraph it replaced, which sat above the sign-in
    // button and needed no interaction — recorded here rather than smoothed
    // over, because the criterion says "states", and a collapsed FAQ row
    // states nothing until somebody opens it.
    fireEvent.click(
      await screen.findByRole('button', { name: /how do i get an api key/i }),
    );

    const invite = await screen.findByRole('link', {
      name: /stellar developers discord/i,
    });
    expect(invite.getAttribute('href')).toBe('https://discord.gg/stellardev');

    // The age line names no number — a hard-coded threshold would drift the
    // moment the SSM parameter changes. Scoped to the answer itself: the
    // landing page says "under 5 minutes" about the quick start three
    // sections away, and a whole-document match would fail on that.
    const answer = screen.getByText(/not brand new/i);
    expect(answer.textContent).not.toMatch(/\d+\s*(minute|hour|day)/i);
  });

  /**
   * The landing param is one-shot (task 0189, closing 0186's O10): the banner
   * renders for the landing that carried it, and the URL is cleaned in place —
   * so a sign-out after a cancelled attempt, or a reload, shows no stale
   * "Sign-in cancelled".
   */
  it('clears the signin outcome from the URL so it cannot go stale', async () => {
    openAndSignedOut();
    render(
      <MemoryRouter initialEntries={['/?signin=cancelled']}>
        <App />
        <LocationSpy />
      </MemoryRouter>,
    );

    expect(await screen.findByTestId('signin-failed')).toBeTruthy();
    await waitFor(() => expect(lastSearch).not.toContain('signin'));
    // The screen survives the cleanup — it belongs to this landing.
    expect(screen.getByTestId('signin-failed')).toBeTruthy();
  });

  /**
   * ⚠️ **Both `?signin=cancelled` and `?signin=failed` now render one screen**
   * (Adam, 2026-08-26, Figma `825:1284`), where task 0186 rendered two
   * banners.
   *
   * What the merge kept is what 0186 was actually defending: the copy never
   * accuses the visitor of anything, and it names the cancellation case out
   * loud rather than implying a fault. What it gave up is the separate
   * "Sign-in cancelled." wording. The backend split is untouched — the two
   * literals and their logs are asserted in `tests/portal_auth.rs` — so this
   * is one line to un-merge if it reads wrong in front of people.
   */
  it.each(['cancelled', 'failed'])(
    'renders the OAuth error screen for signin=%s',
    async (outcome) => {
      openAndSignedOut();
      renderAt(`/?signin=${outcome}`);

      // The frame's header, verbatim — shared with the waiting card, which
      // this screen is the error variant of.
      expect(
        await screen.findByRole('heading', { name: /redirecting to discord/i }),
      ).toBeTruthy();
      const message = screen.getByTestId('signin-failed');
      // The frame's sentence, verbatim — the two causes it names, and never
      // an accusation about who the visitor is.
      expect(message.textContent).toMatch(
        /discord returned an error during authorization/i,
      );
      expect(message.textContent).toMatch(/denied access/i);
      expect(message.textContent).toMatch(/timed out/i);
      // The retry is the card's one filled control.
      expect(
        screen
          .getByRole('link', { name: /try again with discord/i })
          .getAttribute('href'),
      ).toBe('/api-tokens/api/auth/login');
      // And it never reads as a refusal about who the visitor is.
      expect(screen.queryByTestId('signin-not-member')).toBeNull();
    },
  );

  /**
   * ⚠️ On the OAuth error screen the back link moves INSIDE the card, under
   * "Try again with Discord" (Adam, 2026-08-26, Figma `825:1284`).
   *
   * The property that matters is not where it is but that there is still
   * exactly ONE: the section above draws its own on every other login state,
   * and both rendering it is the bug `useOwnBackLink` exists to prevent.
   */
  it('moves the back link inside the error card, and leaves only one on the page', async () => {
    openAndSignedOut();
    renderAt('/?signin=failed');

    await screen.findByTestId('signin-failed');
    const back = screen.getAllByRole('link', { name: /back to landing/i });
    expect(back).toHaveLength(1);
    expect(screen.getByTestId('login-card').contains(back[0])).toBe(true);
  });

  /**
   * The card is REPLACED, so the ordinary sign-in control is gone — a visitor
   * looking at an unfinished round-trip is offered the retry, once, rather
   * than two buttons that do the same thing.
   */
  it('replaces the sign-in card rather than annotating it', async () => {
    openAndSignedOut();
    renderAt('/?signin=failed');

    await screen.findByTestId('signin-failed');
    expect(
      screen.queryByRole('link', { name: /sign in with discord/i }),
    ).toBeNull();
    expect(
      screen.queryByRole('heading', { name: /get your api key/i }),
    ).toBeNull();
  });

  /**
   * ⚠️ Sign-in refuses a non-member as of 2026-08-26 (Adam), and the refusal
   * is a SCREEN (Figma `825:1485`), not a banner over a sign-in button.
   *
   * The property under test is that the card is REPLACED: a visitor refused
   * for membership must not still be offered the control that will hand them
   * back to the same refusal, and must not be shown the prerequisites list
   * they have just been refused under.
   */
  it('replaces the login card with the access-not-available screen for a non-member', async () => {
    openAndSignedOut();
    renderAt('/?signin=not_member');

    expect(
      await screen.findByRole('heading', { name: /access not available/i }),
    ).toBeTruthy();
    const message = screen.getByTestId('signin-not-member');
    expect(message.textContent).toMatch(/members of the stellar discord/i);
    // Kept from 0189: the one line that explains a visitor who HAS joined and
    // is still refused (`pending: true`).
    expect(message.textContent).toMatch(/screening/i);

    // The single filled action is the invite, and it is the real one.
    const join = screen.getByRole('link', { name: /join stellar discord/i });
    expect(join.getAttribute('href')).toBe('https://discord.gg/stellardev');

    // The sign-in control is GONE — replaced by the quiet second action.
    expect(
      screen.queryByRole('link', { name: /sign in with discord/i }),
    ).toBeNull();
    expect(
      screen.getByRole('button', { name: /try different account/i }),
    ).toBeTruthy();

    // Never dressed as our fault, and never as the visitor's cancellation.
    expect(screen.queryByText(/sign-in cancelled/i)).toBeNull();
    expect(screen.queryByTestId('signin-unknown')).toBeNull();
  });

  /**
   * ⚠️ "Try different account" opens a dialog rather than re-running the
   * round-trip (Adam, 2026-08-26). The old link re-authorised the SAME account
   * — OAuth has no `select_account` prompt, so no request this client can make
   * will offer a chooser — and redrew the same refusal, which read as a dead
   * button.
   *
   * What is pinned here is that the dialog says the switch happens ON DISCORD
   * and offers a real way to get there, because that is the only step that can
   * change the answer.
   */
  it('offers a way to switch Discord account instead of re-running the same sign-in', async () => {
    openAndSignedOut();
    renderAt('/?signin=not_member');

    fireEvent.click(await screen.findByTestId('switch-account-open'));

    const dialog = await screen.findByRole('dialog', {
      name: /use a different discord account/i,
    });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    // It says WHY, rather than pretending the portal can switch accounts.
    expect(screen.getByTestId('switch-account-explainer').textContent).toMatch(
      /switch account on discord/i,
    );

    // The way out is a real link to Discord, opened so this card survives.
    const switchLink = screen.getByRole('link', {
      name: /switch account on discord/i,
    });
    expect(switchLink.getAttribute('href')).toBe('https://discord.com/login');
    expect(switchLink.getAttribute('target')).toBe('_blank');
    expect(switchLink.getAttribute('rel')).toContain('noopener');

    // And the retry is still there, second, worded as a retry.
    expect(
      screen
        .getByRole('link', { name: /try signing in again/i })
        .getAttribute('href'),
    ).toBe('/api-tokens/api/auth/login');
  });

  /**
   * The 0193 criterion, on the sign-in path this time: "could not verify" and
   * "not a member" must be tellable apart. One accuses the visitor of
   * something and names a remedy; the other accuses nobody and says retry.
   */
  it('keeps a failed membership check distinct from a refused one', async () => {
    openAndSignedOut();
    renderAt('/?signin=unknown');

    const banner = await screen.findByTestId('signin-unknown');
    expect(banner.textContent).toMatch(
      /not a statement about your membership/i,
    );
    expect(banner.textContent).toMatch(/again/i);
    // The accusation is absent, and so is its remedy: nothing here tells the
    // visitor to go and join anything, because we do not know that they have
    // not.
    expect(screen.queryByTestId('signin-not-member')).toBeNull();
    expect(banner.querySelector('a')).toBeNull();
    // The button is still there — this state is retryable.
    expect(
      screen.getByRole('link', { name: /sign in with discord/i }),
    ).toBeTruthy();
  });

  it('does not claim a cancellation that did not happen', async () => {
    openAndSignedOut();
    renderAt('/login');
    await screen.findByRole('link', { name: /sign in with discord/i });
    expect(screen.queryByText(/cancelled/i)).toBeNull();
  });

  /** Sign-out is a POST, and the page re-asks the server rather than assuming. */
  it('signs out with a POST and re-reads the session', async () => {
    let authenticated = true;
    const fetchMock = stubRoutes({
      [CONFIG_URL]: openConfig,
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
      [KEY_URL]: keyNoKey,
      [USAGE_URL]: usageNoKey,
    });

    renderAt('/');
    const button = await screen.findByRole('button', { name: /sign out/i });
    // `fireEvent` rather than `button.click()`: it wraps the dispatch in
    // `act(...)`, so the state updates it triggers are flushed before the
    // assertions rather than warned about after them.
    fireEvent.click(button);

    // Signing out empties the session, and `/dashboard` sends a visitor with
    // no session to `/api-tokens/` — so the observable result is the landing
    // page with its way back in, not the "you are not signed in" line, which
    // belongs to `/login`. The property this test exists for is unchanged and
    // asserted below: the request was a POST, and the session was re-read
    // rather than assumed.
    expect(
      (await screen.findAllByRole('link', { name: /get api key/i })).length,
    ).toBeGreaterThan(0);
    expect(screen.queryByRole('button', { name: /sign out/i })).toBeNull();
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
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({ ok: false, status: 502 }),
    });
    renderAt('/login');

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
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({ ok: false, status: 502 }),
    });
    renderAt('/login');

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
 * The sign-in popup (task 0193).
 *
 * The control stays an `<a>` with a real `href` and the popup is layered on
 * top of it, so these tests pin BOTH halves: that a click opens the
 * round-trip in a second window, and that a browser which refuses to open one
 * falls through to the navigation that has always worked.
 */
describe('the sign-in popup', () => {
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

  /** A stand-in for the second window, and the `window.open` that returns it. */
  const stubPopup = (opened: Partial<Window> | null) => {
    // The parameters are declared even though the stub ignores them: the
    // assertion below reads `calls[0][0]`, and without them the recorded call
    // tuple has length 0 and will not typecheck.
    const open = vi.fn(
      (_url: string, _name?: string, _features?: string) =>
        opened as Window | null,
    );
    vi.stubGlobal('open', open);
    return open;
  };

  const clickSignIn = async () => {
    const link = await screen.findByRole('link', {
      name: /sign in with discord/i,
    });
    fireEvent.click(link);
    return link;
  };

  it('opens the round-trip in a second window and waits', async () => {
    openAndSignedOut();
    const open = stubPopup({ closed: false, focus: () => undefined });
    renderAt('/login');

    await clickSignIn();

    // The URL is the backend's own login route, relative — the popup goes
    // through `/auth/login` so the PKCE `pending` cookie is set same-origin,
    // exactly as the full-page flow does.
    expect(open.mock.calls[0][0]).toBe('/api-tokens/api/auth/login');
    expect(await screen.findByText(/redirecting to discord/i)).toBeTruthy();
    expect(screen.getByText(/waiting for discord/i)).toBeTruthy();
    // And the escape hatch the copy promises actually points somewhere.
    expect(
      screen.getByRole('link', { name: /click here/i }).getAttribute('href'),
    ).toBe('/api-tokens/api/auth/login');
  });

  it('falls back to navigating this tab when the popup is blocked', async () => {
    openAndSignedOut();
    stubPopup(null);
    renderAt('/login');

    const link = await clickSignIn();

    // No waiting screen: the click was not intercepted, so the browser is
    // following the `href` and this document is already on its way out. A
    // "Waiting for Discord…" spinner here would be a lie about a window that
    // never opened.
    expect(screen.queryByText(/waiting for discord/i)).toBeNull();
    expect(link.getAttribute('href')).toBe('/api-tokens/api/auth/login');
    // Still the signed-out card, identified by the control it offers.
    expect(
      screen.getByRole('heading', { name: /get your api key/i }),
    ).toBeTruthy();
  });

  it('leaves a modified click alone so it can open a tab', async () => {
    openAndSignedOut();
    const open = stubPopup({ closed: false, focus: () => undefined });
    renderAt('/login');

    const link = await screen.findByRole('link', {
      name: /sign in with discord/i,
    });
    fireEvent.click(link, { metaKey: true });

    expect(open).not.toHaveBeenCalled();
    expect(screen.queryByText(/waiting for discord/i)).toBeNull();
  });

  it('reports the refusal the popup brings back, on the one screen that renders it', async () => {
    openAndSignedOut();
    stubPopup({ closed: false, focus: () => undefined });
    renderAt('/login');

    await clickSignIn();
    await screen.findByText(/waiting for discord/i);

    fireEvent(
      window,
      new MessageEvent('message', {
        origin: window.location.origin,
        data: { source: 'stellar-portal-oauth', search: '?signin=cancelled' },
      }),
    );

    // The same screen the full-page flow shows, from the same one place in
    // the component — not a second wording for the popup.
    expect(await screen.findByTestId('signin-failed')).toBeTruthy();
    expect(screen.queryByText(/waiting for discord/i)).toBeNull();
  });

  /**
   * ⚠️ **Shutting the window is reported (Adam, 2026-08-26)**, and reported
   * only after the server has been asked.
   *
   * The old behaviour went quietly back to offering the button, which is what
   * a broken button looks like. The new one must still not cry failure over a
   * sign-in that WORKED — the popup posts and closes in the same breath — so
   * the outcome is decided by `/auth/me`, not by the close.
   */
  it('reports a shut window as a failure only when no session was created', async () => {
    openAndSignedOut();
    const popup = { closed: false, focus: () => undefined };
    stubPopup(popup);
    renderAt('/login');

    await clickSignIn();
    await screen.findByText(/waiting for discord/i);

    popup.closed = true;

    expect(await screen.findByTestId('signin-failed')).toBeTruthy();
  });

  /**
   * ⚠️ The first sign-in issues a key (Adam, 2026-08-26), and the callback
   * lands the POPUP on `?issue=ok` — a query this tab never navigated to.
   * What is pinned: the opener forwards it, so the visitor lands on the
   * dashboard's first-login card rather than on the plain one.
   */
  it('lands the key the sign-in issued on the first-login card', async () => {
    let authenticated = false;
    stubRoutes({
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({
        json: async () =>
          authenticated
            ? {
                authenticated: true,
                user_id: '308994132968210433',
                username: 'adam',
              }
            : { authenticated: false },
      }),
      [KEY_URL]: () => ({
        json: async () => ({
          key_id: 'abc123',
          name: 'discord-308994132968210433-key',
          value: 'sk_test_0123456789abcdef',
        }),
      }),
      [USAGE_URL]: usageNoKey,
    });
    stubPopup({ closed: false, focus: () => undefined });
    render(
      <MemoryRouter initialEntries={['/login']}>
        <App />
        <LocationSpy />
      </MemoryRouter>,
    );

    await clickSignIn();
    await screen.findByText(/waiting for discord/i);

    authenticated = true;
    fireEvent(
      window,
      new MessageEvent('message', {
        origin: window.location.origin,
        data: { source: 'stellar-portal-oauth', search: '?issue=ok' },
      }),
    );

    expect(await screen.findByTestId('issue-ok')).toBeTruthy();
    // The card retitles once `/key` has answered — `justIssued` needs both
    // the landing AND a key on screen — so this is awaited, not read.
    expect(
      await screen.findByRole('heading', { name: /your api key is ready/i }),
    ).toBeTruthy();
    expect(await screen.findByTestId('api-key')).toBeTruthy();
    expect(lastPath).toBe('/dashboard');
  });

  it('ignores a message from another origin', async () => {
    openAndSignedOut();
    stubPopup({ closed: false, focus: () => undefined });
    renderAt('/login');

    await clickSignIn();
    await screen.findByText(/waiting for discord/i);

    fireEvent(
      window,
      new MessageEvent('message', {
        origin: 'https://not-us.example',
        data: { source: 'stellar-portal-oauth', search: '?signin=failed' },
      }),
    );

    // Still waiting. A `message` event arrives from any window that cares to
    // send one, and without the origin check a third-party page could end the
    // wait and make this card claim an outcome that never happened.
    expect(screen.getByText(/waiting for discord/i)).toBeTruthy();
    expect(screen.queryByText(/could not be completed/i)).toBeNull();
  });

  it('goes to the dashboard when the popup completes the sign-in', async () => {
    // The popup reports no refusal; the session it created is what the app
    // then finds. This is the whole journey the routes exist for.
    let authenticated = false;
    stubRoutes({
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({
        json: async () =>
          authenticated
            ? { authenticated: true, user_id: '1', username: 'adam' }
            : { authenticated: false },
      }),
      [KEY_URL]: keyNoKey,
      [USAGE_URL]: usageNoKey,
    });
    stubPopup({ closed: false, focus: () => undefined });
    renderAt('/login');

    await clickSignIn();
    await screen.findByText(/waiting for discord/i);

    authenticated = true;
    fireEvent(
      window,
      new MessageEvent('message', {
        origin: window.location.origin,
        data: { source: 'stellar-portal-oauth', search: '' },
      }),
    );

    expect(
      await screen.findByRole('heading', { name: /^api key$/i }),
    ).toBeTruthy();
  });
});

/**
 * The API key (task 0187; issuance re-shaped by task 0189).
 *
 * Every test here starts signed in, because that is the only state the control
 * exists in. `fetch` is stubbed rather than the module mocked, so these also
 * cover `fetchKey` in `src/api/portal.ts` — including that its URL is relative
 * and that nothing this page does POSTs to the key route: issuing is a
 * top-level navigation through the eligibility round-trip, and the round-trip
 * outcomes land back here as `?issue=<outcome>`.
 */
describe('the API key', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const KEY_VALUE = 'aBcDeF0123456789aBcDeF0123456789aBcDeF01';

  const signedInWithKey = (
    key: Record<string, unknown> = {
      key_id: 'abc123',
      name: 'discord-308994132968210433-key',
      value: KEY_VALUE,
    },
  ) =>
    stubRoutes({
      [CONFIG_URL]: openConfig,
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

  const signedInWithoutKey = (
    keyRoute: () => Partial<Response> & { json?: () => unknown } = keyNoKey,
  ) =>
    stubRoutes({
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: keyRoute,
      [USAGE_URL]: usageNoKey,
    });

  const renderApp = (entry = '/') =>
    render(
      <MemoryRouter initialEntries={[entry]}>
        <App />
        <LocationSpy />
      </MemoryRouter>,
    );

  /**
   * **The reversal of 0187's fetch-nothing rule, re-derived.** The key route
   * is read-only since task 0189 — it can never create — so the page shows
   * the visitor the key they already have without a press. What it must never
   * do is POST its way to one: issuing is the eligibility round-trip.
   */
  it('fetches the existing key on mount and shows it masked', async () => {
    const fetchMock = signedInWithKey();
    renderApp();

    const shown = await screen.findByTestId('api-key');
    // Masked on arrival — this renders during screen-shares.
    expect(shown.textContent).not.toContain(KEY_VALUE);
    expect(shown.textContent).toMatch(/^•+$/);

    const call = fetchMock.mock.calls.find(([url]) => url === KEY_URL) as [
      string,
      RequestInit,
    ];
    expect(call[0].startsWith('http')).toBe(false);
    // A GET — and nothing on this page ever POSTs the key route.
    expect(call[1]?.method).toBeUndefined();
    expect(
      fetchMock.mock.calls.some(
        ([url, init]) =>
          url === KEY_URL && (init as RequestInit | undefined)?.method,
      ),
    ).toBe(false);
  });

  /**
   * No key: the control is a **link into the issue round-trip**, not a button
   * with a fetch — the eligibility proof needs a fresh Discord token, which
   * only a top-level navigation can carry.
   *
   * ⚠️ **This test used to also assert the prerequisites were stated on this
   * card** (`/not brand new/i`), which was task 0189's decision: say them where
   * the decision is made. The `Dashboard - no key` frame (Adam, 2026-08-26)
   * gives the card a red strip and one button and nothing else, so they are no
   * longer here — and the assertion is dropped rather than left failing. They
   * are still stated in full on the landing page, BEFORE the visitor
   * authenticates, which is where the epic's criterion actually places them and
   * where authorising-for-nothing is the risk 0189 was guarding against.
   *
   * The rest of the test is unchanged and is the part that matters most: it is
   * still a link, it still points at the round-trip, and the card still makes
   * no request that could create anything.
   */
  it('offers the issue round-trip as a link, not a fetch', async () => {
    const fetchMock = signedInWithoutKey();
    renderApp();

    // The frame's label. The other states' retry links still read "Get my API
    // key" — this is the empty card's control, not a rename across the app.
    const link = await screen.findByRole('link', { name: /generate api key/i });
    expect(link.getAttribute('href')).toBe(ISSUE_HREF);
    expect(link.getAttribute('href')?.startsWith('http')).toBe(false);
    // And no request was made that could have created anything.
    expect(
      fetchMock.mock.calls.every(
        ([, init]) => !(init as RequestInit | undefined)?.method,
      ),
    ).toBe(true);
  });

  /**
   * The frame's red strip, and the reason it says more than "you have no key":
   * the diagnosis is what tells a signed-in visitor that pressing the button is
   * worth doing rather than a repeat of something that already failed.
   */
  it('names the likely cause on the empty card, not just the empty state', async () => {
    signedInWithoutKey();
    renderApp();

    const notice = await screen.findByTestId('no-key-notice');
    expect(notice.textContent).toMatch(/no api key found for your account/i);
    expect(notice.textContent).toMatch(/issuance failed during sign-in/i);
  });

  /**
   * Only the backend's own `no_key` envelope means "no key". The gate's empty
   * 404 (task 0183, reachable when the portal closes under an open tab) must
   * render as a stated failure — offering "get my API key" against a closed
   * portal would send the visitor into a round-trip that answers 404.
   */
  it("does not read the gate's empty 404 as no key", async () => {
    signedInWithoutKey(() => ({
      ok: false,
      status: 404,
      json: async () => {
        throw new SyntaxError('Unexpected end of JSON input');
      },
    }));
    renderApp();

    expect(await screen.findByText(/could not get your api key/i)).toBeTruthy();
    expect(screen.queryByRole('link', { name: /get my api key/i })).toBeNull();
  });

  /**
   * The mask must not be a prefix-and-suffix of the real value. That habit
   * comes from card numbers, where the unmasked part is not the secret; here it
   * would leak the beginning and end of a credential for no benefit.
   */
  it('leaks no part of the value while masked', async () => {
    signedInWithKey();
    renderApp();
    await screen.findByTestId('api-key');

    for (const fragment of [
      KEY_VALUE.slice(0, 4),
      KEY_VALUE.slice(-4),
      KEY_VALUE,
    ]) {
      expect(document.body.textContent).not.toContain(fragment);
    }
  });

  /**
   * ⚠️ "Reveal"/"Hide" became "Show key"/"Hide key" inside the ring on
   * 2026-08-26 (Adam) — one control and one wording across both dashboards,
   * where the ordinary card used to name the act differently from the
   * first-login one and put it in a different row.
   */
  it('shows and re-hides the value on the toggle', async () => {
    signedInWithKey();
    renderApp();
    await screen.findByTestId('api-key');

    fireEvent.click(screen.getByRole('button', { name: /^show key$/i }));
    expect(screen.getByTestId('api-key').textContent).toBe(KEY_VALUE);

    fireEvent.click(screen.getByRole('button', { name: /^hide key$/i }));
    expect(screen.getByTestId('api-key').textContent).not.toContain(KEY_VALUE);
  });

  it('copies the real value even while it is masked', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    signedInWithKey();
    renderApp();
    await screen.findByTestId('api-key');

    fireEvent.click(screen.getByRole('button', { name: /copy key/i }));

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
    await screen.findByTestId('api-key');

    fireEvent.click(screen.getByRole('button', { name: /copy key/i }));

    expect(await screen.findByText(/copy it by hand/i)).toBeTruthy();
    // And the key is still there to copy by hand.
    expect(screen.getByTestId('api-key')).toBeTruthy();
  });

  it('reports a reveal failure as a stated failure', async () => {
    signedInWithoutKey(() => ({ ok: false, status: 502 }));
    renderApp();

    expect(await screen.findByText(/could not get your api key/i)).toBeTruthy();
  });

  /**
   * `401` is the one failure with an answer other than "try again": the
   * session expired while the tab sat open. `api/portal.ts` carries the status
   * through for exactly this, and without this branch that would be a promise
   * the page did not keep.
   */
  it('tells the visitor to sign in again when the session has expired', async () => {
    signedInWithoutKey(() => ({ ok: false, status: 401 }));
    renderApp();

    expect(await screen.findByText(/session has expired/i)).toBeTruthy();
    // And not the raw status, which says nothing a visitor can act on.
    expect(document.body.textContent).not.toContain('answered 401');
  });

  /** The key belongs to the session, so signing out must take it off screen. */
  it('is not rendered while signed out', async () => {
    openAndSignedOut();
    // `/login`, not `/`: the key lives on the dashboard since task 0193 gave
    // the portal routes, and a signed-out visitor never reaches that route —
    // this asserts the key is absent from the screen they DO reach.
    renderApp('/login');

    await screen.findByRole('link', { name: /sign in with discord/i });
    expect(screen.queryByRole('link', { name: /get my api key/i })).toBeNull();
    expect(screen.queryByTestId('api-key')).toBeNull();
  });

  /**
   * The closed portal must offer no key control at all: the whole flow —
   * login, callback, reveal — answers an empty `404` while the flag is off.
   */
  it('is not rendered while the portal is closed', async () => {
    stubFetch({ json: async () => ({ enabled: false }) });
    renderApp();

    await screen.findByText(/not yet available/i);
    expect(portalPanel().queryAllByRole('button')).toHaveLength(0);
    expect(portalPanel().queryAllByRole('link')).toHaveLength(0);
    expect(screen.queryByRole('link', { name: /get api key/i })).toBeNull();
  });

  // -------------------------------------------------------------------------
  // The issue round-trip's landing states (task 0189) — the wording this task
  // decides, and 0193 restyles without re-deciding.
  // -------------------------------------------------------------------------

  it('welcomes a completed issue and shows the key', async () => {
    signedInWithKey();
    renderApp('/?issue=ok');

    expect(await screen.findByTestId('issue-ok')).toBeTruthy();
    expect(await screen.findByTestId('api-key')).toBeTruthy();
  });

  /**
   * The first-login card (Figma `843:2356`).
   *
   * ⚠️ **This asserted the opposite until 2026-08-26**, when Adam pointed at
   * the frame: the box holds a run of dots and a "Show key" control, so task
   * 0187's mask has no exception. The card's own controls are still its own —
   * "Copy key" and the quick-start link, not the ordinary card's "Reveal".
   */
  it('masks the key on the first-login card, behind its own Show key control', async () => {
    signedInWithKey();
    renderApp('/?issue=ok');

    const field = await screen.findByTestId('api-key');
    expect(field.textContent).not.toContain(KEY_VALUE);
    expect(field.textContent).toMatch(/^•+$/);
    expect(
      screen.getByRole('heading', { name: /your api key is ready/i }),
    ).toBeTruthy();
    expect(screen.getByRole('button', { name: /copy key/i })).toBeTruthy();
    expect(
      screen.getByRole('link', { name: /view quick start/i }),
    ).toBeTruthy();

    // The same control, in the same place, as the ordinary card's — see the
    // toggle test above.
    expect(screen.queryByRole('button', { name: /^reveal$/i })).toBeNull();
    fireEvent.click(screen.getByTestId('show-key'));
    expect(screen.getByTestId('api-key').textContent).toBe(KEY_VALUE);
    expect(screen.getByRole('button', { name: /hide key/i })).toBeTruthy();
  });

  /**
   * The mask survives a copy: pressing "Copy key" hands over the real value
   * without putting it on screen, which is the whole point of masking a card
   * whose purpose is to hand a credential over.
   */
  it('copies the real value while the key is still masked', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    signedInWithKey();
    renderApp('/?issue=ok');

    await screen.findByTestId('api-key');
    fireEvent.click(screen.getByRole('button', { name: /copy key/i }));

    expect(writeText).toHaveBeenCalledWith(KEY_VALUE);
    expect(screen.getByTestId('api-key').textContent).toMatch(/^•+$/);
  });

  /**
   * The metadata row's two instants come from `GET /key`, which task 0193
   * extended to carry `createdDate` and `lastUpdatedDate` — not from this
   * machine's clock, and not invented where AWS omits them.
   *
   * The label is the frame's "Last rotated" (Adam, 2026-08-25) even though the
   * value is `lastUpdatedDate` and this build rotates nothing — the deviation
   * is recorded at the render site.
   */
  it('dates the key from the control plane, in UTC', async () => {
    signedInWithKey({
      key_id: 'abc123',
      name: 'discord-308994132968210433-key',
      value: KEY_VALUE,
      created_at: '2026-04-13T09:30:00Z',
      last_updated_at: '2026-04-30T22:45:00Z',
    });
    renderApp('/');

    expect(await screen.findByText('Issued')).toBeTruthy();
    expect(screen.getByText('13 April 2026')).toBeTruthy();
    expect(screen.getByText('Last rotated')).toBeTruthy();
    // 22:45 UTC on the 30th stays the 30th — rendered in a zone behind UTC it
    // would read as the 1st of May, which is a different quota period.
    expect(screen.getByText('30 April 2026')).toBeTruthy();
  });

  /** A build whose backend has no timestamps yet drops the fields. */
  it('omits the dates rather than inventing them when the API sends none', async () => {
    signedInWithKey();
    renderApp('/');

    await screen.findByTestId('api-key');
    expect(screen.queryByText('Issued')).toBeNull();
    expect(screen.queryByText('Last rotated')).toBeNull();
  });

  /**
   * The frame's yellow strip, in the frame's words (Adam, 2026-08-25) —
   * replacing the "replacing issues nothing in its place" sentence this
   * carried, which was task 0191's model stated plainly.
   *
   * The date is what this pins: whichever wording the strip wears, the instant
   * it names is the start of the next quota period, and it must be a real date
   * rather than the words "next month". `/usage` is stubbed `no_key` here, so
   * this exercises the computed fallback rather than `resets_at`.
   */
  it('names the next rotation date in the notice strip', async () => {
    signedInWithKey();
    renderApp('/');

    const note = await screen.findByRole('note');
    expect(note.textContent).toMatch(/once per calendar month/i);
    const nextMonth = new Date(
      Date.UTC(new Date().getUTCFullYear(), new Date().getUTCMonth() + 1, 1),
    );
    expect(note.textContent).toContain(
      nextMonth.toLocaleDateString('en-GB', {
        day: 'numeric',
        month: 'long',
        year: 'numeric',
        timeZone: 'UTC',
      }),
    );
  });

  /**
   * The mask is the default everywhere else, which is the half of the pair
   * above that keeps 0187's rule true: no `?issue=ok`, no unmasking.
   */
  it('masks the key on an ordinary visit and keeps the plain card title', async () => {
    signedInWithKey();
    renderApp('/');

    const field = await screen.findByTestId('api-key');
    expect(field.textContent).not.toBe(KEY_VALUE);
    // Beside the value, not in the row of actions below it.
    const show = screen.getByTestId('show-key');
    expect(show.textContent).toMatch(/^show key$/i);
    expect(field.parentElement?.contains(show)).toBe(true);
    expect(screen.getByRole('heading', { name: /^api key$/i })).toBeTruthy();
    expect(
      screen.queryByRole('link', { name: /view quick start/i }),
    ).toBeNull();
  });

  /**
   * "Issued" is only ever rendered where the round-trip that just ended
   * created the key, because `GET /key` carries no timestamp — and the rate
   * limit comes from `/config`, which the stub answers with 1 req/s.
   *
   * The quota column is deliberately NOT asserted here: this stub's `/usage`
   * says "no key yet", so the page has not been told a limit and the field is
   * absent rather than invented.
   */
  it('states when the key was issued and at what rate limit', async () => {
    signedInWithKey();
    renderApp('/?issue=ok');

    expect(await screen.findByText('Issued')).toBeTruthy();
    expect(screen.getByText(/just now/i)).toBeTruthy();
    expect(screen.getByText('Rate limit')).toBeTruthy();
    expect(screen.queryByText('Monthly quota')).toBeNull();
  });

  /**
   * One page load, one `GET /key`.
   *
   * The route is read-only, so a second call mints nothing — but it is a
   * paginated `GetApiKeys` plus a `GetApiKey` against the account-wide
   * control-plane budget, doubling the per-load cost 0187's decision 12 hands
   * to 0194. It doubled because `load` depended on the `onKey` prop, which the
   * dashboard passes as an inline arrow: calling it set state, which
   * re-rendered, which made a new prop identity, which re-fired the mount
   * effect. Held in a ref now, so the count is the assertion.
   */
  it('asks for the key exactly once per load, even after reporting it', async () => {
    const fetchMock = signedInWithKey();
    renderApp();

    await screen.findByTestId('api-key');
    // The usage section refetches when a key appears (task 0188), so waiting
    // on its settled render also gives any second key call time to land.
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.filter(([url]) => url === USAGE_URL).length,
      ).toBeGreaterThan(1),
    );

    const keyCalls = fetchMock.mock.calls.filter(([url]) => url === KEY_URL);
    expect(keyCalls).toHaveLength(1);
  });

  /**
   * A cancelled round-trip has to say so, in the section the visitor was
   * looking at. It used to land on `?signin=cancelled`, whose banner renders
   * only while signed out — so a visitor who pressed "Get my API key" and
   * changed their mind at Discord came back to an unchanged dashboard.
   *
   * And it must not read as a refusal: nothing was checked, nothing was
   * denied.
   */
  it('says a cancelled round-trip was cancelled, not refused', async () => {
    signedInWithoutKey();
    renderApp('/?issue=cancelled');

    const said = await screen.findByTestId('issue-cancelled');
    expect(said.textContent).toMatch(/nothing is wrong/i);
    expect(screen.queryByTestId('issue-not-member')).toBeNull();
    expect(screen.queryByTestId('issue-denied')).toBeNull();
  });

  /**
   * `invalid_scope` is our Developer Portal registration drifting, not the
   * visitor's doing and not a Discord outage — so it gets its own state
   * rather than folding into `unknown`, which would render our misconfiguration
   * as a doubt about their membership. Same rule as `failed` vs `unknown`.
   */
  it('separates a refused round-trip from a cancelled one and from unknown', async () => {
    signedInWithoutKey();
    renderApp('/?issue=denied');

    const said = await screen.findByTestId('issue-denied');
    expect(said.textContent).toMatch(/not something you did/i);
    expect(screen.queryByTestId('issue-cancelled')).toBeNull();
    expect(screen.queryByTestId('issue-unknown')).toBeNull();
  });

  /**
   * `GetApiKeys` is eventually consistent, so a first issuance can redirect
   * before the key is listable. The page must not then contradict itself —
   * and above all must not offer to issue a key to somebody who has just been
   * given one.
   */
  it('renders a settling wait, not "no key", when the redirect beats the listing', async () => {
    signedInWithoutKey();
    renderApp('/?issue=ok');

    expect(await screen.findByTestId('issue-ok-settling')).toBeTruthy();
    // Not the success line — the key is not on screen to be ready.
    expect(screen.queryByTestId('issue-ok')).toBeNull();
    // And not the "you have no key" branch, whose control issues another one.
    expect(screen.queryByRole('link', { name: /get my api key/i })).toBeNull();
  });

  /**
   * Not a member: name the server, link the registered vanity invite, and
   * offer retry as the same round-trip — eligibility is proved per attempt,
   * never remembered, so joining and pressing again is all it takes.
   */
  it('names the server and links the invite when the visitor is not a member', async () => {
    signedInWithoutKey();
    renderApp('/?issue=not_member');

    const refusal = await screen.findByTestId('issue-not-member');
    expect(refusal.textContent).toMatch(/stellar developers discord/i);
    const invite = screen.getAllByRole('link', {
      name: /stellar developers discord/i,
    })[0];
    expect(invite.getAttribute('href')).toBe('https://discord.gg/stellardev');
    const retry = screen.getAllByRole('link', { name: /try again/i })[0];
    expect(retry.getAttribute('href')).toBe(ISSUE_HREF);
  });

  /**
   * Too young is a WAIT, not a rejection: the remaining time comes from the
   * backend's `wait_secs` — never a calendar date (that pattern is 0191's,
   * for a weeks-long cap), and never a hard-coded "5 minutes".
   */
  it("renders too-young as a wait with the backend's remaining time", async () => {
    signedInWithoutKey();
    renderApp('/?issue=too_young&wait_secs=173');

    const refusal = await screen.findByTestId('issue-too-young');
    expect(refusal.textContent).toMatch(/about 3 minutes/i);
    expect(refusal.textContent).toMatch(/not a rejection/i);
    // Not a calendar date, and not a number the backend did not send.
    expect(refusal.textContent).not.toMatch(/\d{4}-\d{2}-\d{2}/);
    expect(refusal.textContent).not.toMatch(/5 minutes/i);
    const retry = screen.getAllByRole('link', { name: /get my api key/i })[0];
    expect(retry.getAttribute('href')).toBe(ISSUE_HREF);
  });

  /** `wait_secs` arrives in a URL, so nonsense renders as generic wording. */
  it('sanitises a nonsense wait_secs instead of rendering it', async () => {
    signedInWithoutKey();
    renderApp('/?issue=too_young&wait_secs=<script>9e99');

    const refusal = await screen.findByTestId('issue-too-young');
    expect(refusal.textContent).toMatch(/a few minutes/i);
    expect(refusal.textContent).not.toContain('<script>');
    expect(refusal.textContent).not.toContain('9e99');
  });

  /**
   * "Could not verify" renders differently from "not a member" — a Discord
   * outage is not an accusation the visitor can act on, and the copy says so
   * in as many words.
   */
  it('renders could-not-verify differently from not-a-member', async () => {
    signedInWithoutKey();
    renderApp('/?issue=unknown');

    const refusal = await screen.findByTestId('issue-unknown');
    expect(refusal.textContent).toMatch(/could not verify/i);
    expect(refusal.textContent).toMatch(
      /not a statement about your membership/i,
    );
    expect(screen.queryByTestId('issue-not-member')).toBeNull();
    // Retry, in place.
    const retry = screen.getAllByRole('link', { name: /try again/i })[0];
    expect(retry.getAttribute('href')).toBe(ISSUE_HREF);
  });

  /** A key-service fault is not a membership doubt — the two read differently. */
  it('distinguishes a failed key creation from an unverifiable membership', async () => {
    signedInWithoutKey();
    renderApp('/?issue=failed');

    const failure = await screen.findByTestId('issue-failed');
    // Decision #7's split, in as many words — and worded so it is true of
    // BOTH causes: a control plane that refused after a passed check, and an
    // issuance the deployment cannot perform at all (an unwired
    // `/auth/login?action=issue` lands here rather than on a JSON page). It
    // must never claim a check ran that did not.
    expect(failure.textContent).toMatch(/our key service/i);
    expect(failure.textContent).toMatch(/not your discord membership/i);
    expect(screen.queryByTestId('issue-unknown')).toBeNull();
    expect(screen.queryByTestId('issue-not-member')).toBeNull();
  });

  /**
   * A long wait is CLAMPED into a bigger unit, never rejected into "a few
   * minutes".
   *
   * `min-account-age-minutes` is a `put-parameter` applied without a redeploy
   * and validated by nothing at deploy time, so an operator's typo produces a
   * genuinely enormous `wait_secs`. Rendering that as a coffee break is the
   * most misleading direction available: the visitor retries, is refused
   * again, and the page never lets on. Overstating a wait is recoverable.
   */
  it('renders a very long wait as a long wait, not as "a few minutes"', async () => {
    signedInWithoutKey();
    // 99,999,999s — one digit past what the old length-bounded guard allowed.
    renderApp('/?issue=too_young&wait_secs=99999999');

    const refusal = await screen.findByTestId('issue-too-young');
    expect(refusal.textContent).toMatch(/about 1158 days/i);
    expect(refusal.textContent).not.toMatch(/a few minutes/i);
  });

  /** And a value too large to be a number at all clamps to the ceiling. */
  it('clamps an absurd wait_secs to the top of the scale', async () => {
    signedInWithoutKey();
    renderApp(`/?issue=too_young&wait_secs=${'9'.repeat(40)}`);

    const refusal = await screen.findByTestId('issue-too-young');
    // A hundred years, in days — obviously wrong to a reader, which is the
    // point: it says "this figure is nonsense", not "wait a moment".
    expect(refusal.textContent).toMatch(/about 36500 days/i);
    expect(refusal.textContent).not.toMatch(/a few minutes/i);
    expect(refusal.textContent).not.toContain('Infinity');
    expect(refusal.textContent).not.toContain('NaN');
  });

  /**
   * `?issue=ok` must not sit above "Could not get your API key".
   *
   * The `none` guard beside it was written for exactly this contradiction —
   * the page asserting the key is ready next to the page saying it could not
   * produce one — and `failed` is the same contradiction with a different
   * second half.
   */
  it('does not claim the key is ready when the reveal failed', async () => {
    signedInWithoutKey(() => ({ ok: false, status: 502 }));
    renderApp('/?issue=ok');

    await screen.findByText(/could not get your api key/i);
    expect(screen.queryByTestId('issue-ok')).toBeNull();
    expect(screen.queryByTestId('issue-ok-settling')).toBeNull();
  });

  /**
   * The settling retry has to acknowledge the press.
   *
   * Without a loading state it produced no visible change at all — same
   * words, same button — so the natural reading was that the control was
   * broken. `Usage`'s Refresh has always done this; so does this one now.
   */
  it('says it is checking again when the settling retry is pressed', async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let calls = 0;
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: () => {
        calls += 1;
        if (calls === 1) return keyNoKey();
        // Held open, so the loading state is observable rather than a frame
        // the test races.
        return {
          ok: false,
          status: 404,
          json: async () => {
            await gate;
            return { code: 'no_key' };
          },
        };
      },
      [USAGE_URL]: usageNoKey,
    });
    renderApp('/?issue=ok');

    const retry = await screen.findByRole('button', { name: /check again/i });
    fireEvent.click(retry);
    expect(await screen.findByText(/checking for your api key/i)).toBeTruthy();

    release();
    // And it comes back to the settling wait, not to "you have no key".
    expect(await screen.findByTestId('issue-ok-settling')).toBeTruthy();
  });

  /**
   * The usage section must not offer to issue a key to somebody who has just
   * been issued one.
   *
   * `?issue=ok` is itself proof a key exists — the backend created it before
   * redirecting — even while `GetApiKeys` has not caught up. Without that
   * fact reaching the dashboard, the key section said "your key was created"
   * and the section directly below it said "you have no API key yet — issue
   * one above": the same self-contradiction as the `issue=ok` guard, one
   * section down.
   */
  it('does not tell a visitor who just issued a key that they have none', async () => {
    signedInWithoutKey();
    renderApp('/?issue=ok');

    await screen.findByTestId('issue-ok-settling');
    expect(await screen.findByText(/your key is new/i)).toBeTruthy();
    expect(screen.queryByText(/you have no api key yet/i)).toBeNull();
  });

  /**
   * The issue outcome is one-shot, like `?signin=…`: shown for this landing,
   * stripped from the URL, so a reload does not repeat a refusal about a
   * round-trip that is over.
   */
  it('clears the issue outcome from the URL so a reload does not repeat it', async () => {
    signedInWithoutKey();
    renderApp('/?issue=not_member&wait_secs=9');

    expect(await screen.findByTestId('issue-not-member')).toBeTruthy();
    await waitFor(() => {
      expect(lastSearch).not.toContain('issue');
      expect(lastSearch).not.toContain('wait_secs');
    });
    // The banner survives the cleanup — it belongs to this landing.
    expect(screen.getByTestId('issue-not-member')).toBeTruthy();
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
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      // ⚠️ **A key, not `keyNoKey`** (2026-08-26). This suite is about the
      // usage panel, and every test in it used to run against an account with
      // no key — incidental when the two cards were independent, and wrong
      // since the `Dashboard - no key` frame made the key card's answer empty
      // this one. An account with usage figures is an account with a key, so
      // the stub now says so and the panel under test actually renders.
      //
      // The one test that IS about having no key asserts the empty tile
      // instead, below.
      [KEY_URL]: () => ({
        json: async () => ({
          key_id: 'usage-suite-key',
          name: 'discord-usage-key',
          value: 'USAGESUITEKEY000000000000000000000000000',
        }),
      }),
      [USAGE_URL]: usage,
    });

  /** The same stub with the account's key genuinely absent. */
  const signedInWithoutAnyKey = () =>
    stubRoutes({
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: keyNoKey,
      [USAGE_URL]: usageNoKey,
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
    // ⚠️ The reset RULE sentence ("the 1st of each month, 00:00 UTC") was cut
    // from this card on 2026-08-25 at Adam's instruction, along with the lag
    // line and the Refresh button — the frame has none of them and task 0222's
    // chart takes the space. What survives is the date itself, as the caption
    // under the bar, which is the half a visitor acts on.
    expect(screen.getByText('Resets 1 September')).toBeTruthy();

    // And the URL is relative: same-origin, cookie attached by the browser.
    const call = fetchMock.mock.calls.find(([url]) => url === USAGE_URL) as [
      string,
    ];
    expect(call[0].startsWith('http')).toBe(false);
  });

  /**
   * ⚠️ **DELETED, not moved: the lag line is no longer rendered.**
   *
   * Task 0188 decided that every figure on this card carries "Last updated …
   * — AWS reports usage with a delay, so requests made in the last few minutes
   * may not be counted yet", and this test pinned that wording verbatim. Adam
   * removed the line on 2026-08-25 ("to jest do usunięcia, tutaj będą
   * wykresy") together with the reset-rule sentence and the Refresh button.
   *
   * What that costs is written down rather than quietly dropped: the panel no
   * longer tells a visitor that a figure can trail their last request by
   * minutes, so a developer who has just made calls and sees an unchanged
   * number has nothing on screen explaining why. If it should come back, this
   * is the test to restore with it — `USAGE.as_of` is still in the fixture.
   */

  /** Usage is read-only, so it may and does load on mount. */
  it('fetches usage on mount, without any button press', async () => {
    const fetchMock = signedInWithUsage();
    renderApp();

    await screen.findByTestId('usage-used');
    expect(fetchMock.mock.calls.some(([url]) => url === USAGE_URL)).toBe(true);
    // The key route is read on mount too (read-only since task 0189), but
    // nothing this page loads may WRITE — no request anywhere carries a verb.
    expect(
      fetchMock.mock.calls.every(
        ([, init]) => !(init as RequestInit | undefined)?.method,
      ),
    ).toBe(true);
  });

  /** A signed-in visitor with no key is told so, in words they can act on. */
  /**
   * ⚠️ **Both cards go EMPTY when the account has no key** (Adam, 2026-08-26,
   * from the `Dashboard - no key` frame), and this test used to assert the
   * opposite of each half.
   *
   * It asserted the usage panel's "you have no API key yet" sentence, which is
   * no longer rendered on this state, and it asserted that the rate limit still
   * showed — with a comment arguing the figure belongs to the plan rather than
   * to a key, which is true and was overruled: the frame gives the empty
   * dashboard one action, on the card above, and nothing beside it to compete.
   *
   * What it still guards is the part that matters — the absence is rendered as
   * a deliberate empty state and never as a failure.
   */
  it('renders no key as two empty tiles, not as an error', async () => {
    signedInWithoutAnyKey();
    renderApp();

    // The key card's own state is what proves the page got there.
    await screen.findByTestId('no-key-notice');

    // Both panels keep their titles and lose their bodies.
    expect(screen.getByText('Monthly Usage')).toBeTruthy();
    expect(screen.getByText('Rate Limit')).toBeTruthy();
    expect(screen.queryByText(/request per second/i)).toBeNull();
    expect(screen.queryByTestId('usage-used')).toBeNull();
    // "Active" is a claim about a key, and there is none.
    expect(screen.queryByText('Active')).toBeNull();
    // And nothing anywhere reads as a failure.
    expect(document.body.textContent).not.toContain('Could not load');
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
  });

  /**
   * Straight after 0189's issue round-trip, the backend's short cache can
   * still answer "no key" about a key the page is displaying. "You have no
   * API key yet" would be false at that moment, so the page says what is
   * actually happening instead. Driven entirely by the mount fetches: the
   * key route knows the key, the usage route's cache does not yet.
   */
  it('does not claim "no key" while a fresh key is on screen', async () => {
    stubRoutes({
      [CONFIG_URL]: openConfig,
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
        }),
      }),
      [USAGE_URL]: usageNoKey,
    });
    renderApp();

    await screen.findByTestId('api-key');
    expect(await screen.findByText(/your key is new/i)).toBeTruthy();
    expect(screen.queryByText(/no API key yet/i)).toBeNull();
  });

  /**
   * A key revealed on mount must not blank a usage section that is showing
   * numbers: a reveal changes no counter, the backend's cache would answer an
   * identical body, and a loading flicker for an identical answer reads as
   * breakage. The keyed refetch is for the no-key state alone.
   */
  it('does not refetch or blank rendered numbers when an existing key is revealed', async () => {
    const fetchMock = stubRoutes({
      [CONFIG_URL]: openConfig,
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
        }),
      }),
      [USAGE_URL]: () => ({ json: async () => USAGE }),
    });
    renderApp();

    await screen.findByTestId('usage-used');
    await screen.findByTestId('api-key');

    // The numbers are on screen, and one usage request explains everything.
    expect(screen.getByTestId('usage-used').textContent).toBe('121');
    expect(
      fetchMock.mock.calls.filter(([url]) => url === USAGE_URL).length,
    ).toBe(1);
  });

  /**
   * The press and the mount-time usage fetch race, and the press can win.
   *
   * Watching the `keyOnScreen` transition alone was not enough: with the
   * mount-time fetch still in flight the view is `'loading'` when the
   * transition happens, so there was nothing to refetch out of — and the
   * effect never ran again, because its dependencies had already settled. The
   * in-flight request, issued before the key existed, then resolved `no_key`
   * and the section sat on "your key is new" until the visitor found Refresh.
   *
   * The usage response is deferred through `json()` rather than through
   * `fetch` itself, which is what puts the request in the state this covers:
   * `fetch` has resolved, the page is still awaiting the body.
   */
  it('refetches usage when the key is issued before the first load answers', async () => {
    let answerTheFirstLoad: () => void = () => undefined;
    const firstLoad = new Promise<{ code: string; message: string }>(
      (resolve) => {
        answerTheFirstLoad = () =>
          resolve({ code: 'no_key', message: 'you have no API key yet' });
      },
    );

    let usageCalls = 0;
    const fetchMock = stubRoutes({
      [CONFIG_URL]: openConfig,
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
        }),
      }),
      [USAGE_URL]: () => {
        usageCalls += 1;
        // The load that was in flight when the key was issued snapshotted a
        // keyless account, so it answers `no_key`. Everything after it sees
        // the key.
        return usageCalls === 1
          ? { ok: false, status: 404, json: () => firstLoad }
          : { json: async () => USAGE };
      },
    });
    renderApp();

    // The key lands from the mount-time reveal while the first usage load is
    // still awaiting its body. Since task 0189 there is no press to make here:
    // `/key` is read-only and fetched on mount, and issuing is an OAuth
    // round-trip that lands back on this page with `?issue=ok`. The race the
    // test covers is unchanged — a key on screen before usage has answered.
    await screen.findByTestId('api-key');
    expect(usageCalls).toBe(1);

    answerTheFirstLoad();

    // No Refresh press anywhere in this test: the section recovers on its own.
    expect((await screen.findByTestId('usage-used')).textContent).toBe('121');
    expect(screen.queryByText(/your key is new/i)).toBeNull();
    expect(
      fetchMock.mock.calls.filter(([url]) => url === USAGE_URL).length,
    ).toBe(2);
  });

  /**
   * And it fires at most once, which is what lets the effect above watch the
   * view state at all. The backend can legitimately keep answering `no_key`
   * while its own short cache catches up, so a refetch that re-triggered on
   * its own result would be a fetch loop against the control plane.
   */
  it('does not loop when the refetch is answered no_key again', async () => {
    const fetchMock = stubRoutes({
      [CONFIG_URL]: openConfig,
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
        }),
      }),
      [USAGE_URL]: usageNoKey,
    });
    renderApp();

    await screen.findByTestId('api-key');
    await screen.findByText(/your key is new/i);

    // One load on mount, one refetch once the key is on screen, then it stops.
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.filter(([url]) => url === USAGE_URL).length,
      ).toBe(2),
    );
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(
      fetchMock.mock.calls.filter(([url]) => url === USAGE_URL).length,
    ).toBe(2);
  });

  /**
   * The rate limit is the gateway's, not this bundle's (task 0188).
   *
   * `pricingApiFreePlanRateLimit` is a per-env config value that
   * `api-gateway-stack.ts` hands to `addUsagePlan` and `compute-stack.ts` hands
   * to the backend. Raising it and deploying has to change what this panel
   * says — with a literal here it would not, and the one section whose stated
   * theme is rendering honestly would be quietly stating a limit nobody
   * enforces any more.
   */
  it('states the rate limit the backend reports, not a built-in figure', async () => {
    stubRoutes({
      [CONFIG_URL]: () => ({
        json: async () => ({ enabled: true, rate_limit_per_second: 5 }),
      }),
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [USAGE_URL]: () => ({ json: async () => USAGE }),
    });
    renderApp();

    await screen.findByTestId('usage-used');
    expect(screen.getByTestId('rate-limit').textContent).toBe('5');
    // Plural, because the figure is no longer the one the sentence was
    // written around.
    expect(screen.getByText(/requests per second/i)).toBeTruthy();
  });

  /**
   * A deployment that did not say what the limit is says nothing about it. A
   * fallback figure would be the same silent staleness one layer down — and
   * unlike the missing line, it would look authoritative.
   */
  it('falls back to the plan rate rather than dropping the Rate Limit card', async () => {
    stubRoutes({
      [CONFIG_URL]: () => ({ json: async () => ({ enabled: true }) }),
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [USAGE_URL]: () => ({ json: async () => USAGE }),
    });
    renderApp();

    // ⚠️ The OPPOSITE of what this pinned until 2026-08-25, when Adam found the
    // whole Rate Limit card missing on a local run. `/config` without a limit
    // used to drop the panel; it now shows the free plan's documented 1 req/s
    // (task 0157), the same figure the landing page states to every visitor.
    // A stated figure beats a third of the dashboard disappearing — and where
    // the deployment DOES answer, its value still wins (the test above).
    await screen.findByTestId('usage-used');
    expect((await screen.findByTestId('rate-limit')).textContent).toBe('1');
    expect(screen.getByText(/per-minute limit/i)).toBeTruthy();
    expect(screen.getByText(/request per second/i)).toBeTruthy();
  });

  /**
   * The backend writes a message for each failure it authors; the page shows
   * it (task 0188).
   *
   * A throttle with nothing cached is the case the backend has a distinct 503
   * for, and it is the one where a bare status code helps least: "try again in
   * a moment" is actionable, `answered 503` is a number. Discarding the
   * envelope also made the longer usage timeout pointless — the extra seconds
   * exist to let this answer arrive.
   */
  it("shows the backend's reason for a failed usage read", async () => {
    signedInWithUsage(() => ({
      ok: false,
      status: 503,
      json: async () => ({
        code: 'usage_unavailable',
        message:
          'AWS is rate-limiting the usage lookup right now; try again in a moment',
      }),
    }));
    renderApp();

    expect(
      await screen.findByText(/rate-limiting the usage lookup/i),
    ).toBeTruthy();
    expect(screen.queryByText(/answered 503/i)).toBeNull();
  });

  /**
   * With no envelope to read there is nothing to forward, and the URL and
   * status are then the most specific true thing there is to say. Task 0183's
   * gate answers exactly this: an EMPTY 404, byte-identical to an unrouted
   * path — which must never be read as "you have no API key".
   */
  it('falls back to the status when a failure carries no message', async () => {
    signedInWithUsage(() => ({
      ok: false,
      status: 404,
      json: async () => {
        throw new SyntaxError('Unexpected end of JSON input');
      },
    }));
    renderApp();

    expect(await screen.findByText(/answered 404/i)).toBeTruthy();
    expect(screen.queryByText(/no API key yet/i)).toBeNull();
  });

  /** The refresh control re-asks; the backend's cache bounds what that costs. */
  /**
   * ⚠️ **DELETED with the control.** The usage card had a Refresh button and
   * this pinned that pressing it re-read `/usage`; Adam removed it on
   * 2026-08-25 with the two lines beside it. The panel still refetches on its
   * own — on mount, and when a key appears on screen (the tests above) — but a
   * visitor who wants a fresher figure now reloads the page.
   */

  /** A backend failure is a stated failure, not a blank section. */
  it('reports a failure rather than an empty card', async () => {
    signedInWithUsage(() => ({ ok: false, status: 502 }));
    renderApp();

    expect(await screen.findByText(/could not load your usage/i)).toBeTruthy();
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

describe('replace my key', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const KEY = {
    key_id: 'abc123',
    name: 'discord-308994132968210433-key',
    value: 'aBcDeF0123456789aBcDeF0123456789aBcDeF01',
  };

  const REVOKED = {
    revoked: true,
    next_eligible_at: '2026-09-01T00:00:00Z',
    revoked_at: '2026-08-21T12:00:00Z',
  };

  /** The reveal's answer once the key is revoked (task 0191). */
  const keyRevoked = () => ({
    ok: false,
    status: 404,
    json: async () => ({
      code: 'key_revoked',
      message: 'your API key was revoked',
      details: {
        next_eligible_at: '2026-09-01T00:00:00Z',
        revoked_at: '2026-08-21T12:00:00Z',
      },
    }),
  });

  const signedIn = (
    revoke: () => Partial<Response> & { json?: () => unknown } = () => ({
      json: async () => REVOKED,
    }),
    key: () => Partial<Response> & { json?: () => unknown } = () => ({
      json: async () => KEY,
    }),
  ) =>
    stubRoutes({
      [CONFIG_URL]: openConfig,
      [ME_URL]: () => ({
        json: async () => ({
          authenticated: true,
          user_id: '308994132968210433',
          username: 'adam',
        }),
      }),
      [KEY_URL]: key,
      [USAGE_URL]: usageNoKey,
      [REWORK_URL]: revoke,
    });

  const renderApp = (entry = '/') =>
    render(
      <MemoryRouter initialEntries={[entry]}>
        <App />
        <LocationSpy />
      </MemoryRouter>,
    );

  const openDialog = async () => {
    fireEvent.click(await screen.findByTestId('replace-key-open'));
    return screen.findByTestId('replace-key-dialog');
  };

  const revokeCalls = (fetchMock: ReturnType<typeof stubRoutes>) =>
    fetchMock.mock.calls.filter(([url]) => url === REWORK_URL);

  /**
   * The control lives beside the key and nowhere else, and pressing it opens
   * a confirmation: nothing is sent to the backend until the armed confirm.
   */
  it('offers replacement only beside an existing key, and opening the dialog sends nothing', async () => {
    const fetchMock = signedIn();
    renderApp();

    await openDialog();
    expect(revokeCalls(fetchMock)).toHaveLength(0);
  });

  /**
   * Task 0193 made it a real modal rather than a section spliced into the
   * page: the frame draws a floating card over a dimmed dashboard, and the
   * properties that come with that — an accessible name, `aria-modal`, and a
   * confirm carrying the frame's verb — are what a visitor and a screen
   * reader both need to know which decision they are being asked for.
   */
  it('opens as a modal dialog named for the action, with the frame verb on the confirm', async () => {
    signedIn();
    renderApp();

    await openDialog();
    const dialog = screen.getByRole('dialog', {
      name: /regenerate api key\?/i,
    });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(
      (screen.getByTestId('replace-key-confirm') as HTMLButtonElement)
        .textContent,
    ).toMatch(/^regenerate$/i);
  });

  it('does not offer replacement to a visitor with no key', async () => {
    signedIn(undefined, keyNoKey);
    renderApp();

    await screen.findByRole('link', { name: /generate api key/i });
    expect(screen.queryByTestId('replace-key-open')).toBeNull();
  });

  /**
   * The wording is the acceptance criterion: the key is deactivated
   * immediately, and no new key is issued until the next period.
   */
  it('states that the key is deactivated, names the propagation window, and says no new key is issued until the next period', async () => {
    signedIn();
    renderApp();

    await openDialog();
    const warning = await screen.findByTestId('replace-key-warning');
    expect(warning.textContent).toMatch(/deactivates the current one/i);
    // The 0192 criterion: no claim of immediacy the data plane does not
    // have. ~25 s measured (0180 item 8); the copy says half a minute and
    // tells the visitor to treat the key as live until then.
    expect(warning.textContent).toMatch(/within about half a minute/i);
    expect(warning.textContent).toMatch(/treat it as live until then/i);
    expect(warning.textContent).not.toMatch(/immediately/i);
    expect(warning.textContent).toMatch(/will break/i);
    expect(warning.textContent).toMatch(/no new key is issued now/i);
    expect(warning.textContent).toMatch(/next quota period/i);
  });

  /**
   * Confirm is disabled until `regenerate-key` is typed — exactly that
   * phrase. The string is Adam's (2026-08-26, following the button's word);
   * the property task 0191 decided is the one asserted here, that a
   * near-miss never arms the control and never reaches the backend.
   */
  it('keeps confirm disabled until the visitor types regenerate-key', async () => {
    const fetchMock = signedIn();
    renderApp();

    await openDialog();
    const confirm = (await screen.findByTestId(
      'replace-key-confirm',
    )) as HTMLButtonElement;
    const phrase = screen.getByTestId('replace-key-phrase') as HTMLInputElement;
    expect(confirm.disabled).toBe(true);

    for (const typed of [
      'regenerate',
      'regenerate key',
      'REGENERATE-KEY',
      'regenerate-key ',
    ]) {
      fireEvent.change(phrase, { target: { value: typed } });
      expect(confirm.disabled, typed).toBe(true);
      fireEvent.click(confirm);
    }
    expect(revokeCalls(fetchMock)).toHaveLength(0);

    fireEvent.change(phrase, { target: { value: 'regenerate-key' } });
    expect(confirm.disabled).toBe(false);
  });

  /**
   * One armed press is one `POST`; the button is disabled again on submit so
   * a double-click cannot fire two. On success the panel shows the revoked
   * state with the backend's date, and the old value is gone from the page.
   */
  it('revokes with one POST, cannot double-fire, and renders the revoked state with the date', async () => {
    const fetchMock = signedIn();
    renderApp();

    await openDialog();
    const confirm = (await screen.findByTestId(
      'replace-key-confirm',
    )) as HTMLButtonElement;
    fireEvent.change(screen.getByTestId('replace-key-phrase'), {
      target: { value: 'regenerate-key' },
    });

    fireEvent.click(confirm);
    expect(confirm.disabled).toBe(true);
    fireEvent.click(confirm);
    fireEvent.click(confirm);

    const revoked = await screen.findByTestId('key-revoked');
    expect(revoked.textContent).toMatch(/deactivated/i);
    // The revocation instant from the API, in UTC, is the anchor for the
    // propagation window — not "now", not the viewer's zone.
    expect(screen.getByTestId('revoked-at').textContent).toBe(
      '21 August 2026, 12:00 UTC',
    );
    expect(revoked.textContent).toMatch(/within about half a minute/i);
    expect(revoked.textContent).toMatch(/1 September 2026/);
    expect(revoked.textContent).toMatch(/do not have a working key/i);
    expect(revokeCalls(fetchMock)).toHaveLength(1);
    expect((revokeCalls(fetchMock)[0][1] as RequestInit).method).toBe('POST');
    // The CSRF marker: a non-simple header the backend requires and a
    // cross-origin page cannot send without a preflight.
    expect(
      (
        (revokeCalls(fetchMock)[0][1] as RequestInit).headers as Record<
          string,
          string
        >
      )['x-requested-with'],
    ).toBe('stellar-prices-portal');
    expect(revokeCalls(fetchMock)[0][0].startsWith('http')).toBe(false);
    expect(screen.queryByTestId('api-key')).toBeNull();
    expect(document.body.textContent).not.toContain(KEY.value);
    // And no issue link while the date is ahead.
    expect(screen.queryByRole('link', { name: /get my api key/i })).toBeNull();
  });

  /**
   * A partial revocation warns instead of reading as a clean one.
   *
   * The backend answers `200 partial` when it disabled some keys under the
   * name and failed on another — the visitor's own key is off, so a `502`
   * would be false, but a duplicate may still answer on `/v1/` and a plain
   * "revoked" would be false too. The warning is what tells them the next move
   * is to press Replace again, not to wait for the 1st.
   */
  it('warns on a partial revocation instead of calling it revoked', async () => {
    signedIn(() => ({
      json: async () => ({ ...REVOKED, partial: true }),
    }));
    renderApp();

    await openDialog();
    fireEvent.change(screen.getByTestId('replace-key-phrase'), {
      target: { value: 'regenerate-key' },
    });
    fireEvent.click(screen.getByTestId('replace-key-confirm'));

    const warning = await screen.findByTestId('revoke-partial');
    expect(warning.textContent).toMatch(/could not be deactivated/i);
    expect(warning.textContent).toMatch(/may still work/i);
    // The dates still render — the disables that landed are real.
    expect(screen.getByTestId('revoked-at')).toBeTruthy();
    // But NOT the cap sentences: the surviving duplicate is a working key,
    // and the issue path adopts it rather than refusing, so both "you do not
    // have a working key" and the next-eligible date would be false here.
    const revoked = screen.getByTestId('key-revoked');
    expect(revoked.textContent).not.toMatch(/do not have a working key/i);
    expect(revoked.textContent).not.toMatch(/1 September 2026/);
    expect(screen.queryByRole('link', { name: /get my api key/i })).toBeNull();
  });

  /** The ordinary answer carries no flag, and renders no warning. */
  it('renders no partial warning on a clean revocation', async () => {
    signedIn();
    renderApp();

    await openDialog();
    fireEvent.change(screen.getByTestId('replace-key-phrase'), {
      target: { value: 'regenerate-key' },
    });
    fireEvent.click(screen.getByTestId('replace-key-confirm'));

    await screen.findByTestId('key-revoked');
    expect(screen.queryByTestId('revoke-partial')).toBeNull();
  });

  /**
   * A failed revoke says so and keeps the key — "revoked" is never false.
   *
   * And it does not make the opposite claim either: a `502` can be a refusal
   * with nothing written or a lost response on a patch that landed, so the copy
   * says the deactivation was not CONFIRMED rather than that the key is still
   * active.
   */
  it('reports a failed revoke without claiming the key is still active', async () => {
    signedIn(() => ({
      ok: false,
      status: 502,
      json: async () => ({
        code: 'key_unavailable',
        message: 'could not reach the API key service; try again',
      }),
    }));
    renderApp();

    await openDialog();
    fireEvent.change(screen.getByTestId('replace-key-phrase'), {
      target: { value: 'regenerate-key' },
    });
    fireEvent.click(screen.getByTestId('replace-key-confirm'));

    const failed = await screen.findByTestId('replace-key-failed');
    expect(failed.textContent).toMatch(/could not confirm the deactivation/i);
    expect(failed.textContent).not.toMatch(/it is still active/i);
    expect(failed.textContent).toMatch(/could not reach the API key service/i);
    expect(screen.queryByTestId('key-revoked')).toBeNull();
    expect(screen.getByTestId('api-key')).toBeTruthy();
    // The dialog stays, re-armed, for a retry.
    expect(
      (screen.getByTestId('replace-key-confirm') as HTMLButtonElement).disabled,
    ).toBe(false);
  });

  it('closes without sending anything', async () => {
    const fetchMock = signedIn();
    renderApp();

    await openDialog();
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }));
    expect(screen.queryByTestId('replace-key-dialog')).toBeNull();
    expect(screen.getByTestId('replace-key-open')).toBeTruthy();
    expect(revokeCalls(fetchMock)).toHaveLength(0);
  });

  // -------------------------------------------------------------------------
  // The revoked state on load, and the capped issue landing
  // -------------------------------------------------------------------------

  /**
   * A reload after a revoke: the reveal answers `key_revoked`, the page
   * renders the date and never the value, and offers no issue link while the
   * date is ahead.
   */
  it('renders a revoked key as revoked with the date, not as "no key"', async () => {
    signedIn(undefined, keyRevoked);
    renderApp();

    const revoked = await screen.findByTestId('key-revoked');
    expect(revoked.textContent).toMatch(/1 September 2026/);
    expect(screen.queryByRole('link', { name: /get my api key/i })).toBeNull();
    expect(screen.queryByTestId('replace-key-open')).toBeNull();
    // The key section's keyless copy (and its issue link) must not render;
    // the usage section beside it may still say "no key" — that is its own
    // endpoint's answer, stubbed here, not the key section's.
    expect(screen.queryByText(/one key, on the free plan/i)).toBeNull();
  });

  /**
   * After an in-page revoke the usage section stops describing the key as
   * new, re-asks the backend (which evicted its cache), and — when AWS has no
   * row — says the key was deactivated rather than "issue one above".
   */
  it('tells the usage section about an in-page revoke', async () => {
    const fetchMock = signedIn();
    renderApp();

    // Before: a key on screen, usage says no key → "your key is new".
    await screen.findByTestId('api-key');
    expect(await screen.findByText(/your key is new/i)).toBeTruthy();
    const usageCallsBefore = fetchMock.mock.calls.filter(
      ([url]) => url === USAGE_URL,
    ).length;

    await openDialog();
    fireEvent.change(screen.getByTestId('replace-key-phrase'), {
      target: { value: 'regenerate-key' },
    });
    fireEvent.click(screen.getByTestId('replace-key-confirm'));
    await screen.findByTestId('key-revoked');

    expect(await screen.findByTestId('usage-after-revoke')).toBeTruthy();
    expect(screen.queryByText(/your key is new/i)).toBeNull();
    expect(document.body.textContent).not.toMatch(/issue one above/i);
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.filter(([url]) => url === USAGE_URL).length,
      ).toBe(usageCallsBefore + 1),
    );
  });

  /**
   * ⚠️ **Signing in to an account whose key is already revoked replaces the
   * whole dashboard with one card** (Adam, 2026-08-26, Figma `997:2114`).
   *
   * Three boundaries are asserted here rather than the pixels, because each
   * one is a way this could quietly swallow something a visitor needs:
   * the card must carry task 0191's decided sentence, it must not appear for a
   * revocation that happened in THIS page load, and it must not appear on a
   * landing that carries an `?issue=…` outcome it cannot explain.
   */
  it('replaces the dashboard for an account that arrives already revoked', async () => {
    signedIn(undefined, keyRevoked);
    renderApp();

    const card = await screen.findByTestId('key-revoked');
    expect(
      screen.getByRole('heading', { name: /your api key has been revoked/i }),
    ).toBeTruthy();
    expect(screen.getByTestId('revoked-next-eligible').textContent).toMatch(
      /1 September 2026/,
    );
    // The frame's Reason, verbatim.
    expect(screen.getByTestId('revoked-reason').textContent).toMatch(
      /monthly quota exceeded repeatedly/i,
    );
    // Both of the frame's actions, and the footer's date.
    expect(screen.getByTestId('revoked-contact')).toBeTruthy();
    expect(screen.getByTestId('revoked-sign-out')).toBeTruthy();
    expect(card.textContent).toMatch(/After 1 September 2026, sign in again/i);
    // No key panel behind it: nothing to copy, reveal or regenerate.
    expect(screen.queryByTestId('api-key')).toBeNull();
    expect(screen.queryByTestId('replace-key-open')).toBeNull();
  });

  /**
   * The account is not stuck: once the period has rolled, the frame's
   * "Contact us about this decision" gives way to the control that actually
   * issues a key. The frame's footer says one arrives automatically on signing
   * in, which nothing implements.
   */
  it('offers the issue round-trip from the revoked card once the period has passed', async () => {
    signedIn(undefined, () => ({
      ok: false,
      status: 404,
      json: async () => ({
        code: 'key_revoked',
        details: { next_eligible_at: '2020-01-01T00:00:00Z' },
      }),
    }));
    renderApp();

    await screen.findByTestId('key-revoked');
    expect(screen.getByTestId('revoked-issue').getAttribute('href')).toBe(
      ISSUE_HREF,
    );
    expect(screen.queryByTestId('revoked-contact')).toBeNull();
  });

  /**
   * The `?issue=…` outcomes are reachable BY a revoked account — `capped` is
   * the obvious one, but `not_member`, `too_young` and `unknown` all are — and
   * each says something this card cannot. A landing carrying one keeps the
   * ordinary dashboard.
   */
  it('keeps the ordinary dashboard when the landing carries an issue outcome', async () => {
    signedIn(undefined, keyRevoked);
    renderApp('/?issue=capped&next_eligible_at=2026-09-01');

    expect(await screen.findByTestId('issue-capped')).toBeTruthy();
    expect(
      screen.queryByRole('heading', { name: /your api key has been revoked/i }),
    ).toBeNull();
  });

  /**
   * A malformed `next_eligible_at` must NOT unlock the issue link — the safe
   * direction for garbage is to keep waiting (the server would only refuse).
   */
  it('keeps the issue link hidden when the revoked date is unparseable', async () => {
    signedIn(undefined, () => ({
      ok: false,
      status: 404,
      json: async () => ({
        code: 'key_revoked',
        details: { next_eligible_at: 'not-a-date' },
      }),
    }));
    renderApp();

    await screen.findByTestId('key-revoked');
    expect(screen.queryByRole('link', { name: /get my api key/i })).toBeNull();
    expect(document.body.textContent).toMatch(
      /start of the next quota period/i,
    );
  });

  /** Once the date has passed, the issue link comes back. */
  it('offers the issue round-trip once the revocation period has passed', async () => {
    signedIn(undefined, () => ({
      ok: false,
      status: 404,
      json: async () => ({
        code: 'key_revoked',
        details: { next_eligible_at: '2020-01-01T00:00:00Z' },
      }),
    }));
    renderApp();

    await screen.findByTestId('key-revoked');
    const link = await screen.findByRole('link', { name: /get my api key/i });
    expect(link.getAttribute('href')).toBe(ISSUE_HREF);
  });

  /**
   * A revocation the backend cannot date renders WITHOUT an instant — never
   * "deactivated on just now", and never the next-eligible phrase presented
   * as the revocation instant.
   */
  /** Same move as the test above, and for the same reason. */
  it('renders an undated revocation without inventing an instant', async () => {
    signedIn(() => ({
      json: async () => ({
        revoked: true,
        next_eligible_at: '2026-09-01T00:00:00Z',
      }),
    }));
    renderApp();

    await openDialog();
    fireEvent.change(screen.getByTestId('replace-key-phrase'), {
      target: { value: 'regenerate-key' },
    });
    fireEvent.click(screen.getByTestId('replace-key-confirm'));

    const revoked = await screen.findByTestId('key-revoked');
    expect(revoked.textContent).toMatch(/deactivated/i);
    expect(screen.queryByTestId('revoked-at')).toBeNull();
    expect(revoked.textContent).not.toMatch(/just now/i);
    expect(revoked.textContent).not.toMatch(
      /deactivated on the start of the next quota period/i,
    );
    // The date it CAN be re-issued is still named — that one is known.
    expect(revoked.textContent).toMatch(/1 September 2026/);
  });

  /**
   * The propagation window is a statement about now. Rendered on a page load
   * days after the revocation (the reveal path), it must not tell the owner
   * of a long-dead key to keep treating it as live.
   */
  /**
   * ⚠️ Driven through the in-page revoke since 2026-08-26. Task 0191's
   * sentence lives on that path and on the partial one; an account that
   * ARRIVES already revoked now gets the frame's card
   * (`RevokedDashboard`), whose Reason box is a fixed string. The property
   * being pinned — the tense follows the propagation window — is unchanged.
   */
  it('states the propagation window in the past tense for an old revocation', async () => {
    signedIn(() => ({
      json: async () => ({
        revoked: true,
        next_eligible_at: '2026-09-01T00:00:00Z',
        revoked_at: '2026-08-01T09:00:00Z',
      }),
    }));
    renderApp();

    await openDialog();
    fireEvent.change(screen.getByTestId('replace-key-phrase'), {
      target: { value: 'regenerate-key' },
    });
    fireEvent.click(screen.getByTestId('replace-key-confirm'));

    const revoked = await screen.findByTestId('key-revoked');
    expect(screen.getByTestId('revoked-at').textContent).toBe(
      '1 August 2026, 09:00 UTC',
    );
    expect(revoked.textContent).toMatch(/stopped working/i);
    expect(revoked.textContent).not.toMatch(/treat it as live/i);
    // The measured window is still named — only the tense changed.
    expect(revoked.textContent).toMatch(/within about half a minute/i);
  });

  /**
   * An idempotent revoke whose period has already rolled answers with a
   * next-eligible instant of *now*, and the page offers the round-trip
   * rather than a date a month out.
   */
  it('offers the issue link when the revoke answers a next eligible date already passed', async () => {
    signedIn(() => ({
      json: async () => ({
        revoked: true,
        next_eligible_at: '2020-01-01T00:00:00Z',
        revoked_at: '2019-12-03T12:00:00Z',
      }),
    }));
    renderApp();

    await openDialog();
    fireEvent.change(screen.getByTestId('replace-key-phrase'), {
      target: { value: 'regenerate-key' },
    });
    fireEvent.click(screen.getByTestId('replace-key-confirm'));

    await screen.findByTestId('key-revoked');
    const link = await screen.findByRole('link', { name: /get my api key/i });
    expect(link.getAttribute('href')).toBe(ISSUE_HREF);
  });

  /** The capped issue landing renders the date, sanitised, and no link. */
  it('renders a capped issue with the next eligible date', async () => {
    signedIn(undefined, keyRevoked);
    renderApp('/?issue=capped&next_eligible_at=2026-09-01');

    const capped = await screen.findByTestId('issue-capped');
    expect(capped.textContent).toMatch(/1 September 2026/);
    expect(capped.textContent).toMatch(/do not have a working key/i);
    expect(capped.querySelector('a')).toBeNull();
  });

  it('sanitises a nonsense next_eligible_at instead of rendering it', async () => {
    signedIn(undefined, keyRevoked);
    renderApp('/?issue=capped&next_eligible_at=<script>nope');

    const capped = await screen.findByTestId('issue-capped');
    expect(capped.textContent).toMatch(/start of the next quota period/i);
    expect(capped.textContent).not.toContain('<script>');
  });

  /** One-shot, like every other landing param: stripped from the URL. */
  it('clears the capped outcome from the URL so a reload does not repeat it', async () => {
    signedIn(undefined, keyRevoked);
    renderApp('/?issue=capped&next_eligible_at=2026-09-01');

    await screen.findByTestId('issue-capped');
    await waitFor(() => expect(lastSearch).toBe(''));
    expect(screen.getByTestId('issue-capped')).toBeTruthy();
  });
});
