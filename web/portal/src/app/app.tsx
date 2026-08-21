import { useCallback, useEffect, useRef, useState } from 'react';
import { Route, Routes, useSearchParams } from 'react-router-dom';

import {
  fetchKey,
  fetchPortalConfig,
  fetchSession,
  fetchUsage,
  issueUrl,
  signInUrl,
  signOut,
  PortalApiError,
  type PortalConfig,
  type PortalKey,
  type PortalSession,
  type PortalUsage,
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

/** The landing params sign-in's callback appends. */
const SIGNIN_PARAMS = ['signin'] as const;
/** The landing params the issue callback appends (task 0189). */
const ISSUE_PARAMS = ['issue', 'wait_secs'] as const;

/**
 * Read landing-state query params **once**, then strip them from the URL.
 *
 * The values survive in state for this mount; the URL does not — so a reload,
 * a bookmark, or a sign-out-and-back-in shows no stale banner. This closes
 * 0186's open item O10 (`?signin=cancelled` never left the URL, so a sign-out
 * after a cancelled attempt could still say "Sign-in cancelled"), and the
 * issue outcomes (task 0189) get the same lifecycle from day one: an
 * eligibility refusal is about the round-trip that just ended, not about
 * every future visit to the same URL.
 *
 * `replace`, not `push`: the stripped URL takes the history entry's place, so
 * Back does not resurrect the banner either.
 */
function useOneShotParams(
  names: readonly string[],
): Record<string, string | null> {
  const [searchParams, setSearchParams] = useSearchParams();
  // Captured exactly once per mount. (React StrictMode's simulated
  // remount preserves state, so the capture survives it; a REAL remount —
  // sign-out and back in — reads the already-stripped URL, which is the
  // one-shot behaviour wanted.)
  const [taken] = useState<Record<string, string | null>>(() => {
    const out: Record<string, string | null> = {};
    for (const name of names) out[name] = searchParams.get(name);
    return out;
  });

  // Strip at most once, and only when there is something to strip — a
  // navigation for a URL that would not change is a render loop waiting for a
  // `setSearchParams` whose identity is not stable.
  const stripped = useRef(false);
  const anythingToStrip = Object.values(taken).some((value) => value !== null);
  useEffect(() => {
    if (!anythingToStrip || stripped.current) return;
    stripped.current = true;
    setSearchParams(
      (previous) => {
        const next = new URLSearchParams(previous);
        for (const name of names) next.delete(name);
        return next;
      },
      { replace: true },
    );
  }, [anythingToStrip, names, setSearchParams]);

  return taken;
}

/**
 * The two prerequisites, stated **before** the visitor authenticates
 * (task 0189): learning about the membership requirement after the consent
 * screen means they authorised an app for nothing. The wording is this task's;
 * 0193 restyles it without re-deciding it.
 *
 * `discord.gg/stellardev` is the registered vanity invite — the other invites
 * SDF publishes are personal and at least one is already dead (task 0179).
 * The account-age line deliberately names no number: the threshold is operator
 * configuration the backend reports when it matters, and a hard-coded "5
 * minutes" here would drift the moment the SSM parameter changes.
 */
function Prerequisites() {
  return (
    <p>
      Getting an API key needs two things, both checked via Discord when you ask
      for one: membership of the{' '}
      <a href="https://discord.gg/stellardev">Stellar Developers Discord</a>,
      and a Discord account that is not brand new.
    </p>
  );
}

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
 *
 * `rateLimit` is nothing to do with sign-in and is not read here: it comes off
 * `/config`, which only this component's parent has, and is wanted three levels
 * down by the usage panel (task 0188). Passed through rather than re-fetched or
 * put in a context — one prop across two hops is less machinery than either,
 * and it keeps the value's single source visible in the call chain.
 */
function SignIn({ rateLimit }: { rateLimit?: number }) {
  const [session, setSession] = useState<SessionState>({ state: 'loading' });
  // Two landing states, from two literals the backend appends. `cancelled` is
  // the visitor's own choice at Discord's consent screen; `failed` is any other
  // OAuth error — a drifted scope registration, a Discord outage — which the
  // backend also logs. Telling them apart on the page is the visible half of
  // that split: calling a misconfiguration "cancelled" is what made it look
  // like every visitor was changing their mind. One-shot (task 0189, closing
  // 0186's O10): shown for this landing, stripped from the URL.
  const { signin } = useOneShotParams(SIGNIN_PARAMS);
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
      <Dashboard
        onSignOut={onSignOut}
        session={session.session}
        rateLimit={rateLimit}
      />
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
      {/* Both prerequisites, BEFORE the control that starts an OAuth flow —
          the acceptance criterion. Signing in itself needs neither, but the
          visitor deciding whether to authorise an app deserves to know what
          the key they came for will require. */}
      <Prerequisites />
      {/* A link, not a button with an onClick. The OAuth flow is a top-level
          navigation to discord.com and back; `fetch` cannot perform one, and
          the session cookie is `SameSite=Lax` precisely so that this navigation
          carries it. `href` is relative, so it stays same-origin. */}
      <a href={signInUrl()}>Sign in with Discord</a>
    </>
  );
}

