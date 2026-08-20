import { useCallback, useEffect, useRef, useState } from 'react';
import { Route, Routes, useSearchParams } from 'react-router-dom';

import {
  fetchPortalConfig,
  fetchSession,
  issueKey,
  signInUrl,
  signOut,
  PortalApiError,
  type PortalConfig,
  type PortalKey,
  type PortalSession,
} from '../api/portal';

/**
 * The portal, slices 2 to 4 (tasks 0185, 0186 and 0187).
 *
 * One route, plain HTML elements, and no styling worth the name. **Ugly is a
 * requirement here, not a concession**: anything presentable in this slice has
 * been taken from task 0193, where MUI arrives.
 *
 * What this page is for is the pipeline behind it — a change to this file
 * reaches `/api-tokens/` — plus two properties that are expensive to prove any
 * later: that a relative `fetch` from this bundle reaches the API
 * **same-origin** through task 0184's distribution, and now that the session
 * cookie survives that round-trip in a real browser.
 *
 * Do not add a second route before task 0195 lands the per-prefix SPA fallback.
 * A hard refresh on a sub-path resolves against S3, which grants `s3:GetObject`
 * and not `s3:ListBucket`, so a missing key reads as `403 AccessDenied` rather
 * than a 404. Sign-in does not need one: every redirect in the flow lands back
 * on `/api-tokens/`, which is a real object.
 */

type Probe =
  | { state: 'loading' }
  | { state: 'ok'; config: PortalConfig }
  | { state: 'failed'; reason: string };

type SessionState =
  | { state: 'loading' }
  | { state: 'ok'; session: PortalSession }
  | { state: 'failed'; reason: string };

/**
 * Sign-in, as plain text and one control (task 0186).
 *
 * Rendered only when the portal is open — while the flag is off there is nothing
 * behind the button, and the closed page must still offer nothing to click.
 *
 * Both the signed-out and cancelled states are **plain text, not screens**, per
 * the task. "Cancelled" is not an error: the visitor pressed Cancel at Discord's
 * consent screen, the callback redirected here with `?signin=cancelled`, and the
 * only reasonable response is to say so and leave the button where it was.
 */
function SignIn() {
  const [session, setSession] = useState<SessionState>({ state: 'loading' });
  const [searchParams] = useSearchParams();
  // Two landing states, from two literals the backend appends. `cancelled` is
  // the visitor's own choice at Discord's consent screen; `failed` is any other
  // OAuth error — a drifted scope registration, a Discord outage — which the
  // backend also logs. Telling them apart on the page is the visible half of
  // that split: calling a misconfiguration "cancelled" is what made it look
  // like every visitor was changing their mind.
  const signin = searchParams.get('signin');
  const cancelled = signin === 'cancelled';
  const failed = signin === 'failed';

  // Cancels whichever `/auth/me` is currently in flight, whoever started it.
  //
  // `load` is called from two places — the mount effect and the sign-out
  // handler — and the second used to drop the canceller it was handed, so that
  // request had a `live` flag nothing could ever flip. Keeping the latest one
  // here means both the supersede case (a second call while the first is
  // pending) and the unmount case are covered by one mechanism, rather than by
  // whichever caller happened to remember.
  const cancelInFlight = useRef<(() => void) | null>(null);

  const load = useCallback(() => {
    cancelInFlight.current?.();
    let live = true;
    const cancel = () => {
      live = false;
    };
    cancelInFlight.current = cancel;

    fetchSession()
      .then((result) => {
        if (live) setSession({ state: 'ok', session: result });
      })
      .catch((error: unknown) => {
        if (live)
          setSession({
            state: 'failed',
            reason: error instanceof Error ? error.message : String(error),
          });
      });
    return cancel;
  }, []);

  // Same StrictMode guard as the config probe below: React mounts twice in
  // development, and without it the second mount's response can land after the
  // first and overwrite it. The cleanup reads the ref rather than closing over
  // this call's canceller, so it also cancels a request the sign-out handler
  // started.
  useEffect(() => {
    load();
    return () => cancelInFlight.current?.();
  }, [load]);

  const onSignOut = () => {
    setSession({ state: 'loading' });
    signOut()
      // Re-ask rather than assuming. The server is what decides whether the
      // cookie is gone, and asking it costs one request on an action nobody
      // performs in a loop.
      .then(() => load())
      .catch((error: unknown) =>
        setSession({
          state: 'failed',
          reason: error instanceof Error ? error.message : String(error),
        }),
      );
  };

  if (session.state === 'loading') {
    return <p>Checking whether you are signed in…</p>;
  }

  if (session.state === 'failed') {
    return (
      <>
        <p>
          Could not check your sign-in status: <code>{session.reason}</code>
        </p>
        {/* The control stays. A failed `/auth/me` usually means the backend is
            unreachable, in which case signing in will fail too — but it can
            also be one bad response, and a page that reports an error while
            removing the only thing the visitor could do about it is a dead end
            they can leave only by guessing at a reload. Signing in is a fresh
            top-level navigation, so it does not depend on the request that just
            failed. */}
        <a href={signInUrl()}>Sign in with Discord</a>
      </>
    );
  }

  if (session.session.authenticated) {
    return (
      <>
        {/* The acceptance criterion, rendered: username and ID. The ID is the
            account key (ADR 0010) and the username is display only — it comes
            from the signed session cookie and is refreshed at each sign-in. */}
        <p>
          Signed in as <strong>{session.session.username}</strong> (ID{' '}
          <code>{session.session.user_id}</code>)
        </p>
        <button type="button" onClick={onSignOut}>
          Sign out
        </button>
        {/* Task 0187. Inside the authenticated branch, so signing out removes
            it along with the key it was showing — the component unmounts and
            its state goes with it, rather than leaving a stale credential on
            screen for the next person at the keyboard. */}
        <ApiKey />
      </>
    );
  }

  return (
    <>
      {cancelled && <p>Sign-in cancelled.</p>}
      {failed && (
        <p>
          Sign-in could not be completed. This is not something you did — try
          again, and tell us if it keeps happening.
        </p>
      )}
      <p>You are not signed in.</p>
      {/* A link, not a button with an onClick. The OAuth flow is a top-level
          navigation to discord.com and back; `fetch` cannot perform one, and
          the session cookie is `SameSite=Lax` precisely so that this navigation
          carries it. `href` is relative, so it stays same-origin. */}
      <a href={signInUrl()}>Sign in with Discord</a>
    </>
  );
}

