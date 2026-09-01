/**
 * The Discord sign-in popup, and the bridge the popup runs on its way back.
 *
 * **The round-trip still happens entirely in the browser's top-level
 * navigation — just in a second window.** That matters more than it looks:
 * every cookie the flow depends on is `SameSite=Lax` and `HttpOnly`, and the
 * reason it survives a popup is that cookies are scoped to an ORIGIN, not to a
 * window. The sequence is
 *
 *   1. the popup navigates to `/api/auth/login` — same-origin, so
 *      the backend's short-lived `pending` cookie (the PKCE verifier, see
 *      `portal/auth/state_token.rs`) is set normally;
 *   2. Discord sends the popup back to `/api/auth/callback` — a
 *      cross-site TOP-LEVEL GET, which is precisely the case `Lax` permits, so
 *      the pending cookie is sent and the exchange completes;
 *   3. the callback sets the session cookie and 303s the popup to
 *      `/api/`, where the bridge below posts the outcome to the opener
 *      and closes the window.
 *
 * Step 2 is why this cannot be a `fetch` and why the control stays an `<a>`
 * with a real `href`: a background request to `/auth/login` would follow the
 * 303 to discord.com as a cross-origin fetch, with no consent screen anyone
 * can see and no way back. The popup is an enhancement layered ON that link —
 * if `window.open` is blocked or throws, the anchor navigates the current tab
 * and the original flow happens unchanged.
 */

/**
 * The popup's `window.name`, and the marker the bridge recognises itself by.
 *
 * `window.name` survives navigation within the same window, so it is still set
 * after the trip through discord.com and back — which is exactly the property
 * needed, and the reason this is not a query parameter the backend would have
 * to carry.
 */
const POPUP_NAME = 'stellar-portal-oauth';

/** Marks a message as ours. `postMessage` is a public channel. */
const MESSAGE_SOURCE = 'stellar-portal-oauth';

/** What the popup sends home: the query the callback landed it on. */
export type OAuthPopupMessage = {
  // The literal, spelled out rather than `typeof MESSAGE_SOURCE`. Referencing
  // the const from an EXPORTED type forces it into the emitted `.d.ts`, where
  // it appears as a declaration nothing uses at runtime — which is what the
  // build output's lint pass reports. The two must stay in step; the assignment
  // in `bridgeOAuthPopup` is what would fail to typecheck if they drifted.
  source: 'stellar-portal-oauth';
  /** `location.search` at the landing — `''`, `?signin=cancelled`, … */
  search: string;
};

/**
 * Open the sign-in round-trip in a second window.
 *
 * Returns the window, or `null` when the browser refused — a popup blocker, a
 * context where `window.open` is not implemented (jsdom), or an environment
 * that throws. **Every one of those cases must fall back to navigating the
 * current tab**, which is what the caller does by not calling
 * `preventDefault()`.
 *
 * Sized and centred rather than left to the browser: an OAuth consent screen
 * opened as a 100 × 100 window in the corner reads as something that went
 * wrong.
 */
export function openOAuthPopup(url: string): Window | null {
  const width = 520;
  const height = 760;
  // `screenX`/`screenY` rather than `screenLeft`: on a multi-monitor setup the
  // popup belongs on the same screen as the window that opened it.
  const left = Math.max(0, window.screenX + (window.outerWidth - width) / 2);
  const top = Math.max(0, window.screenY + (window.outerHeight - height) / 3);

  try {
    const popup = window.open(
      url,
      POPUP_NAME,
      `popup=yes,width=${width},height=${height},left=${Math.round(
        left,
      )},top=${Math.round(top)}`,
    );
    // A blocked popup is `null`; some blockers return a window that is already
    // closed, which is the same thing for our purposes.
    if (!popup || popup.closed) return null;
    popup.focus?.();
    return popup;
  } catch {
    return null;
  }
}

/**
 * Run inside the popup, before anything else mounts.
 *
 * Returns `true` when this document IS the OAuth popup and has handled itself —
 * the caller must then render nothing. Without that, the popup would boot the
 * whole app, follow the `/` route's redirect to `/dashboard`, and sit there as
 * a second copy of the portal that the visitor has to close by hand.
 *
 * `targetOrigin` is this origin, never `'*'`: the message says nothing secret
 * (the session is an `HttpOnly` cookie the opener will read for itself), but a
 * wildcard would broadcast the outcome to whatever page happens to be the
 * opener, and there is no reason to.
 */
export function bridgeOAuthPopup(): boolean {
  const opener = window.opener as Window | null;
  if (!opener || opener === window || window.name !== POPUP_NAME) {
    return false;
  }

  const message: OAuthPopupMessage = {
    source: MESSAGE_SOURCE,
    search: window.location.search,
  };

  try {
    opener.postMessage(message, window.location.origin);
  } catch {
    // The opener is gone (the visitor closed the tab mid-flow). Nothing to
    // tell, and closing is still the right thing to do.
  }
  window.close();
  return true;
}

/**
 * Listen for the popup's message on the opener side.
 *
 * Returns an unsubscribe function. The origin check is not optional: `message`
 * events arrive from any frame or window that cares to send one, and without it
 * a third-party page could end the wait early and make the portal claim an
 * outcome that never happened.
 */
export function onOAuthPopupMessage(
  handler: (message: OAuthPopupMessage) => void,
): () => void {
  const listener = (event: MessageEvent) => {
    if (event.origin !== window.location.origin) return;
    const data = event.data as Partial<OAuthPopupMessage> | null;
    if (!data || data.source !== MESSAGE_SOURCE) return;
    handler({ source: MESSAGE_SOURCE, search: data.search ?? '' });
  };
  window.addEventListener('message', listener);
  return () => window.removeEventListener('message', listener);
}

/**
 * The `?signin=…` outcome the callback appended, out of a popup message.
 *
 * The literals are the backend's (`portal/auth/mod.rs`) and anything else —
 * including the empty string a successful sign-in lands on — is `null`,
 * meaning "no refusal to report; ask the server whether there is a session
 * now".
 *
 * `cancelled` and `failed` are task 0186's. `not_member` and `unknown` arrived
 * with the sign-in membership gate (Adam, 2026-08-26) and are the SAME two
 * verdicts the issue round-trip has always had — kept as separate literals
 * here for the reason they are separate there: refusing someone is not the
 * same as failing to check.
 *
 * Allow-listed rather than passed through: the value reaches a render branch,
 * and the query it comes from is attacker-supplied on a top-level navigation.
 */
export type SigninOutcome =
  | 'cancelled'
  | 'failed'
  | 'not_member'
  | 'unknown'
  | 'not_open';

/** Every literal the backend lands, and the only values this returns. */
const SIGNIN_OUTCOMES: readonly SigninOutcome[] = [
  'cancelled',
  'failed',
  'not_member',
  'unknown',
  // A deployment with no eligibility parameters wired — 0183's closed portal,
  // reached through the callback rather than through `/config`.
  'not_open',
] as const;

export function readSigninOutcome(search: string): SigninOutcome | null {
  const value = new URLSearchParams(search).get('signin');
  // A union rather than a widened `string`: the caller derives one boolean per
  // literal, and a value outside this list used to fall through every branch
  // to the plain sign-in card. Typing the return makes a new literal here a
  // compile error at the render site until it is given a screen.
  return SIGNIN_OUTCOMES.find((outcome) => outcome === value) ?? null;
}
