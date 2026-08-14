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
 * therefore nothing to generate this from. See `src/api/generated.ts` for the
 * types that ARE generated.
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

async function getJson<T>(url: string): Promise<T> {
  let response: Response;
  try {
    response = await fetch(url, { headers: { accept: 'application/json' } });
  } catch {
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
  return (await response.json()) as T;
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