/**
 * The API key, masked (task 0187).
 *
 * Rendered only for a signed-in visitor, because that is the only state in which
 * the backend will answer — and because a control that cannot work is worse than
 * no control at all.
 *
 * **Two UI niceties, and only two.** Styling is task 0193's and this page is
 * deliberately unstyled, so masking and copying had to earn their place ahead of
 * it. Masking did because this renders during screen-shares and pairing
 * sessions, and an API key on screen is a credential in somebody's recording.
 * Copying did because a 40-character opaque string is not something anyone
 * retypes — without a button, the first thing every visitor does is select it by
 * hand, which defeats the masking they just toggled.
 */
function ApiKey() {
  const [key, setKey] = useState<PortalKey | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);

  // The same guard `SignIn` above keeps, for a narrower reason. There is no
  // supersede case here — the only caller is a button that disables itself
  // while busy — but signing out unmounts this component, and a response that
  // lands afterwards would be a credential written into the state of a
  // component the visitor has just left. Cheap, and consistent with the
  // component next to it, which is what stops the next reader wondering which
  // of the two is wrong.
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  // Nothing runs on mount. The backend treats `GET` and `POST` on `/key`
  // identically — without a registry it cannot tell "deleted by hand" from
  // "never issued", so a reveal has to be able to create — which means a page
  // that fetched on load would issue a production API key to anyone who merely
  // opened it. Keeping the only call behind a press is what makes the visitor's
  // intent explicit rather than implied by having loaded a URL.
  const onIssue = () => {
    setBusy(true);
    setError(null);
    setCopied(false);
    issueKey()
      .then((issued) => {
        if (!live.current) return;
        setKey(issued);
        // Masked on arrival, including the very first time. The visitor pressed
        // the button and can press reveal; what they did not ask for is the
        // credential appearing on screen while they were looking at the button.
        setRevealed(false);
      })
      .catch((cause: unknown) => {
        if (!live.current) return;
        setError(describeFailure(cause));
      })
      .finally(() => {
        if (live.current) setBusy(false);
      });
  };

  const onCopy = () => {
    if (!key) return;
    // `navigator.clipboard` is absent on an insecure origin and in jsdom, and a
    // missing API must not throw past this handler and blank the page. The
    // fallback is honest text rather than a silent no-op: the visitor can still
    // reveal the key and select it.
    const clipboard = navigator.clipboard;
    if (!clipboard) {
      setError(
        'Copying is not available here — reveal the key and copy it by hand.',
      );
      return;
    }
    clipboard
      .writeText(key.value)
      .then(() => {
        setCopied(true);
        setError(null);
      })
      .catch(() =>
        setError('Could not copy — reveal the key and copy it by hand.'),
      );
  };

  return (
    <section>
      <h2>Your API key</h2>

      {!key && (
        <>
          <p>
            One key, on the free plan. Pressing this again later shows the same
            key rather than issuing another.
          </p>
          <button type="button" onClick={onIssue} disabled={busy}>
            {busy ? 'Working…' : 'Get my API key'}
          </button>
        </>
      )}

      {key && (
        <>
          <p>
            {/* Masked by default. The mask is a fixed run of dots, not a
                prefix-and-suffix of the real value: showing the first and last
                few characters of a credential is a habit borrowed from card
                numbers, where the rest is high-entropy. Here it would leak part
                of the secret for no benefit anyone asked for. */}
            <code data-testid="api-key">
              {revealed ? key.value : '••••••••••••••••••••••••••••••••'}
            </code>
          </p>
          <button type="button" onClick={() => setRevealed((was) => !was)}>
            {revealed ? 'Hide' : 'Reveal'}
          </button>{' '}
          <button type="button" onClick={onCopy}>
            Copy
          </button>
          {copied && <p>Copied.</p>}
          <p>
            Send it as the <code>X-API-Key</code> header on <code>/v1/</code>{' '}
            requests. Key <code>{key.name}</code>.
          </p>
        </>
      )}

      {error && (
        <p>
          Could not get your API key: <code>{error}</code>
        </p>
      )}
    </section>
  );
}

