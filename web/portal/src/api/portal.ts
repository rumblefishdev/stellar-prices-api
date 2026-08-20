/**
 * The portal's own backend calls.
 *
 * Every URL here is **relative and same-origin**, which is the property task
 * 0184 bought by putting the API on the same CloudFront distribution as this
 * bundle: no base URL to configure, no CORS preflight, and task 0186's session
 * cookie can be `SameSite=Lax`. Do not reintroduce an absolute base — an
 * absolute URL here would silently undo all three.
 */

/** Mirrors `PORTAL_API_PREFIX` in `packages/prices-api/src/portal/mod.rs`. */
const PORTAL_API = '/api-tokens/api';

/**
 * What the backend tells the bundle before it renders anything.
 *
 * Hand-written rather than generated, and it is the one type in this app that
 * should be: the portal's routes are **deliberately absent** from the published
 * OpenAPI document (`tools/scripts/verify-openapi-routes.mjs` fails CI if one
 * appears in it), because the document describes the public data API to
 * integrators and the portal describes itself to its own bundle. There is
 * therefore nothing to generate this from, and no generated file in this
 * directory to defer to — `npm run portal:api-types` exists and emits
 * `src/api/generated.ts` from the published document, but that document covers
 * only the `/v1` data API, so nothing here imports it and it is not checked in.
 *
 * Keep it in step with `PortalConfig` in `portal/mod.rs`.
 */
export interface PortalConfig {
  /** Whether the portal is open for business (task 0183's `PORTAL_ENABLED`). */
  enabled: boolean;
}

/**
 * What `GET /api-tokens/api/auth/me` answers (task 0186).
 *
 * Mirrors `MeResponse` in `packages/prices-api/src/portal/auth/mod.rs`. Same
 * reason it is hand-written as `PortalConfig` above: the portal's routes are
 * deliberately absent from the published OpenAPI document, so there is nothing
 * to generate it from.
 *
 * `authenticated: false` is a `200`, not a `401` — being signed out is an
 * answer, not a refusal, and this page renders plain text for it.
 */
export interface PortalSession {
  authenticated: boolean;
  /** The Discord user ID (a snowflake). Present only when authenticated. */
  user_id?: string;
  /** The Discord username, for display. Present only when authenticated. */
  username?: string;
}

export class PortalApiError extends Error {
  constructor(
    message: string,
    readonly status?: number,
  ) {
    super(message);
    this.name = 'PortalApiError';
  }
}

/**
 * How long the page waits before calling the backend unreachable.
 *
 * `fetch` has no default timeout, and neither the gateway's 29s cap nor the
 * Lambda's own limit bounds a connection that never reaches an origin — a
 * stalled TCP handshake or a proxy that accepts and then says nothing leaves the
 * promise pending forever. Without this the page sits on "Checking whether the
 * portal is open…" with no end, which is exactly the spinner that never resolves
 * the failure branch exists to avoid.
 *
 * Ten seconds is well past a cold Lambda behind this route (the handler reads a
 * cached SSM parameter and returns a single boolean) and well short of a
 * visitor's patience.
 */
const PROBE_TIMEOUT_MS = 10_000;

/**
 * How long the page waits on the key reveal.
 *
 * Longer than {@link PROBE_TIMEOUT_MS}, because this call is not a probe: a cold
 * Lambda resolves credentials, reads an SSM parameter and then walks a
 * paginated listing plus a read. The api-handler's own Lambda timeout is 15s
 * and API Gateway cuts everything off at 29s, so 20s is inside the window
 * where an answer — including a `502` — is still possible.
 */
const KEY_TIMEOUT_MS = 20_000;

/**
 * How long the page waits on the usage read.
 *
 * Between the other two, because the call is: the backend's own wall-clock
 * deadline on the lookup is 10s (`USAGE_DEADLINE` in `portal/usage/mod.rs`),
 * after which it answers a `503` that names the condition. Waiting only
 * {@link PROBE_TIMEOUT_MS} would tie with that deadline and the page would
 * report its own timeout instead of the backend's more useful answer; 15s
 * leaves the answer time to arrive while staying inside the gateway's 29s cap.
 */