/**
 * The signed-in half of the page: identity, the key (task 0187), and usage
 * against quota (task 0188). Still one flat, unstyled column — task 0193 is
 * what turns this into something that looks like a dashboard.
 *
 * The one piece of state lifted here is whether THIS page load issued or
 * revealed a key. `Usage` needs it for honesty's sake: the backend caches its
 * answers briefly and `GetUsage` itself lags, so straight after a press the
 * usage endpoint can still say "no key" or "nothing recorded" about a key the
 * page is literally displaying — and "you have no key" would be false. Knowing
 * a key exists turns both of those answers into "your key is new; figures
 * appear with a delay", which is the same lag message the whole slice is built
 * around.
 */
function Dashboard({
  onSignOut,
  session,
  rateLimit,
}: {
  onSignOut: () => void;
  session: PortalSession;
  /** The free plan's per-second rate limit, straight from `/config`. */
  rateLimit?: number;
}) {
  const [keyOnScreen, setKeyOnScreen] = useState(false);

  return (
    <>
      {/* The acceptance criterion, rendered: username and ID. The ID is the
          account key (ADR 0010) and the username is display only — it comes
          from the signed session cookie and is refreshed at each sign-in. */}
      <p>
        Signed in as <strong>{session.username}</strong> (ID{' '}
        <code>{session.user_id}</code>)
      </p>
      <button type="button" onClick={onSignOut}>
        Sign out
      </button>
      {/* Tasks 0187 + 0189. Inside the authenticated branch, so signing out
          removes it along with the key it was showing — the component unmounts
          and its state goes with it, rather than leaving a stale credential on
          screen for the next person at the keyboard. */}
      <ApiKey onKey={() => setKeyOnScreen(true)} />
      {/* Task 0188. Keyed refetch: a key appearing on screen (revealed on
          mount, or fresh off 0189's issue round-trip) re-asks for usage, so
          the section leaves "no key yet" without a manual refresh. */}
      <Usage keyOnScreen={keyOnScreen} rateLimit={rateLimit} />
    </>
  );
}

/**
 * The largest wait this page will name, in seconds — a hundred years.
 *
 * Not a rejection threshold: a value above it is CLAMPED to it, not thrown
 * away. It exists only to keep the arithmetic inside `Number`'s exact-integer
 * range, so that a nonsense `wait_secs` renders as an obviously absurd number
 * of days rather than as `Infinity` or a rounded float.
 */
const MAX_WAIT_SECS = 100 * 365 * 24 * 60 * 60;

