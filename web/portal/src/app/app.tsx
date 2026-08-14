import { useEffect, useState } from 'react';
import { Route, Routes } from 'react-router-dom';

import { fetchPortalConfig, type PortalConfig } from '../api/portal';

/**
 * The portal, slice 2 (task 0185).
 *
 * One route, plain HTML elements, and no styling worth the name. **Ugly is a
 * requirement here, not a concession**: anything presentable in this slice has
 * been taken from task 0193 and has delayed task 0186. MUI arrives with 0193.
 *
 * What this page is actually for is the pipeline behind it — a change to this
 * file reaches `/api-tokens/` — and one property that is expensive to prove any
 * later: that a relative `fetch` from this bundle reaches the API **same-origin**
 * through task 0184's distribution.
 *
 * Do not add a second route before task 0195 lands the per-prefix SPA fallback.
 * A hard refresh on a sub-path resolves against S3, which grants `s3:GetObject`
 * and not `s3:ListBucket`, so a missing key reads as `403 AccessDenied` rather
 * than a 404. With one route there is nothing to break yet.
 */

type Probe =
  | { state: 'loading' }
  | { state: 'ok'; config: PortalConfig }
  | { state: 'failed'; reason: string };

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
          click. No sign-in button: there is nothing behind it, and task 0186 is
          what puts one here. */}
      {probe.state === 'ok' && !probe.config.enabled && (
        <p>
          This is where you will sign in and issue an API key. It is not yet
          available.
        </p>
      )}

      {/* Reachable only once PORTAL_ENABLED is true. Deliberately empty of
          function: sign-in is task 0186's, key issue is task 0187's. It exists
          so that flipping the flag is visibly not a no-op. */}
      {probe.state === 'ok' && probe.config.enabled && (
        <p>The portal is open. Sign-in arrives with the next slice.</p>
      )}

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
          not exist. The portal backend maps exactly one path, `/config`
          (`portal/mod.rs`), and task 0183's gate answers an empty 404 on every
          other path under the prefix — so a `/health` probe would render a
          failure whether or not anyone implemented it. `/config` answers 200 in
          BOTH flag states, which is what makes it the honest probe. */}
      <p>
        Reached <code>/api-tokens/api/config</code>{' '}
        {probe.state === 'ok' ? 'successfully' : 'unsuccessfully'} —
        same-origin, no API key, no CORS.
      </p>
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