/**
 * Turn a failed issue into something the visitor can act on.
 *
 * `401` is the one status that has an answer other than "try again": the
 * session expired while the tab sat open, and no amount of retrying the button
 * will help. `api/portal.ts` carries the status through for exactly this, and
 * without this branch that would be a promise the page did not keep.
 */
function describeFailure(cause: unknown): string {
  if (cause instanceof PortalApiError && cause.status === 401) {
    return 'your session has expired — sign out and sign in again';
  }
  return cause instanceof Error ? cause.message : String(cause);
}

function PortalHome() {
  const [probe, setProbe] = useState<Probe>({ state: 'loading' });

  useEffect(() => {
    let live = true;
    fetchPortalConfig()
      .then((config) => {
        if (live) setProbe({ state: 'ok', config });
      })
      .catch((error: unknown) => {
        if (live)
          setProbe({
            state: 'failed',
            reason: error instanceof Error ? error.message : String(error),
          });
      });
    // React 18+ StrictMode mounts twice in development; without this the second
    // mount's response can land after the first and overwrite it.
    return () => {
      live = false;
    };
  }, []);

  return (
    <main>
      <h1>Stellar Prices API — API keys</h1>

      {probe.state === 'loading' && <p>Checking whether the portal is open…</p>}

      {/* The closed state is the one that ships. Task 0194 flips the flag,
          after task 0189's eligibility gate passes — so for now this is what
          every visitor sees, and it must say so plainly and offer nothing to
          click. No sign-in control: the backend answers those routes with an
          empty 404 while the flag is off, so a button here would be a button
          that cannot work. */}
      {probe.state === 'ok' && !probe.config.enabled && (
        <p>
          This is where you will sign in and issue an API key. It is not yet
          available.
        </p>
      )}

      {/* Reachable only once PORTAL_ENABLED is true. Task 0186 put sign-in
          here; issuing a key is task 0187's. */}
      {probe.state === 'ok' && probe.config.enabled && <SignIn />}

      {/* A failure here is not cosmetic: it means the bundle could not reach
          its own backend, which is either the behaviour ordering in task 0184's
          routing table or the gate in task 0183. Show the reason rather than a
          spinner that never resolves. */}
      {probe.state === 'failed' && (
        <p>
          Could not reach the portal backend: <code>{probe.reason}</code>
        </p>
      )}

      <hr />

      {/* The same-origin proof, and the reason it is this route rather than the
          `/api-tokens/api/health` named in task 0185's criteria: that route does
          not exist. The portal backend maps `/config` and, from task 0186,
          `/auth/*`; task 0183's gate answers an empty 404 on every other path
          under the prefix — so a `/health` probe would render a failure whether
          or not anyone implemented it. `/config` answers 200 in BOTH flag
          states, which is what makes it the honest probe. */}
      {/* Three branches, not a two-way ternary on `=== 'ok'`. While the probe
          is in flight the answer is not yet known, and a ternary claimed
          "unsuccessfully" on first paint — so the page said it had failed to
          reach the backend at the same time as saying it was still asking. This
          paragraph is the acceptance criterion's evidence of a live call; it has
          to be silent about the outcome until there is one. */}
      {probe.state === 'loading' ? (
        <p>
          Calling <code>/api-tokens/api/config</code> — same-origin, no API key,
          no CORS.
        </p>
      ) : (
        <p>
          Reached <code>/api-tokens/api/config</code>{' '}
          {probe.state === 'ok' ? 'successfully' : 'unsuccessfully'} —
          same-origin, no API key, no CORS.
        </p>
      )}
    </main>
  );
}

export function App() {
  return (
    <Routes>
      <Route path="/" element={<PortalHome />} />
    </Routes>
  );
}

export default App;