/**
 * Render the too-young wait from the backend's `wait_secs`.
 *
 * The number comes from a URL parameter, so it is sanitised: digits only, and
 * clamped rather than rejected. **Never a calendar date** — that pattern is
 * right for 0191's weeks-long rework cap and absurd for minutes — and never a
 * hard-coded "5 minutes": the copy follows what the backend computed from the
 * operator's threshold.
 *
 * **Clamping, not rejecting, is the whole point of the bucket ladder.** A
 * length-bounded digit pattern (`\d{1,7}`) rejected anything past ~16 weeks
 * and fell through to "a few minutes" — so a `min-account-age-minutes` set
 * high by an operator's typo (it is a `put-parameter`, applied without a
 * redeploy and validated by nothing at deploy time) rendered a months-long
 * refusal as a coffee break. That is the most misleading direction available:
 * the visitor retries, is refused again, and has no way to tell the wait is
 * not what the page said. Overstating is recoverable; understating is not.
 */
function describeWait(waitSecs: string | null): string {
  if (!waitSecs || !/^\d+$/.test(waitSecs)) {
    return 'a few minutes';
  }
  // `Number` of a very long digit string is `Infinity`, which `Math.min`
  // resolves to the ceiling — so no length bound is needed to stay finite.
  const parsed = Math.min(Number(waitSecs), MAX_WAIT_SECS);
  if (!(parsed > 0)) {
    return 'a few minutes';
  }
  const plural = (n: number, unit: string) =>
    `about ${n} ${unit}${n === 1 ? '' : 's'}`;
  if (parsed < 60) return plural(parsed, 'second');
  if (parsed < 3600) return plural(Math.ceil(parsed / 60), 'minute');
  if (parsed < 86_400) return plural(Math.ceil(parsed / 3600), 'hour');
  return plural(Math.ceil(parsed / 86_400), 'day');
}

/**
 * The API key, masked (task 0187; issuance re-shaped by task 0189).
 *
 * Rendered only for a signed-in visitor, because that is the only state in which
 * the backend will answer — and because a control that cannot work is worse than
 * no control at all.
 *
 * **Fetches on mount — the reversal of 0187's fetch-nothing rule, re-derived
 * rather than ignored.** That rule existed because `GET /key` could create;
 * since 0189 the route is read-only by construction and by test, so showing
 * the visitor the key they already have costs nothing and mints nothing.
 * Creating one is now `issueUrl()`'s OAuth round-trip: a top-level navigation
 * (the eligibility proof needs a fresh Discord token, which only a navigation
 * can fetch), which lands back here with `?issue=<outcome>` — rendered below,
 * once, in the wording this task decides and 0193 restyles.
 *
 * **Two UI niceties, and only two.** Styling is task 0193's and this page is
 * deliberately unstyled, so masking and copying had to earn their place ahead of
 * it. Masking did because this renders during screen-shares and pairing
 * sessions, and an API key on screen is a credential in somebody's recording.
 * Copying did because a 40-character opaque string is not something anyone
 * retypes — without a button, the first thing every visitor does is select it by
 * hand, which defeats the masking they just toggled.
 */
