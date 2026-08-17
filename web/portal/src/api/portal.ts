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