const USAGE_TIMEOUT_MS = 15_000;

/** Whether a rejection from `fetch` is this timeout firing. See the call site. */
const isTimeout = (error: unknown): boolean =>
  typeof error === 'object' &&
  error !== null &&
  (error as { name?: unknown }).name === 'TimeoutError';

async function getJson<T>(url: string): Promise<T> {
  let response: Response;
  try {
    response = await fetch(url, {
      headers: { accept: 'application/json' },
      signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    });
  } catch (error) {
    // Distinguished from the generic failure below because the two point at
    // different causes: a timeout means something accepted the connection and
    // then said nothing, which is a gateway or origin problem, whereas the
    // generic branch is usually the viewer's own network.
    //
    // Matched on `name` alone, NOT `instanceof DOMException` or
    // `instanceof Error`. What `AbortSignal.timeout` rejects with comes from the
    // platform, and an `instanceof` against it is a same-realm test: it is false
    // across an iframe, and it is false under jsdom, where the DOMException the
    // environment provides does not descend from the test realm's `Error`. That
    // is not a test artefact to work around — it is the same fragility in a
    // place where it can be seen.
    if (isTimeout(error)) {
      throw new PortalApiError(
        `${url} did not answer within ${PROBE_TIMEOUT_MS / 1000}s`,
      );
    }
    // A network-level failure, not an HTTP status — offline, DNS, TLS. The
    // browser's own message is deliberately vague for privacy reasons, so
    // there is nothing worth forwarding from it.
    throw new PortalApiError(`${url} could not be reached`);
  }
  if (!response.ok) {
    throw new PortalApiError(
      `${url} answered ${response.status}`,
      response.status,
    );
  }
  try {
    return (await response.json()) as T;
  } catch {
    // A `200` that is not JSON is the signature of the most likely routing
    // regression there is here: if the `/api-tokens/api/*` behaviour ever stops
    // winning over `/api-tokens/*` (see `portal-hosting-stack.ts`, which fails
    // CI on that ordering), CloudFront answers this call with the SPA bundle as
    // `200 text/html`. Left unwrapped, that surfaces as a bare `SyntaxError`
    // about an unexpected `<` — no status, no URL, and no hint that the cause is
    // a routing table. Carry the status so the page can say which URL lied.
    throw new PortalApiError(
      `${url} answered ${response.status}, not JSON`,
      response.status,
    );
  }
}

/**
 * `GET /api-tokens/api/config` — the one route that answers while the portal is
 * closed.
 *
 * Task 0183 gates the whole prefix to an empty `404`, byte-identical to a path
 * that was never deployed, and exempts exactly this path so the bundle can ask
 * whether to render the real UI or a "not yet available" page. That makes it the
 * only honest liveness probe this app has, and the reason the page below uses it
 * rather than the `/api-tokens/api/health` named in task 0185's criteria — that
 * route does not exist, and if it did the gate would answer `404` on it.
 */
export const fetchPortalConfig = (): Promise<PortalConfig> =>
  getJson<PortalConfig>(`${PORTAL_API}/config`);

/**
 * `GET /api-tokens/api/auth/me` — who the browser is talking as (task 0186).
 *
 * No credential is passed here and none could be: the session is an `HttpOnly`
 * cookie, so this code cannot read it and does not need to. The browser attaches
 * it because the request is **same-origin** — the property task 0184 bought and
 * that `PORTAL_API` being a relative path preserves. An absolute URL here would
 * make it cross-site, at which point `SameSite=Lax` withholds the cookie and
 * every visitor reads as signed out.
 */
export const fetchSession = (): Promise<PortalSession> =>
  getJson<PortalSession>(`${PORTAL_API}/auth/me`);