function ApiKey({ onKey }: { onKey?: () => void }) {
  type KeyView =
    | { state: 'loading' }
    | { state: 'ok'; key: PortalKey }
    | { state: 'none' }
    | { state: 'failed'; reason: string };

  const [view, setView] = useState<KeyView>({ state: 'loading' });
  const [error, setError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);

  // The issue round-trip's landing state, read once and stripped from the URL
  // — see `useOneShotParams`. `wait_secs` only means anything alongside
  // `issue=too_young`.
  const { issue, wait_secs: waitSecs } = useOneShotParams(ISSUE_PARAMS);

  // The same guard `SignIn` and `Usage` keep, and for both of their reasons:
  // signing out unmounts this component, and a response landing afterwards
  // would be a credential written into the state of a component the visitor
  // has just left — and "Check again" can be pressed while a load is still in
  // flight, where the older answer must not overwrite the newer one.
  const cancelInFlight = useRef<(() => void) | null>(null);

  // `onKey` is held in a ref rather than closed over, so `load` has EMPTY
  // dependencies and the mount effect below runs exactly once.
  //
  // It read `[onKey]` before, and the caller passes an inline arrow — a fresh
  // identity on every render. Calling `onKey()` set state on the dashboard,
  // which re-rendered, which made a new `onKey`, which made a new `load`,
  // which re-fired the effect: **two `GET /key` calls per page load**, three
  // when landing on `?issue=ok`. Each is a paginated `GetApiKeys` plus a
  // `GetApiKey` against the account-wide control-plane budget, so the
  // per-load cost 0187's decision 12 hands to 0194 was quietly doubled.
  //
  // The ref, not a `useCallback` at the call site: this component should not
  // depend on every caller remembering to memoise a prop.
  const onKeyRef = useRef(onKey);
  useEffect(() => {
    onKeyRef.current = onKey;
  });

  const load = useCallback(() => {
    cancelInFlight.current?.();
    let live = true;
    cancelInFlight.current = () => {
      live = false;
    };
    fetchKey()
      .then((key) => {
        if (!live) return;
        if (key) {
          setView({ state: 'ok', key });
          // Masked on arrival, always: what nobody asked for is the
          // credential appearing on screen while they were looking elsewhere.
          setRevealed(false);
          // Tell the dashboard a key exists (task 0188's usage section reads
          // it) — the FACT only, never the value or the id: nothing outside
          // this component needs the credential, so nothing outside it gets
          // to hold one.
          onKeyRef.current?.();
        } else {
          setView({ state: 'none' });
        }
      })
      .catch((cause: unknown) => {
        if (!live) return;
        setView({ state: 'failed', reason: describeFailure(cause) });
      });
  }, []);

  /**
   * Re-ask, from a control the visitor pressed.
   *
   * The loading state is the point: pressing "Check again" while the key is
   * still settling produced *no visible change at all* — same words, same
   * button — and a control that does nothing observable reads as broken. The
   * sibling `Usage` section's Refresh has always done this; this is the same
   * feedback, on the retry that needs it most (it is pressed precisely when
   * the previous answer was "not yet").
   */
  const reload = useCallback(() => {
    setView({ state: 'loading' });
    load();
  }, [load]);

  useEffect(() => {
    // `?issue=ok` is itself proof a key exists — the backend created one
    // before it redirected — even though `GetApiKeys` may not list it for a
    // moment yet. Reporting the fact NOW keeps the usage section beside this
    // one from telling a visitor who has just been given their first key that
    // they have none and should issue one.
    if (issue === 'ok') onKeyRef.current?.();
    load();
    return () => cancelInFlight.current?.();
  }, [load, issue]);

  const onCopy = () => {
    if (view.state !== 'ok') return;
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
      .writeText(view.key.value)
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

      {/* The issue round-trip's outcome, in the wording this task decides.
          The two refusals a visitor can fix render differently from the one
          they cannot ("could not verify"), and "could not verify" is
          explicitly NOT an accusation — a Discord outage says nothing about
          anyone's membership. Retry is the same round-trip link in every
          case, because eligibility is proved per attempt, never remembered. */}
      {/* Only where the key is on screen, or about to be. `GetApiKeys` is
          eventually consistent (see `keys/mod.rs` — "the listing can come
          back NON-empty and still not contain the key just created"), and the
          reveal is a single read with no retry, so on a first issuance the
          redirect can beat the listing: "Your key is ready." above "you have
          no API key yet" is the page contradicting itself about the one fact
          the visitor came for, and the settling branch below says what is
          actually true. The `failed` state is the same contradiction with a
          different second half — "Your key is ready." directly above "Could
          not get your API key" — so it is excluded by naming the two states
          this line belongs to rather than by excluding `none` alone. */}
      {issue === 'ok' && (view.state === 'ok' || view.state === 'loading') && (
        <p data-testid="issue-ok">Your key is ready.</p>
      )}
      {issue === 'not_member' && (
        <p data-testid="issue-not-member">
          You need to be a member of the{' '}
          <a href="https://discord.gg/stellardev">Stellar Developers Discord</a>{' '}
          to get an API key — joining is an open invite, and new members may
          need to complete the server&apos;s screening first. Once you are in,{' '}
          <a href={issueUrl()}>try again</a>.
        </p>
      )}
      {issue === 'too_young' && (
        <p data-testid="issue-too-young">
          Your Discord account is too new to get an API key — this is a short
          wait, not a rejection. Try again in {describeWait(waitSecs)}:{' '}
          <a href={issueUrl()}>get my API key</a>.
        </p>
      )}
      {issue === 'unknown' && (
        <p data-testid="issue-unknown">
          We could not verify your Discord membership just now — that is a
          problem talking to Discord, not a statement about your membership.
          Please <a href={issueUrl()}>try again</a> shortly.
        </p>
      )}
      {/* Worded so it is true in both of its causes: the control plane
          refusing after a passed check, and an issuance the deployment cannot
          perform at all (`/auth/login?action=issue` on an unwired build lands
          here too, rather than on a JSON error page). What it must keep
          saying either way is decision #7's split — this is our key service,
          never a doubt about the visitor. */}
      {issue === 'failed' && (
        <p data-testid="issue-failed">
          We could not create your key. This is our key service, not your
          Discord membership. Please <a href={issueUrl()}>try again</a>, and
          tell us if it keeps happening.
        </p>
      )}
      {/* The round-trip ended at Discord, before any check ran. Two states,
          not one, for the same reason `failed` and `unknown` are two: the
          visitor's own choice and our broken registration are different
          events belonging to different people. Neither is a verdict — these
          are the issue flow's half of the pair sign-in has carried since
          0186, and without them a cancelled press landed on a `?signin=…`
          banner that only renders while signed out, which an issue round-trip
          has by definition left. */}
      {issue === 'cancelled' && (
        <p data-testid="issue-cancelled">
          You stopped before Discord could check your membership — nothing was
          created, and nothing is wrong. Pick it up again whenever you like:{' '}
          <a href={issueUrl()}>get my API key</a>.
        </p>
      )}
      {issue === 'denied' && (
        <p data-testid="issue-denied">
          Discord would not complete the check, and this is not something you
          did. Please <a href={issueUrl()}>try again</a>, and tell us if it
          keeps happening.
        </p>
      )}

      {view.state === 'loading' && <p>Checking for your API key…</p>}

      {/* Issued a moment ago, not listed yet. A *wait*, like `too_young` —
          so it renders as one, and specifically NOT as the "you have no key"
          branch below, which would offer to issue a second key for a visitor
          who has just been given their first. */}
      {view.state === 'none' && issue === 'ok' && (
        <p data-testid="issue-ok-settling">
          Your key was created, and is taking a moment to appear.{' '}
          <button type="button" onClick={reload}>
            Check again
          </button>
        </p>
      )}

      {view.state === 'none' && issue !== 'ok' && (
        <>
          <Prerequisites />
          <p>
            One key, on the free plan. Asking again later shows the same key
            rather than issuing another.
          </p>
          {/* A link, not a button: issuing is an OAuth round-trip (see
              `issueUrl`), and only a top-level navigation can carry it. */}
          <a href={issueUrl()}>Get my API key</a>
        </>
      )}

      {view.state === 'ok' && (
        <>
          <p>
            {/* Masked by default. The mask is a fixed run of dots, not a
                prefix-and-suffix of the real value: showing the first and last
                few characters of a credential is a habit borrowed from card
                numbers, where the rest is high-entropy. Here it would leak part
                of the secret for no benefit anyone asked for. */}
            <code data-testid="api-key">
              {revealed ? view.key.value : '••••••••••••••••••••••••••••••••'}
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
            requests. Key <code>{view.key.name}</code>.
          </p>
        </>
      )}

      {view.state === 'failed' && (
        <p>
          Could not get your API key: <code>{view.reason}</code>
        </p>
      )}

      {error && (
        <p>
          Could not copy your API key: <code>{error}</code>
        </p>
      )}
    </section>
  );
}

