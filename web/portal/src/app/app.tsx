import { useCallback, useEffect, useState } from 'react';
import { Route, Routes, useSearchParams } from 'react-router-dom';

import {
  fetchPortalConfig,
  fetchSession,
  signInUrl,
  signOut,
  type PortalConfig,
  type PortalSession,
} from '../api/portal';

/**
 * The portal, slices 2 and 3 (tasks 0185 and 0186).
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
  const cancelled = searchParams.get('signin') === 'cancelled';

  const load = useCallback(() => {
    let live = true;
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
    return () => {
      live = false;
    };
  }, []);

  // Same StrictMode guard as the config probe below: React mounts twice in
  // development, and without it the second mount's response can land after the
  // first and overwrite it.
  useEffect(load, [load]);

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
      <p>
        Could not check your sign-in status: <code>{session.reason}</code>
      </p>
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
      </>
    );
  }

  return (
    <>
      {cancelled && <p>Sign-in cancelled.</p>}
      <p>You are not signed in.</p>
      {/* A link, not a button with an onClick. The OAuth flow is a top-level
          navigation to discord.com and back; `fetch` cannot perform one, and
          the session cookie is `SameSite=Lax` precisely so that this navigation
          carries it. `href` is relative, so it stays same-origin. */}
      <a href={signInUrl()}>Sign in with Discord</a>
    </>
  );
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