/**
 * Where the "Sign in with Discord" control points.
 *
 * A **plain link**, not a `fetch`. The flow is three top-level navigations —
 * here, to Discord, back to the callback — and `fetch` cannot perform any of
 * them: it would follow the redirect to discord.com as a cross-origin request,
 * with no consent screen the visitor can see and no way to come back. This is
 * also why the session cookie is `SameSite=Lax` rather than `Strict`: `Lax`
 * sends cookies on exactly this kind of navigation.
 */
export const signInUrl = (): string => `${PORTAL_API}/auth/login`;

/**
 * Where the "Get my API key" control points (task 0189).
 *
 * The same plain-link reasoning as {@link signInUrl}, because issuing a key IS
 * an OAuth round-trip now: eligibility (Stellar Discord membership + minimum
 * account age) is proved per action by re-authentication, never carried in the
 * session — a fresh Discord token is what the backend checks membership with,
 * and only a top-level navigation can fetch one. Discord does not re-prompt
 * for consent on repeat authorisation of the same scopes, so for a signed-in
 * visitor this is a redirect, not a login. The callback lands back on the
 * portal with `?issue=<outcome>`, which `app.tsx` renders.
 */
export const issueUrl = (): string => `${PORTAL_API}/auth/login?action=issue`;

/**
 * `POST /api-tokens/api/auth/logout` — clear the session.
 *
 * `POST`, because the backend only accepts that: a `GET` sign-out is triggerable
 * by any third-party page that can make the browser issue a request, which
 * `SameSite=Lax` permits for top-level navigations.
 *
 * Answers `204` with no body, so this does not go through `getJson`.
 */
export async function signOut(): Promise<void> {
  const url = `${PORTAL_API}/auth/logout`;
  let response: Response;
  try {
    response = await fetch(url, {
      method: 'POST',
      signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    });
  } catch {
    throw new PortalApiError(`${url} could not be reached`);
  }
  if (!response.ok) {
    throw new PortalApiError(
      `${url} answered ${response.status}`,
      response.status,
    );
  }
}

/**
 * What `GET` (and `POST`) `/api-tokens/api/key` answer (task 0187, read-only
 * since task 0189 — the route reveals and never creates).
 *
 * Mirrors `KeyResponse` in `packages/prices-api/src/portal/keys/mod.rs`, and
 * hand-written for the same reason the two types above are: the portal's routes
 * are deliberately absent from the published OpenAPI document.
 */
export interface PortalKey {
  /** The API Gateway key id. Not a secret. */
  key_id: string;
  /** `discord-<userId>-key`. */
  name: string;
  /** The credential itself — what goes in `X-API-Key`. */
  value: string;
}

/**
 * What `GET /api-tokens/api/usage` answers (task 0188).
 *
 * Mirrors `UsageResponse` in `packages/prices-api/src/portal/usage/mod.rs`, and
 * hand-written for the same reason every type above is: the portal's routes are
 * deliberately absent from the published OpenAPI document.
 *
 * The three counters are `null` **together** when AWS has recorded nothing for
 * the key yet — the ordinary state minutes after issuance, because `GetUsage`
 * lags. The page renders that as "nothing recorded yet" rather than inventing
 * zeros; the period and `as_of` are always present.
 */
export interface PortalUsage {
  /** Requests counted against the quota this period, per AWS. */
  used: number | null;
  /** Requests left, as of the latest day AWS has data for. */
  remaining: number | null;
  /** The monthly quota, reconstructed as `used + remaining`. */
  limit: number | null;
  /** First day of the current period, `YYYY-MM-DD` (our rule: calendar month, UTC). */
  period_start: string;
  /** Last day of the current period, inclusive. */
  period_end: string;
  /** When the quota resets under our stated rule, RFC 3339. */
  resets_at: string;
  /** When the `GetUsage` behind this answer was made, RFC 3339. */
  as_of: string;
}