/**
 * Usage against quota, rendered honestly (task 0188).
 *
 * Three numbers and a date, unstyled — task 0193 makes it a dashboard. What is
 * decided HERE, once, and what 0193 restyles without re-deciding:
 *
 * - **The lag wording.** `GetUsage` is not a read-after-write surface
 *   (measured, task 0180), so the figures can trail reality by minutes. Every
 *   render of the numbers carries the "last updated" line below, verbatim; a
 *   dashboard that admits a lag beats one that looks broken.
 * - **The reset rule.** "The 1st of each month, 00:00 UTC" is OUR stated rule,
 *   not an AWS guarantee — AWS documents neither the instant nor the timezone
 *   (ADR 0010, correction #2) — and it is worded as ours.
 * - **The limits as numbers**: requests per second (task 0157) and
 *   used-of-quota, not prose. The rate figure comes from `/config`, not from a
 *   literal here — it is the per-env value the gateway enforces
 *   (`pricingApiFreePlanRateLimit`), and it was the one number on this panel
 *   that could drift from what is actually in force.
 *
 * Fetches on mount, unlike `ApiKey` beside it, and the difference is the whole
 * point of the backend's design: `GET /usage` is read-only by construction —
 * it can never create, attach or delete a key — so opening the page costs
 * nothing and mints nothing. The refresh button re-asks; the backend's
 * in-process cache is what keeps that from turning into control-plane traffic.
 */
function Usage({
  keyOnScreen,
  rateLimit,
}: {
  keyOnScreen: boolean;
  rateLimit?: number;
}) {
  type UsageView =
    | { state: 'loading' }
    | { state: 'ok'; usage: PortalUsage }
    | { state: 'no-key' }
    | { state: 'failed'; reason: string };

  const [view, setView] = useState<UsageView>({ state: 'loading' });

  // The same in-flight guard `SignIn` keeps, for the same two reasons: the
  // refresh button can supersede a pending load, and signing out unmounts this
  // component mid-flight.
  const cancelInFlight = useRef<(() => void) | null>(null);
  const load = useCallback(() => {
    cancelInFlight.current?.();
    let live = true;
    cancelInFlight.current = () => {
      live = false;
    };
    setView({ state: 'loading' });
    fetchUsage()
      .then((usage) => {
        if (!live) return;
        setView(usage ? { state: 'ok', usage } : { state: 'no-key' });
      })
      .catch((error: unknown) => {
        // Through `describeFailure`, like the key section beside this one: an
        // expired session must read "sign out and sign in again" in BOTH
        // places, not as that sentence in one and a raw "answered 401" in the
        // other — two wordings for one cause on one screen reads as two bugs.
        if (live) setView({ state: 'failed', reason: describeFailure(error) });
      });
  }, []);

  // On mount only.
  useEffect(() => {
    load();
    return () => cancelInFlight.current?.();
  }, [load]);

  // Fires the keyed refetch AT MOST ONCE per mount. This latch is what lets
  // the effect below watch `view.state` — the obvious dependency — without
  // becoming a fetch loop: the backend can legitimately keep answering
  // `no_key` while its own cache catches up, so "no-key → load → no-key"
  // would otherwise re-trigger itself forever.
  //
  // Watching the state (rather than the `keyOnScreen` transition alone, which
  // is what a ref-read did) is the point, and it survives task 0189 changing
  // where the key comes from. The two arrivals race the mount-time usage
  // fetch: the reveal resolving with a key, and a landing on `?issue=ok`.
  // Either can happen while the view is still `'loading'`, so a
  // transition-only effect saw nothing to do — and then never ran again,
  // because its dependencies had already settled. The in-flight request,
  // issued before the key existed, resolves `no_key`, and the section sat on
  // "your key is new" until the visitor found the Refresh button. Nothing the
  // visitor could press helped either: `setKeyOnScreen(true)` on an
  // already-`true` state changes no dependency.
  const refetchedForKey = useRef(false);

  // When a key is on screen and the usage section says "no key", refetch —
  // that is the one answer the issue just falsified, whenever it arrives. A
  // section already showing numbers is showing an answer a reveal does not
  // change, and blanking it into a loading flicker for an identical body would
  // make the press look like it broke something, so `'ok'` is left alone.
  useEffect(() => {
    if (!keyOnScreen || view.state !== 'no-key' || refetchedForKey.current) {
      return;
    }
    refetchedForKey.current = true;
    load();
  }, [keyOnScreen, view.state, load]);

  /**
   * THE lag line — the wording this task decides once. Rendered under every
   * state that shows (or withholds) a figure, so the page never presents an
   * AWS-lagged number as live.
   */
  const lastUpdated = (asOf: string) => (
    <p>
      {/* `toUTCString` spells the zone "GMT"; the decided wording says UTC,
          and since 0193 must not re-decide this line, the suffix is corrected
          here rather than frozen. Same instant either way. */}
      Last updated {new Date(asOf).toUTCString().replace(/GMT$/, 'UTC')} — AWS
      reports usage with a delay, so requests made in the last few minutes may
      not be counted yet.
    </p>
  );

  /** The reset rule (ours) and the rate limit (0157), as numbers. */
  const limits = (resetsAt?: string) => (
    <>
      {/* Omitted, not defaulted, when the deployment did not say what the
          limit is: a fallback figure here would be exactly the silent
          staleness reading it from `/config` removes. Every deployed
          environment sets it — `compute-stack.ts` passes
          `pricingApiFreePlanRateLimit` unconditionally — so the absent case is
          a local run, where saying nothing is the honest answer. */}
      {rateLimit !== undefined && (
        <p>
          Rate limit: <strong data-testid="rate-limit">{rateLimit}</strong>{' '}
          request
          {rateLimit === 1 ? '' : 's'} per second.
        </p>
      )}
      <p>
        Quota resets on the 1st of each month, 00:00 UTC
        {resetsAt ? (
          <>
            {' '}
            — next reset <strong>{resetsAt.slice(0, 10)}</strong>
          </>
        ) : null}
        .
      </p>
    </>
  );

  return (
    <section>
      <h2>Usage this period</h2>

      {view.state === 'loading' && <p>Loading your usage…</p>}

      {view.state === 'no-key' && (
        <>
          {keyOnScreen ? (
            // The endpoint said "no key" while a key is on this very screen:
            // the backend's short cache has not caught up with the issue yet.
            // "You have no key" would be false, so say what is actually
            // happening.
            <p>
              Your key is new — usage figures appear here with a delay after
              your first requests.
            </p>
          ) : (
            <p>You have no API key yet — issue one above to see your usage.</p>
          )}
          {/* The limits still render: they are properties of the plan every
              key joins, not of a particular key, and the visitor deciding
              whether to issue one is exactly who they inform. No next-reset
              date — that comes with a usage answer. */}
          {limits()}
        </>
      )}

      {view.state === 'ok' &&
        (view.usage.used === null ? (
          <>
            {/* AWS has no rows for the key yet. Not zeros: inventing
                `remaining` and `limit` would be guessing, and the honest state
                is "nothing recorded", which for a fresh key is expected. */}
            <p>
              AWS has not recorded any usage for your key this period yet —
              figures appear with a delay after your first requests.
            </p>
            {limits(view.usage.resets_at)}
            {lastUpdated(view.usage.as_of)}
          </>
        ) : (
          <>
            <p>
              Used: <strong data-testid="usage-used">{view.usage.used}</strong>
            </p>
            <p>
              Remaining:{' '}
              <strong data-testid="usage-remaining">
                {view.usage.remaining}
              </strong>
            </p>
            <p>
              Monthly limit:{' '}
              <strong data-testid="usage-limit">{view.usage.limit}</strong>
            </p>
            {limits(view.usage.resets_at)}
            {lastUpdated(view.usage.as_of)}
          </>
        ))}

      {view.state === 'failed' && (
        <p>
          Could not load your usage: <code>{view.reason}</code>
        </p>
      )}

      {view.state !== 'loading' && (
        <button type="button" onClick={load}>
          Refresh
        </button>
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
      {probe.state === 'ok' && probe.config.enabled && (
        <SignIn rateLimit={probe.config.rate_limit_per_second} />
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