/**
 * `GET /api-tokens/api/usage` — the signed-in caller's usage against quota.
 *
 * Read-only by construction on the backend (it can never create, attach or
 * delete a key), which is why — unlike `issueKey` below — the page may call it
 * on load: opening the dashboard cannot mint anything.
 *
 * Resolves to `null` when the caller has no key yet (the backend's
 * `404 no_key`), because for this page that is a renderable state, not a
 * failure.
 */
export async function fetchUsage(): Promise<PortalUsage | null> {
  const url = `${PORTAL_API}/usage`;
  let response: Response;
  try {
    response = await fetch(url, {
      headers: { accept: 'application/json' },
      signal: AbortSignal.timeout(USAGE_TIMEOUT_MS),
    });
  } catch (error) {
    if (isTimeout(error)) {
      throw new PortalApiError(
        `${url} did not answer within ${USAGE_TIMEOUT_MS / 1000}s`,
      );
    }
    throw new PortalApiError(`${url} could not be reached`);
  }
  if (response.status === 404) {
    // Only the backend's own `no_key` envelope means "no key". The other 404
    // on this path is task 0183's gate — an EMPTY body, deliberately
    // byte-identical to an unrouted route — which the backend goes out of its
    // way to keep distinguishable, and which can appear here if the portal is
    // closed while a signed-in tab presses Refresh. Reading that as "you have
    // no API key" would be a false statement on the slice whose theme is
    // rendering honestly.
    try {
      const body = (await response.json()) as { code?: string };
      if (body.code === 'no_key') {
        return null;
      }
    } catch {
      // Not JSON — the gate's empty 404, or something else entirely.
    }
    throw new PortalApiError(`${url} answered 404`, 404);
  }
  if (!response.ok) {
    throw new PortalApiError(
      `${url} answered ${response.status}`,
      response.status,
    );
  }
  try {
    return (await response.json()) as PortalUsage;
  } catch {
    throw new PortalApiError(
      `${url} answered ${response.status}, not JSON`,
      response.status,
    );
  }
}

/**
 * Reveal the key this account already has, or learn there is none.
 *
 * `GET`, and — since task 0189 — safe to fire on load: the backend's key route
 * is **read-only by construction** (it can never create, attach or delete),
 * tested as such, so opening the dashboard cannot mint anything. That reverses
 * 0187's fetch-nothing-on-mount rule by re-deriving it rather than ignoring
 * it: the rule existed because the route could create, and it no longer can.
 * Creating is `issueUrl()`'s round-trip, behind an explicit press.
 *
 * Resolves to `null` when the caller has no key (the backend's `404 no_key`
 * envelope) — a renderable state, not a failure — with the same gate-404
 * caution `fetchUsage` takes: an EMPTY 404 is task 0183's closed portal, and
 * reading it as "you have no key" would be a false statement.
 */
export async function fetchKey(): Promise<PortalKey | null> {
  const url = `${PORTAL_API}/key`;
  let response: Response;
  try {
    response = await fetch(url, {
      headers: { accept: 'application/json' },
      signal: AbortSignal.timeout(KEY_TIMEOUT_MS),
    });
  } catch (error) {
    if (isTimeout(error)) {
      throw new PortalApiError(
        `${url} did not answer within ${KEY_TIMEOUT_MS / 1000}s`,
      );
    }
    throw new PortalApiError(`${url} could not be reached`);
  }
  if (response.status === 404) {
    try {
      const body = (await response.json()) as { code?: string };
      if (body.code === 'no_key') {
        return null;
      }
    } catch {
      // Not JSON — the gate's empty 404, or something else entirely.
    }
    throw new PortalApiError(`${url} answered 404`, 404);
  }
  if (!response.ok) {
    // `401` is the one status this page can act on: it means the session
    // expired while the tab was open, and the answer is to sign in again rather
    // than to retry. Carried through as the status so the caller can say so.
    throw new PortalApiError(
      `${url} answered ${response.status}`,
      response.status,
    );
  }
  try {
    return (await response.json()) as PortalKey;
  } catch {
    throw new PortalApiError(
      `${url} answered ${response.status}, not JSON`,
      response.status,
    );
  }
}
