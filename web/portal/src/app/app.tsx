import Box from '@mui/material/Box';
import Button from '@mui/material/Button';
import Container from '@mui/material/Container';
import Divider from '@mui/material/Divider';
import Dialog from '@mui/material/Dialog';
import Stack from '@mui/material/Stack';
import CircularProgress from '@mui/material/CircularProgress';
import Typography from '@mui/material/Typography';
import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type MouseEvent,
  type ReactNode,
} from 'react';
import {
  Link as RouterLink,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
  useSearchParams,
} from 'react-router-dom';

import { Navbar, Footer } from '../landing/Chrome';
import { DiscordIcon } from '../landing/DiscordIcon';
import {
  Benefits,
  Callout,
  DISCORD,
  LOGIN_CARD_MAX_WIDTH,
  LabelledRule,
  LoginCard,
} from '../landing/LoginCard';
import {
  DashboardCard,
  KeyField,
  MetaField,
  MetaRow,
  NoticeStrip,
  UsageMeter,
} from '../landing/DashboardPanel';
import { DashboardNavbar } from '../landing/DashboardChrome';
import {
  LoginSection,
  useOwnBackLink,
  visuallyHidden,
} from '../landing/LoginSection';
import {
  onOAuthPopupMessage,
  openOAuthPopup,
  readSigninOutcome,
} from '../landing/oauthPopup';
import { cardBorder } from '../landing/primitives';
import { Documentation } from '../landing/Documentation';
import { DeveloperDashboard } from '../landing/DeveloperDashboard';
import { Endpoints } from '../landing/Endpoints';
import { FairAccess } from '../landing/FairAccess';
import { Faq } from '../landing/Faq';
import { Features } from '../landing/Features';
import { FinalCta } from '../landing/FinalCta';
import { HeroSection } from '../landing/Hero';
import { QuickStart } from '../quickstart/QuickStart';
import { SelfService } from '../landing/SelfService';
import { UseCases } from '../landing/UseCases';
import { ArrowBadge } from '../landing/primitives';
import AddRoundedIcon from '@mui/icons-material/AddRounded';
import AutorenewRoundedIcon from '@mui/icons-material/AutorenewRounded';
import ChevronLeftRoundedIcon from '@mui/icons-material/ChevronLeftRounded';
import ContentCopyRoundedIcon from '@mui/icons-material/ContentCopyRounded';
import ErrorOutlineRoundedIcon from '@mui/icons-material/ErrorOutlineRounded';
import SwitchAccountRoundedIcon from '@mui/icons-material/SwitchAccountRounded';
import SyncProblemRoundedIcon from '@mui/icons-material/SyncProblemRounded';

import {
  LOGIN_ANCHOR,
  QUICKSTART,
  STELLAR_DISCORD_INVITE,
} from '../landing/links';
import { alpha } from '@mui/material/styles';

import { theme } from '../theme/theme';
import { color, font, radius } from '../theme/tokens';

import {
  fetchKey,
  fetchPortalConfig,
  fetchSession,
  fetchUsage,
  issueUrl,
  revokeKey,
  signInUrl,
  signOut,
  PortalApiError,
  type PortalConfig,
  type PortalKey,
  type PortalKeyRevoked,
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
 * reaches `/api/` — plus two properties that are expensive to prove any
 * later: that a `fetch` from this bundle reaches the API and that the session
 * cookie survives that round-trip in a real browser.
 *
 * Sub-routes were once forbidden here — a hard refresh resolved against S3 and
 * came back `403 AccessDenied`. That constraint is gone (task 0194): the host
 * rewrites every extensionless path under `/api/` to this bundle, so a route
 * costs nothing on the hosting side. Sign-in still needs none: every redirect
 * in the flow lands back on `/api/`.
 */

type Probe =
  | { state: 'loading' }
  | { state: 'ok'; config: PortalConfig }
  | { state: 'failed'; reason: string };

type SessionState =
  | { state: 'loading' }
  | { state: 'ok'; session: PortalSession }
  /**
   * `while` says which request failed, because the two need different
   * sentences and different remedies: a failed CHECK leaves the visitor's
   * standing unknown and the fix is to ask again (or to sign in afresh); a
   * failed SIGN-OUT leaves them signed in on a device they meant to leave,
   * and the fix is to try again or close the browser. Rendering the second
   * as the first — or, as this used to, as a successful sign-out — is what
   * put the key back on screen after a reload on a shared machine.
   */
  | { state: 'failed'; reason: string; while: 'checking' | 'signing-out' };

/**
 * The login card's headline and standfirst — **new copy, owned by task 0193**,
 * from the Figma login frame (`778:2499`).
 *
 * Task 0193's rule is that it re-decides no copy another slice owns, and this
 * is the other side of that rule: the card's own chrome is not owned by anyone
 * else, so the design's wording is taken as written. Everything INSIDE the card
 * body — the prerequisites, the refusals, the cancelled and failed banners —
 * still belongs to tasks 0186 and 0189 and is rendered verbatim.
 *
 * Shared across all four probe states so the card does not appear to change
 * identity while it is deciding what to say.
 */
const LOGIN_TITLE = 'Get your API key';
const LOGIN_SUBTITLE =
  'Sign in with Discord to receive your key instantly. No forms, no waiting, no manual approval.';

/**
 * The "What you get" list from the design.
 *
 * Also 0193's copy — and deliberately NOT a restatement of the eligibility
 * prerequisites, which are task 0189's and appear above the button where the
 * acceptance criterion puts them. These four say what the key is; those two say
 * who may have one. Collapsing them into one list is how the requirement stops
 * being stated before the visitor authorises.
 */
const BENEFITS = [
  { text: 'Instant API key — no waiting', kind: 'check' },
  { text: '100,000 requests/month — free', kind: 'check' },
  { text: 'Usage dashboard and key management', kind: 'check' },
  { text: 'Discord account is your identity', kind: 'discord' },
] as const;

/** The landing params sign-in's callback appends. */
const SIGNIN_PARAMS = ['signin'] as const;
/**
 * The landing params the issue callback appends (task 0189; `next_eligible_at`
 * since task 0191). `wait_secs` only means anything beside `issue=too_young`,
 * `next_eligible_at` only beside `issue=capped`.
 */
const ISSUE_PARAMS = ['issue', 'wait_secs', 'next_eligible_at'] as const;

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
function useSession(enabled: boolean): {
  session: SessionState;
  onSignOut: () => void;
  reload: () => void;
} {
  const [session, setSession] = useState<SessionState>({ state: 'loading' });

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
            while: 'checking',
          });
      });
    return cancel;
  }, []);

  // Same StrictMode guard as the config probe below: React mounts twice in
  // development, and without it the second mount's response can land after the
  // first and overwrite it. The cleanup reads the ref rather than closing over
  // this call's canceller, so it also cancels a request the sign-out handler
  // started.
  //
  // `enabled` gates the fetch, and that gate is not an optimisation: while
  // `PORTAL_ENABLED` is off, task 0183's route gate answers `/auth/me` with an
  // empty 404, so asking would put a guaranteed failure in the console of
  // every visitor to a closed portal and leave this hook reporting `failed`
  // for a portal that is merely shut. Not asking leaves it `loading`, and the
  // routes that care never consult it while the portal is closed.
  useEffect(() => {
    if (!enabled) return;
    load();
    return () => cancelInFlight.current?.();
  }, [enabled, load]);

  const onSignOut = () => {
    setSession({ state: 'loading' });
    signOut()
      // Re-ask rather than assuming. The server is what decides whether the
      // cookie is gone, and asking it costs one request on an action nobody
      // performs in a loop.
      .then(() => load())
      // ⚠️ A failed sign-out is NOT a sign-out. The `HttpOnly` cookie was
      // never cleared, so the session is exactly as live as before the
      // press; this state exists so the page says that rather than landing
      // the visitor on the marketing page as if it had worked.
      .catch((error: unknown) =>
        setSession({
          state: 'failed',
          reason: error instanceof Error ? error.message : String(error),
          while: 'signing-out',
        }),
      );
  };

  return { session, onSignOut, reload: load };
}

/**
 * The `/login` view — the Figma login frame (`778:2499`) and nothing else.
 *
 * A route of its own since the portal grew a second page. The OAuth callback
 * still lands on `/api/`, exactly as `portal/auth/mod.rs` says it will
 * ("when the portal grows a second page, the page it lands on decides where to
 * go next; this handler still will not") — so `RootRoute` is what forwards a
 * `?signin=…` landing here, carrying the query with it. That is why the
 * banners below still read their params from the URL: they arrive on this
 * route, just not from Discord directly.
 */
function LoginView({
  session,
  onSignedIn,
}: {
  session: SessionState;
  onSignedIn: () => void;
}) {
  // Two landing states, from two literals the backend appends. `cancelled` is
  // the visitor's own choice at Discord's consent screen; `failed` is any other
  // OAuth error — a drifted scope registration, a Discord outage — which the
  // backend also logs. Telling them apart on the page is the visible half of
  // that split: calling a misconfiguration "cancelled" is what made it look
  // like every visitor was changing their mind. One-shot (task 0189, closing
  // 0186's O10): shown for this landing, stripped from the URL.
  const { signin } = useOneShotParams(SIGNIN_PARAMS);

  /**
   * The refusal on screen, from EITHER source.
   *
   * The wording below is task 0186's and there is one copy of it, but there are
   * now two ways to arrive at it: the full-page round-trip lands on
   * `?signin=…`, and the popup hands the same literal back through
   * `postMessage`. Feeding both into one piece of state is what keeps that a
   * single rendering path — two branches saying the same sentence is how the
   * two wordings drift apart.
   */
  const [outcome, setOutcome] = useState<string | null>(signin);
  const cancelled = outcome === 'cancelled';
  const failed = outcome === 'failed';
  // ⚠️ Two states this card did not have before 2026-08-26 (Adam): sign-in
  // now proves Discord membership, so the two verdicts that used to appear
  // only after pressing "Get my API key" can also end the sign-in itself.
  const notMember = outcome === 'not_member';
  const unverified = outcome === 'unknown';
  // A deployment that cannot ask the eligibility question at all. Its own
  // literal since 2026-08-27 — it used to arrive as `unknown` and borrow the
  // "problem talking to Discord" wording for a state where nothing was asked
  // and retrying cannot help. See `NOT_OPEN_QUERY` in `portal/auth/mod.rs`.
  const notOpen = outcome === 'not_open';

  // The OAuth error card draws its own back link, under the retry button, so
  // the section above must not draw one too. Called unconditionally with a
  // boolean — it is a hook. See `useOwnBackLink`.
  useOwnBackLink(cancelled || failed);

  /** Whether a sign-in window is open and being waited on. */
  const [waiting, setWaiting] = useState(false);
  /**
   * The session answer that was on screen when a SUCCESSFUL round-trip ended.
   *
   * `null` whenever nothing is being confirmed. Set by `finish`, cleared by
   * the effect that watches for a newer answer — see it for why this is a
   * reference rather than a flag.
   */
  const confirmingFrom = useRef<SessionState | null>(null);
  /**
   * The current session answer, readable from inside the wait effect without
   * being one of its dependencies.
   *
   * Listing `session` there would tear down and rebuild the poll and the two
   * watchers every time the session object changed — including the change the
   * round-trip itself causes. A ref is also the more correct read: what
   * `finish` needs is the answer on screen WHEN IT RUNS, not the one that was
   * there when the effect was created.
   */
  const sessionNow = useRef(session);
  sessionNow.current = session;
  /** Whether the "use a different account" dialog is open. */
  const [switching, setSwitching] = useState(false);

  const popup = useRef<Window | null>(null);
  const navigate = useNavigate();

  /**
   * Watch the sign-in window: its message, its closing, and the session it is
   * trying to create.
   *
   * Three signals rather than one, because each covers a case the others
   * cannot. The message is the fast, precise path — it carries the refusal
   * literal. Polling `/auth/me` covers a popup whose `postMessage` never
   * arrives (an extension, a `noopener` policy, a browser that reuses a tab).
   * Watching `closed` covers the visitor who simply shuts the window: nothing
   * happened, so the card goes back to offering the button rather than sitting
   * on a spinner for ever.
   */
  useEffect(() => {
    if (!waiting) return;

    let live = true;
    let grace: number | undefined;
    const finish = (refusal: string | null) => {
      if (!live) return;
      live = false;
      window.clearTimeout(grace);
      window.clearInterval(poll);
      window.clearInterval(watchClosed);
      // ⚠️ A SUCCESS keeps waiting. `onSignedIn` only STARTS the `/auth/me`
      // read that flips this app to signed-in and navigates to the dashboard;
      // dropping `waiting` here rendered the "Sign in with Discord" card again
      // for the length of that request, immediately after a sign-in that
      // worked — the visitor watches the button they just pressed come back.
      // The spinner stays until the session settles (the effect below), which
      // is also the honest description of what is happening.
      if (refusal !== null) {
        setWaiting(false);
      } else {
        // Remember WHICH session answer was on screen when the round-trip
        // ended, so the effect below can tell the reload's answer from it.
        confirmingFrom.current = sessionNow.current;
      }
      setOutcome(refusal);
      // Ask the server either way. A refusal means no session, and saying so
      // costs one request; a success means the cookie is already set and this
      // is the only thing that will notice.
      onSignedIn();
    };

    /**
     * Hold a slower signal's verdict open for `POPUP_MESSAGE_GRACE_MS` so the
     * message can still overtake it. ⚠️ The poll and the close both used to
     * end the wait on the spot, and both can observe their fact BEFORE the
     * popup's `postMessage` arrives: the callback's 303 sets the cookie and
     * lands the popup on its query, but the popup still has to download the
     * bundle before `bridgeOAuthPopup` posts — and once it does post it closes
     * in the same breath, so `closed` can be true while the message is still
     * queued. Ending early tore the listener down and the query it carried
     * (`?issue=too_young`, `?signin=not_member`) was lost: a refused visitor
     * saw a plain dashboard or the generic failure card. Only the first
     * caller starts the timer; a message during the grace calls `finish`
     * directly and cancels it.
     */
    const afterGrace = (verdict: () => void) => {
      if (!live || grace !== undefined) return;
      grace = window.setTimeout(() => {
        grace = undefined;
        if (live) verdict();
      }, POPUP_MESSAGE_GRACE_MS);
    };

    const stopListening = onOAuthPopupMessage(({ search }) => {
      // ⚠️ The popup's landing query is FORWARDED, not only read (Adam,
      // 2026-08-26). Sign-in now issues the first key, and the callback lands
      // the popup on `?issue=ok` (or `too_young`, `failed`) — a query the
      // OPENER never sees, because the popup is the window that navigated.
      // Putting it on this tab's URL before the session is re-read lets
      // `LoginRoute`'s redirect carry it to `/dashboard`, where the first-login
      // card reads it exactly as it reads the full-page flow's. `signin=…`
      // outcomes are not forwarded: they belong to this card, and
      // `readSigninOutcome` is what reads them.
      if (!live) return;
      if (new URLSearchParams(search).has('issue')) {
        navigate({ search }, { replace: true });
      }
      finish(readSigninOutcome(search));
    });

    // 1.5s: fast enough that a visitor who finishes at Discord does not sit
    // looking at a spinner, slow enough that a two-minute consent screen costs
    // eighty requests to a route the backend answers from its own session
    // cookie.
    const poll = window.setInterval(() => {
      if (!live || grace !== undefined) return;
      fetchSession()
        .then((result) => {
          if (result.authenticated) afterGrace(() => finish(null));
        })
        .catch(() => {
          // A failed poll says nothing about the round-trip in the other
          // window. Keep waiting; the message or the close will end this.
        });
    }, 1500);

    const watchClosed = window.setInterval(() => {
      if (!live || !popup.current?.closed) return;
      // ⚠️ **A shut window is now a reported failure (Adam, 2026-08-26).** It
      // used to be `finish(null)`: the card went quietly back to offering the
      // button, which is indistinguishable from the button not having worked.
      //
      // But it is decided AGAINST THE SERVER, not from the close alone. The
      // popup posts its message and closes in the same breath, and the poll
      // may not have run yet, so "the window is gone" is not evidence that
      // nothing happened — a visitor who completed the round-trip and then
      // closed the window must land on the dashboard, never on an error about
      // a sign-in that worked. And it is decided only AFTER the grace, so a
      // message the close raced past still lands its own, precise refusal.
      afterGrace(() => {
        fetchSession()
          .then((result) => finish(result.authenticated ? null : 'failed'))
          // No answer is not proof of a session, and the visitor is looking
          // at a window they just closed. Say the sign-in did not complete;
          // the card it lands on offers the retry.
          .catch(() => finish('failed'));
      });
    }, 500);

    return () => {
      live = false;
      window.clearTimeout(grace);
      stopListening();
      window.clearInterval(poll);
      window.clearInterval(watchClosed);
    };
  }, [waiting, onSignedIn, navigate]);

  /**
   * Stop waiting once the session read `finish` started has ANSWERED.
   *
   * Identity, not `state`: `reload` leaves the previous answer on screen while
   * its request is in flight (it never flips to `loading`), so "the state is
   * not loading" is true the whole time and would end the wait instantly —
   * which is the flash this exists to prevent. Every answer is a fresh object,
   * so a change of reference is the arrival of a new one.
   *
   * On the happy path the redirect unmounts this component before it runs. It
   * matters on the unhappy one: the popup reported success, the session read
   * came back NOT authenticated (the cookie never arrived, or expired between
   * the two), and without this the card would spin for ever on a round-trip
   * that is over. The button comes back, which is all there is left to offer.
   */
  useEffect(() => {
    if (confirmingFrom.current === null) return;
    if (session === confirmingFrom.current) return;
    confirmingFrom.current = null;
    setWaiting(false);
  }, [session]);

  /**
   * Open the round-trip in a second window — and do nothing at all if the
   * browser will not allow it.
   *
   * `preventDefault` is called ONLY after a window is actually open. When it is
   * blocked, the click falls through to the anchor's `href` and the flow
   * happens in this tab exactly as it did before the popup existed.
   */
  const onSignInClick = (event: MouseEvent<HTMLAnchorElement>) => {
    // Let the browser handle the gestures that mean "somewhere else": a
    // middle-click, ⌘/Ctrl-click or a modified click is a request for a tab,
    // and hijacking it into a popup is the kind of thing that makes people
    // stop trusting a page with their credentials.
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }
    const opened = openOAuthPopup(signInUrl());
    if (!opened) return;
    event.preventDefault();
    popup.current = opened;
    setOutcome(null);
    setWaiting(true);
  };

  if (session.state === 'loading') {
    return (
      <LoginCard
        title={LOGIN_TITLE}
        titleComponent="h1"
        subtitle={LOGIN_SUBTITLE}
      >
        <Callout variant="neutral" icon={<CircularProgress size={18} />}>
          <p>Checking whether you are signed in…</p>
        </Callout>
      </LoginCard>
    );
  }

  if (session.state === 'failed') {
    return (
      <LoginCard
        title={LOGIN_TITLE}
        titleComponent="h1"
        subtitle={LOGIN_SUBTITLE}
      >
        <SessionFailedCallout session={session} />
        {/* The control stays. A failed `/auth/me` usually means the backend is
            unreachable, in which case signing in will fail too — but it can
            also be one bad response, and a page that reports an error while
            removing the only thing the visitor could do about it is a dead end
            they can leave only by guessing at a reload. Signing in is a fresh
            top-level navigation, so it does not depend on the request that just
            failed. */}
        <DiscordButton href={signInUrl()}>Sign in with Discord</DiscordButton>
      </LoginCard>
    );
  }

  // The design's second screen. Reached only when a popup actually opened —
  // when one did not, the click became a full-page navigation and this window
  // is already on its way to Discord.
  if (waiting) {
    return (
      <LoginCard
        title="Redirecting to Discord"
        titleComponent="h1"
        subtitle="A new window will open for you to authorize with Discord."
      >
        <Callout variant="discord" title="Discord authorization window opened.">
          Complete the sign-in there to continue. If nothing opened,{' '}
          {/* The fallback the design's wording promises, wired to the thing it
              promises: a plain top-level navigation in THIS tab, which is the
              flow that works without a popup at all. */}
          <a href={signInUrl()}>click here</a>.
        </Callout>
        <Stack spacing={1.5} alignItems="center" sx={{ py: 2 }}>
          {/* The design's spinner is an arc travelling around a visible track,
              not a bare arc. MUI draws only the arc, so the track is a second
              ring underneath — a `determinate` progress pinned at 100%, which
              inherits the same geometry rather than guessing at a border
              radius that happens to line up. */}
          <Box sx={{ position: 'relative', display: 'inline-flex' }}>
            <CircularProgress
              aria-hidden
              variant="determinate"
              value={100}
              size={44}
              thickness={3}
              sx={{ color: alpha(color.stroke.default, 0.45) }}
            />
            <CircularProgress
              size={44}
              thickness={3}
              sx={{ position: 'absolute', left: 0 }}
            />
          </Box>
          <Typography variant="body1" color="text.primary">
            Waiting for Discord…
          </Typography>
          <Typography variant="body2" sx={{ color: color.text.tertiary }}>
            This page will update automatically.
          </Typography>
        </Stack>
      </LoginCard>
    );
  }

  // ⚠️ **The OAuth failure is a screen too** (Adam, 2026-08-26, Figma
  // `825:1284`), and it covers all THREE ways the round-trip can end without a
  // session: the visitor pressed Cancel at Discord's consent screen, they shut
  // the window, or Discord answered with an error.
  //
  // **This merges a split task 0186 decided, and the merge is Adam's.** That
  // task kept `cancelled` in plain text with no error skin, on the grounds
  // that colouring a deliberate choice red tells the visitor they broke
  // something. The frame puts both under one "OAuth error" callout whose own
  // wording names the cancellation case, so the presentation is now one state.
  //
  // What is NOT merged, and must not be: the backend still lands the two on
  // different literals (`?signin=cancelled` and `?signin=failed`,
  // `portal/auth/mod.rs`) and still logs them apart. 0186's real defect was a
  // MISCONFIGURATION reading as "every visitor is changing their mind", and
  // that direction is still closed — the distinction survives everywhere it
  // was load-bearing, and un-merging the screen is a one-line change here.
  // Before the OAuth-failure screen, because it is not a failure of anything
  // the visitor did: the portal is not open on this deployment, and 0183's
  // card is the one that says so.
  if (notOpen) {
    return <ClosedPortalCard />;
  }

  if (cancelled || failed) {
    return (
      <LoginCard
        // ⚠️ **The frame's header, verbatim, at Adam's instruction
        // (2026-08-26)** — the same two lines the waiting card carries,
        // because this screen is that card's error variant and the designer
        // kept them. It was briefly "Sign-in was not completed"; that is
        // reverted. Read literally the header now describes a redirect that
        // has already happened and will not happen again until the visitor
        // presses the button below, and the callout under it is what carries
        // the state. Recorded rather than argued: it is the designer's call,
        // and one line to change if it reads wrong in front of people.
        title="Redirecting to Discord"
        titleComponent="h1"
        subtitle="A new window will open for you to authorize with Discord."
        footer={<KeepsHappening />}
      >
        {/* ⚠️ The frame's sentence, verbatim (Adam, 2026-08-26). It names two
            of the three ways in — denied access, and a timed-out session —
            and not the third, a window shut before finishing, which lands
            here too. Left as the frame has it. */}
        <Callout variant="error" title="OAuth error">
          <p data-testid="signin-failed">
            Discord returned an error during authorization. This can happen if
            you denied access or if the session timed out.
          </p>
        </Callout>
        <DiscordButton href={signInUrl()} onClick={onSignInClick}>
          Try again with Discord
        </DiscordButton>
        <BackToLanding />
      </LoginCard>
    );
  }

  // ⚠️ **A screen, not a banner** (Adam, 2026-08-26, Figma `825:1485`). A
  // visitor refused for membership cannot use anything else on this card: the
  // sign-in button would hand them straight back to the same refusal, and the
  // prerequisites list would be telling them a rule they have just been
  // refused under. The card is replaced rather than annotated, and the ONE
  // action it offers is the one that changes the answer — joining.
  //
  // Its sibling `unknown` deliberately stays a banner on the ordinary card.
  // See the comment there.
  if (notMember) {
    return (
      <LoginCard
        title="Access not available"
        titleComponent="h1"
        subtitle={
          <>
            Your Discord account does not meet the requirements to receive an
            API key.
          </>
        }
        footer={<KeepsHappening />}
      >
        {/* `neutral`, whose fallback glyph is already the frame's padlock.
            NOT the error skin: nothing failed — we asked, Discord answered,
            and the answer was no. Red would say the portal broke. */}
        <Callout variant="neutral" title="Stellar Discord membership required">
          <p data-testid="signin-not-member">
            API keys are available to members of the Stellar Discord server.
            Join the server and try again.{' '}
            {/* ⚠️ The frame stops at the sentence above; this one is kept from
                task 0189's wording (Adam, 2026-08-26) because it is the only
                thing that explains the case where a visitor HAS joined and is
                still refused. `pending: true` — on the server, still inside
                its screening — is a refusal by `eligibility::membership`, and
                without this line that visitor's only next step is a support
                thread. */}
            New members may need to complete the server&apos;s screening first.
          </p>
        </Callout>
        {/* The invite, in Discord's own blurple — the same control the sign-in
            state uses, because both hand the visitor to Discord and neither
            should look like the rest of the site while doing it. */}
        <DiscordButton href={STELLAR_DISCORD_INVITE}>
          Join Stellar Discord
        </DiscordButton>
        {/* The quiet second action. A REFUSED sign-in leaves no session (see
            `portal/auth/mod.rs`), so there is nothing to sign out of and this
            is simply the round-trip again — where Discord's own account
            switcher is the part that does the work. */}
        <TryDifferentAccount onClick={() => setSwitching(true)} />
        {switching && (
          <SwitchAccountDialog
            onClose={() => setSwitching(false)}
            onSignIn={(event) => {
              // Closed first, so the popup-blocked fallback navigates this tab
              // away from a dialog rather than out from under one.
              setSwitching(false);
              onSignInClick(event);
            }}
          />
        )}
      </LoginCard>
    );
  }

  return (
    <LoginCard
      title={LOGIN_TITLE}
      titleComponent="h1"
      subtitle={LOGIN_SUBTITLE}
    >
      {/* Both banners keep task 0186's wording exactly. "Cancelled" is not an
          error and does not get the error skin — the visitor pressed Cancel at
          Discord's consent screen, and colouring that red would tell them they
          broke something. "Failed" is ours, so it does. */}
      {cancelled && (
        <Callout variant="discord">
          <p>Sign-in cancelled.</p>
        </Callout>
      )}
      {failed && (
        <Callout variant="error">
          <p>
            Sign-in could not be completed. This is not something you did — try
            again, and tell us if it keeps happening.
          </p>
        </Callout>
      )}
      {/* ⚠️ **New copy, and the only copy this slice writes rather than
          restyles** (Adam, 2026-08-26). The sign-in membership gate is new, so
          this refusal has no owning task to inherit wording from; it is
          modelled line-for-line on task 0189's `issue=unknown`, which says the
          same thing one press later. If it is wrong it is wrong in both
          places — fix them together.

          Its sibling, `not_member`, is not here: it is a SCREEN, above. That
          asymmetry is 0193's acceptance criterion made structural — a refusal
          the visitor can act on takes over the card and names the way out; one
          that says only "we could not check" stays a banner over a sign-in
          button that still works, because retrying is the whole remedy. */}
      {unverified && (
        <Callout
          variant="error"
          icon={<SyncProblemRoundedIcon sx={{ fontSize: 18 }} />}
          title="We could not check your membership"
        >
          <p data-testid="signin-unknown">
            That is a problem talking to Discord, not a statement about your
            membership. Please try signing in again shortly.
          </p>
        </Callout>
      )}
      {/* ⚠️ **The two prerequisites, back above the button since 2026-08-27.**
          "You are not signed in." and task 0189's two-rules paragraph were
          both removed here on 2026-08-26, at Adam's instruction, to match
          Figma `824:140`, on the argument that the landing page's FAQ states
          the same rules. PR #249's review named what that cost — the FAQ is
          a COLLAPSED accordion and states nothing until opened, and on
          `/login` the card is the whole page and named neither rule — and
          this task's acceptance criterion still reads "states both
          prerequisites before the sign-in button". Restored as one small
          line rather than the paragraph: 0189's words, verbatim, in the
          card's tertiary type, ABOVE the control, so a developer learns the
          rules before authorising an app for nothing. The FAQ keeps the long
          form. The frame does not draw this line; the criterion does. */}
      <Typography
        variant="body2"
        data-testid="prerequisites"
        sx={{ color: color.text.tertiary, m: 0 }}
      >
        Getting an API key needs two things, both checked via Discord when you
        ask for one: membership of the{' '}
        <a href={STELLAR_DISCORD_INVITE}>Stellar Developers Discord</a>, and a
        Discord account that is not brand new.
      </Typography>
      {/* A link, not a button with an onClick. The OAuth flow is a top-level
          navigation to discord.com and back; `fetch` cannot perform one, and
          the session cookie is `SameSite=Lax` precisely so that this navigation
          carries it. `href` is relative, so it stays same-origin.

          `DiscordButton` renders an `<a>` — `component="a"` with an `href` —
          so it is still a link to the browser, to a screen reader and to the
          tests that pin it by role. Only its appearance changed. */}
      <DiscordButton href={signInUrl()} onClick={onSignInClick}>
        Sign in with Discord
      </DiscordButton>
      <LabelledRule>What you get</LabelledRule>
      <Benefits items={BENEFITS} />
    </LoginCard>
  );
}

/**
 * The card's primary control, in Discord's blurple.
 *
 * Always an `<a>`: every caller is starting an OAuth round-trip, which is a
 * top-level navigation that `fetch` cannot perform and that the `SameSite=Lax`
 * session cookie depends on. Rendering it as a `<button>` would break the flow
 * and the tests that find it by link role.
 *
 * Blurple rather than the brand yellow because the action is "hand me to
 * Discord", and a button that looks like the rest of the site sets the wrong
 * expectation about which consent screen is about to appear. Contrast of white
 * on #5865f2 is 4.6:1, which clears AA for the 16 px semibold label.
 */
function DiscordButton({
  href,
  onClick,
  target,
  children,
}: {
  href: string;
  onClick?: (event: MouseEvent<HTMLAnchorElement>) => void;
  /**
   * `_blank` for the one caller that sends the visitor to discord.com to do
   * something there and come BACK — switching account. `rel` is set from this
   * rather than passed separately, because `noopener` is not optional on a
   * `_blank` and leaving it to the call site is how one of them ends up
   * without it.
   */
  target?: '_blank';
  children: ReactNode;
}) {
  return (
    <Button
      component="a"
      href={href}
      onClick={onClick}
      target={target}
      rel={target === '_blank' ? 'noopener noreferrer' : undefined}
      variant="contained"
      fullWidth
      startIcon={<DiscordIcon />}
      sx={{
        backgroundColor: DISCORD,
        color: color.white,
        minHeight: 48,
        '&:hover': { backgroundColor: '#4752c4' },
      }}
    >
      {children}
    </Button>
  );
}

/**
 * The refusal card's second action — the frame's "‹ Try different account".
 *
 * ⚠️ **A button that opens [`SwitchAccountDialog`], not a link into the
 * round-trip (Adam, 2026-08-26).** It WAS the link, and the link did nothing
 * useful: `/auth/login` sends no `prompt`, so a visitor Discord already has a
 * session for is re-authorised against the same account, the popup returns the
 * identical refusal, and the screen redraws unchanged. Pressing a control and
 * getting the screen you were already on is indistinguishable from a broken
 * button, which is what it was reported as.
 *
 * What replaced it is the dialog, because switching account is not something
 * this application can perform — see its docs.
 *
 * Quiet rather than filled: the frame gives the card one filled control and
 * this is not it. The chevron is `aria-hidden` — it is direction, not meaning,
 * and "less-than sign, try different account" is what a screen reader would
 * otherwise say.
 */
function TryDifferentAccount({ onClick }: { onClick: () => void }) {
  return (
    <Button
      type="button"
      onClick={onClick}
      fullWidth
      startIcon={<ChevronLeftRoundedIcon aria-hidden />}
      data-testid="switch-account-open"
      sx={{
        minHeight: 44,
        ...theme.typography.body1,
        fontWeight: 500,
        color: color.text.tertiary,
        '&:hover': {
          color: color.text.primary,
          backgroundColor: alpha(color.gray[50], 0.06),
        },
      }}
    >
      Try different account
    </Button>
  );
}

/**
 * "‹ Back to landing" drawn INSIDE a card, under its primary action.
 *
 * ⚠️ The OAuth error screen only (Adam, 2026-08-26, Figma `825:1284`). Every
 * other login state keeps the link above the card, where `LoginSection` puts
 * it; this one claims it with `useOwnBackLink` so the section stands its own
 * down and the page never carries two.
 *
 * A router `Link` behind a `Button`, matching `TryDifferentAccount` beside it:
 * in-app navigation, so a full document load here would throw away the
 * `/config` answer and the session lookup the app has already made.
 */
function BackToLanding() {
  return (
    <Button
      component={RouterLink}
      to="/"
      fullWidth
      startIcon={<ChevronLeftRoundedIcon aria-hidden />}
      sx={{
        minHeight: 44,
        ...theme.typography.body1,
        fontWeight: 500,
        color: color.text.tertiary,
        '&:hover': {
          color: color.text.primary,
          backgroundColor: alpha(color.gray[50], 0.06),
        },
      }}
    >
      Back to landing
    </Button>
  );
}

/** Where a visitor switches Discord account. Discord's, not ours. */
const DISCORD_LOGIN = 'https://discord.com/login';

/**
 * "Use a different Discord account" — what `Try different account` opens.
 *
 * **Why a dialog and not simply another sign-in link.** OAuth gives a client no
 * way to ask for an account chooser: Discord's `prompt` takes `consent` and
 * `none` and nothing else — there is no `select_account` — so an application
 * cannot make the authorisation screen offer a different account. Whoever
 * Discord has a session for is whoever comes back. The honest thing is to say
 * that and hand over the two steps that actually work, rather than to keep
 * re-running a round-trip whose answer cannot change.
 *
 * Both actions are real and neither is dressed as the other:
 *
 * - **Switch on Discord** opens `discord.com/login` in a NEW tab, so this card
 *   and its refusal survive the trip. A link, `target="_blank"` with
 *   `noopener` — never a script, on a page that renders credentials elsewhere
 *   and keeps its CSP trivial ([0193]).
 * - **Try signing in again** re-runs the round-trip, which is the right action
 *   AFTER the first one and a no-op before it. Listed second for that reason,
 *   and worded as a retry rather than as a switch.
 *
 * It claims nothing about the outcome: signing in as another account reaches
 * the same membership gate, and if that account is not a member either the
 * visitor lands back here. Saying "then you will be in" would be inventing a
 * verdict this page cannot hold.
 */
function SwitchAccountDialog({
  onClose,
  onSignIn,
}: {
  onClose: () => void;
  onSignIn: (event: MouseEvent<HTMLAnchorElement>) => void;
}) {
  return (
    <Dialog
      open
      onClose={onClose}
      aria-labelledby="switch-account-title"
      maxWidth="sm"
      fullWidth
      slotProps={{
        paper: {
          sx: {
            backgroundColor: color.surface.gray,
            border: cardBorder,
            borderRadius: `${radius.lg}px`,
            p: { xs: 2.5, sm: 3 },
            m: 2,
          },
        },
        backdrop: { sx: { backgroundColor: alpha(color.black, 0.72) } },
      }}
    >
      <Stack spacing={2.5} data-testid="switch-account-dialog">
        {/* Discord's own blurple, because every step below happens on their
            side. `aria-hidden` — the heading is the label. */}
        <Box
          aria-hidden
          sx={{
            width: 56,
            height: 56,
            borderRadius: '50%',
            display: 'grid',
            placeItems: 'center',
            backgroundColor: DISCORD,
            color: color.white,
          }}
        >
          <SwitchAccountRoundedIcon sx={{ fontSize: 28 }} />
        </Box>

        <Typography
          id="switch-account-title"
          component="h2"
          sx={{
            fontFamily: font.primary,
            fontWeight: 700,
            fontSize: { xs: '1.5rem', sm: '1.75rem' },
            lineHeight: 1.15,
            color: color.text.primary,
          }}
        >
          Use a different Discord account
        </Typography>

        <Typography
          data-testid="switch-account-explainer"
          sx={{ ...theme.typography.body1, color: color.text.secondary }}
        >
          Discord signs you in as whichever account it already has a session
          for, and we cannot change that from here. Switch account on Discord
          first, then come back and sign in again.
        </Typography>

        <Stack spacing={1.5}>
          <DiscordButton href={DISCORD_LOGIN} target="_blank">
            Switch account on Discord
          </DiscordButton>
          <Button
            component="a"
            href={signInUrl()}
            onClick={onSignIn}
            fullWidth
            variant="outlined"
            sx={{
              minHeight: 48,
              ...theme.typography.body1,
              fontWeight: 700,
              color: color.text.primary,
              borderColor: color.stroke.default,
              '&:hover': {
                borderColor: color.text.primary,
                backgroundColor: alpha(color.gray[50], 0.06),
              },
            }}
          >
            Try signing in again
          </Button>
          <Button
            type="button"
            onClick={onClose}
            fullWidth
            sx={{
              minHeight: 44,
              ...theme.typography.body2,
              fontWeight: 700,
              color: color.text.tertiary,
              '&:hover': {
                color: color.text.primary,
                backgroundColor: alpha(color.gray[50], 0.06),
              },
            }}
          >
            Cancel
          </Button>
        </Stack>
      </Stack>
    </Dialog>
  );
}

/**
 * The refusal card's footer — the frame's "If this keeps happening, contact
 * support or check our status page."
 *
 * ⚠️ **The frame underlines both as links; here they are plain text**, and
 * that is [`Legal`]'s rule applied rather than an omission: neither a support
 * destination nor a status page exists in this build (`landing/links.ts` holds
 * every off-page destination the portal names, and neither is among them). A
 * link to a placeholder is a promise the page cannot keep, and it is a worse
 * failure here than on the legal line — this sentence is read by somebody
 * already stuck. Give this component the two URLs when they exist and the
 * words become links without moving.
 */
function KeepsHappening() {
  return (
    <Typography variant="body2" sx={{ color: color.text.tertiary }}>
      If this keeps happening,{' '}
      <Box component="span" sx={UNDERLINED}>
        contact support
      </Box>{' '}
      or check our{' '}
      <Box component="span" sx={UNDERLINED}>
        status page
      </Box>
      .
    </Typography>
  );
}

/**
 * The frame's underline on a footer's named destinations — the OAuth error
 * card's "contact support" and "status page".
 *
 * ⚠️ **Underlined at Adam's instruction (2026-08-26), and still `<span>` and
 * not `<a>`**, because neither destination exists in this build —
 * `landing/links.ts` holds every off-page target the portal names and neither
 * a support address nor a status page is among them. The rule this follows:
 * a link to a placeholder is a promise the page cannot keep. The two are
 * therefore drawn as the frame draws them and are not clickable, which is a
 * known and deliberate mismatch — swap the `<span>`s for `<a>`s the day the
 * URLs land and nothing else moves. (The legal footer that used to share
 * this rule is no longer rendered at all — see the note below `UNDERLINED`.)
 */
const UNDERLINED = {
  textDecoration: 'underline',
  textUnderlineOffset: '0.2em',
  color: color.text.secondary,
} as const;

/*
 * ⚠️ **The card's legal footer is not rendered — task 0193, decision #4,
 * 2026-08-27.** The frame's "By continuing you agree to our Terms of Service
 * and Privacy Policy" was drawn here as two underlined `<span>`s, because
 * neither document exists yet and a link to a placeholder is a promise the
 * page cannot keep. PR #249's review named the other half of that: a sentence
 * asking the visitor to agree to two documents they cannot open is a promise
 * too, and a worse one. The sentence returns as two `<a>`s the day the URLs
 * land in `landing/links.ts`; until then the cards carry no `footer`.
 */

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
  session,
  rateLimit,
  onSignOut,
}: {
  session: PortalSession;
  /** The free plan's per-second rate limit, straight from `/config`. */
  rateLimit?: number;
  /**
   * ⚠️ Back on this component since 2026-08-26, for one caller only: the
   * revoked card puts "Sign out" among its actions, as the frame draws it.
   * Sign-out otherwise lives in the navbar, which is why this prop had been
   * removed — see `DashboardRoute`.
   */
  onSignOut: () => void;
}) {
  const landingSearch = useLocation().search;
  const [keyOnScreen, setKeyOnScreen] = useState(false);
  // Task 0191: a revoke in THIS page load. The key leaves the screen, and the
  // usage section is told so — its cached "no key" copy ("your key is new")
  // would otherwise describe a key just deactivated — and re-asked, because
  // the backend evicted its cache on the revoke.
  const [revokedCount, setRevokedCount] = useState(0);
  // Task 0191 + the revoked dashboard (Adam, 2026-08-26): the account's key is
  // gone, learned either on this page load or from the confirmation dialog.
  const [revoked, setRevoked] = useState<PortalKeyRevoked | null>(null);
  /**
   * The account has no key at all, per the key card above (`onKeyAbsent`).
   *
   * `false` until told otherwise, which is what keeps the two cards below
   * showing their loading state on a cold load rather than flashing empty and
   * then filling in.
   *
   * ⚠️ **This empties both tiles** (Adam, 2026-08-26, from the
   * `Dashboard - no key` frame). It is a deliberate reversal of the rule the
   * rest of this slice works under — "every non-happy state gets a screen, not
   * a blank page" — and it is narrow: the ONE state where the whole dashboard
   * has a single action, on the card above, and the frame gives the other two
   * nothing so that nothing competes with it. Every other empty-ish state
   * (revoked, nothing recorded yet, the portal closed) still says its
   * sentence.
   */
  const [keyAbsent, setKeyAbsent] = useState(false);

  /**
   * Whether THIS page load landed carrying an `?issue=…` outcome.
   *
   * ⚠️ A landing that carries one keeps the ordinary dashboard, because the
   * revoked card cannot say what those outcomes say: `not_member`,
   * `too_young`, `unknown` and `failed` are all reachable by an account whose
   * key is revoked, and each is a different reason the issue round-trip did
   * not produce one. Swallowing them behind a card that only knows a date
   * would leave the visitor with no explanation at all.
   *
   * A snapshot taken ONCE, not a live read. `ApiKey` consumes the parameter
   * with `useOneShotParams`, which strips it from the URL — and if this
   * component had early-returned before that ran, the banner would render for
   * a frame and vanish as the panel unmounted.
   */
  const [issueOnLanding] = useState(() => {
    const landed = new URLSearchParams(landingSearch);
    // Both keys, not `issue` alone. Since sign-in started running the
    // membership check (2026-08-26) the two refusals this guard exists to
    // keep on screen also arrive as `?signin=not_member` / `?signin=unknown`
    // — and an account whose key is revoked and which has since left the
    // Discord is exactly the one that gets both at once. Reading only `issue`
    // put the revoked card over the refusal, and the one-shot parameter was
    // already consumed by then, so a reload could not bring it back.
    return landed.get('issue') !== null || landed.get('signin') !== null;
  });
  // The quota `/usage` reported, so the key card's "Monthly quota" field can
  // state a number this page was actually told. `undefined` until the panel
  // below has an answer; the field is simply absent until then.
  const [usageFacts, setUsageFacts] = useState<{
    quota: number | null;
    resetsAt: string | null;
  }>({ quota: null, resetsAt: null });

  // ⚠️ **A revoked key replaces the whole dashboard** (Adam, 2026-08-26).
  //
  // A PARTIAL revocation is deliberately excluded: task 0191's warning tells
  // that visitor to reload and press Regenerate again, and this card has no
  // key panel to press it in. They keep the ordinary dashboard — where the
  // warning renders under the revoked state, and where a reload adopts the
  // duplicate that refused to be disabled and brings the control back.
  if (revoked && !revoked.partial && !issueOnLanding) {
    return <RevokedDashboard revoked={revoked} onSignOut={onSignOut} />;
  }

  return (
    <Stack spacing={3}>
      <Stack spacing={1}>
        {/* The page's `h1`. The design puts the word "Dashboard" at the top of
            the page and the card titles below it, which is also the right
            outline: one page subject, three panels under it. */}
        {/* 24px: measured equal to the card titles on the frame, so it takes
            the same step of the scale — `h3` (28px) was one step too large. */}
        <Typography variant="h4" component="h1" color="text.primary">
          Dashboard
        </Typography>
        <Typography variant="body1" sx={{ color: color.text.tertiary }}>
          Copy your API Key below and make your first request
        </Typography>
      </Stack>

      {/* Tasks 0187 + 0189. Inside the authenticated branch, so signing out
          removes it along with the key it was showing — the component unmounts
          and its state goes with it, rather than leaving a stale credential on
          screen for the next person at the keyboard. */}
      <ApiKey
        onKey={() => setKeyOnScreen(true)}
        onRevoked={() => {
          setKeyOnScreen(false);
          setRevokedCount((n) => n + 1);
        }}
        onRevokedState={setRevoked}
        onKeyAbsent={setKeyAbsent}
        session={session}
        rateLimit={rateLimit}
        quota={usageFacts.quota}
        resetsAt={usageFacts.resetsAt}
      />

      {/* Two columns at the design's 5:3 ratio, one at 375px. `align-items:
          start` so the shorter card does not stretch to match the taller one —
          the Rate Limit panel has a fixed amount to say. */}
      <Box
        sx={{
          display: 'grid',
          // Measured off the frame: 740 and 524 either side of a 16px gutter,
          // which is 7fr : 5fr of the 1264 that leaves. The old 5fr : 3fr gave
          // the usage card 62% where the design gives it 58%, and the Rate
          // Limit card's two figures wrapped a line early because of it.
          gap: 2,
          alignItems: 'start',
          gridTemplateColumns: { xs: '1fr', md: '7fr 5fr' },
        }}
      >
        {/* Task 0188. Keyed refetch: a key appearing on screen (revealed on
            mount, or fresh off 0189's issue round-trip) re-asks for usage, so
            the section leaves "no key yet" without a manual refresh. Task
            0191 adds the other direction: an in-page revoke re-asks too. */}
        <Usage
          keyAbsent={keyAbsent}
          keyOnScreen={keyOnScreen}
          revokedCount={revokedCount}
          rateLimit={rateLimit}
          onUsage={(usage) =>
            setUsageFacts({
              quota: usage?.limit ?? null,
              resetsAt: usage?.resets_at ?? null,
            })
          }
        />
        <RateLimitCard rateLimit={rateLimit} keyAbsent={keyAbsent} />
      </Box>
    </Stack>
  );
}

/**
 * "Your API key was deactivated on <date>. It stopped working within about
 * half a minute of that instant…" — task 0191's sentence, and the only copy in
 * this file that two screens share.
 *
 * Extracted 2026-08-26 when the revoked dashboard arrived: it is stated both
 * in that card's Reason box and, for a PARTIAL revocation, in the ordinary
 * dashboard's key panel. Two copies of a decided sentence is how the two
 * wordings drift apart.
 *
 * The `<strong data-testid="revoked-at">` and the tense switch are 0191's and
 * are not re-decided here: the present tense holds only while the propagation
 * window is still open, because this same sentence renders days later on the
 * reveal path, where "treat it as live" would be false.
 */
function DeactivationSentence({ revoked }: { revoked: PortalKeyRevoked }) {
  const at = describeUtcInstant(revoked.revoked_at);
  const fresh = revokedJustNow(revoked.revoked_at);
  return (
    <p>
      Your API key was deactivated
      {at && (
        <>
          {' on '}
          <strong data-testid="revoked-at">{at}</strong>
        </>
      )}
      {fresh
        ? `. It stops working ${PROPAGATION_COPY}${
            at ? ' of that instant' : ''
          } — until then treat it as live — and anything still using it will break.`
        : `. It stopped working ${PROPAGATION_COPY}${
            at ? ' of that instant' : ''
          }, and anything still using it is broken.`}
    </p>
  );
}

/**
 * The badge behind the revoked card's glyph — a pale wash with a dark glyph.
 *
 * ⚠️ These were sampled off the PNG (`#fde8e8` / `#8b1a1a`) until the Figma
 * variables for node `997:2210` were read on 2026-08-26. They ARE design-system
 * values — `Red/100` and `Red/950` — so they come through `tokens.ts` now, and
 * a designer changing the red scale changes this with it.
 */
const REVOKED_BADGE_SURFACE = color.red[100];
const REVOKED_BADGE_GLYPH = color.red[950];

/** One labelled panel inside the revoked card — the frame's two inset boxes. */
function RevokedPanel({
  label,
  align = 'left',
  labelVariant = 'body1',
  children,
}: {
  label: string;
  align?: 'left' | 'center';
  /**
   * The frame gives its two panels different label sizes, measured against the
   * variables for node `997:2210`: "Next key available" is `Body/Regular/500`
   * (16) and "Reason" is `Body/Extra Small/500` (12). One is the card's second
   * headline, the other is a field name — so this is a real distinction, not
   * drift.
   */
  labelVariant?: 'body1' | 'caption';
  children: ReactNode;
}) {
  return (
    <Stack
      spacing={0.5}
      sx={{
        p: 2,
        borderRadius: `${radius.md}px`,
        backgroundColor: color.surface.grayAlt,
        textAlign: align,
      }}
    >
      <Typography variant={labelVariant} sx={{ color: color.text.tertiary }}>
        {label}
      </Typography>
      {children}
    </Stack>
  );
}

/**
 * The whole dashboard, replaced by one card, for an account that ARRIVES with
 * its key already revoked (Adam, 2026-08-26, Figma `997:2114` / `1000005216`).
 *
 * **Why it replaces the page rather than sitting in it.** Every other panel the
 * dashboard draws is about a key: the masked value, usage against quota, the
 * rate limit. With no key they would all render their empty states at once,
 * which is four ways of saying the same thing and none of them the thing that
 * matters — when a new key can be issued.
 *
 * **The frame's copy is used verbatim** (Adam, 2026-08-26). It is written for
 * an operator-side suspension — "suspended for this account", a fixed Reason
 * about quota abuse, "Contact us about this decision" — while the only way to
 * reach this state today is the owner revoking their own key (task 0191). That
 * was raised and Adam decided; the wording is his to change and is not
 * re-decided here.
 *
 * Two things the frame does not cover, both additive and invisible in the
 * mock:
 *
 * - **Once the period HAS rolled**, the primary control becomes "Get my API
 *   key". The frame's footer says a new key arrives automatically on signing
 *   in, which nothing implements — issuance is the OAuth round-trip
 *   `issueUrl()` starts. Without this the account has no way out of the card.
 * - A PARTIAL revocation never reaches here, and neither does a revoke that
 *   happened in this page load — see `Dashboard`.
 */
function RevokedDashboard({
  revoked,
  onSignOut,
}: {
  revoked: PortalKeyRevoked;
  onSignOut: () => void;
}) {
  const waiting = stillWaiting(revoked.next_eligible_at);
  const nextEligible = describeNextEligible(revoked.next_eligible_at);

  return (
    <Box
      sx={{ display: 'flex', justifyContent: 'center', py: { xs: 2, md: 6 } }}
    >
      <Box
        data-testid="key-revoked"
        sx={{
          width: '100%',
          maxWidth: LOGIN_CARD_MAX_WIDTH,
          borderRadius: `${radius.lg}px`,
          border: cardBorder,
          backgroundColor: color.surface.gray,
          overflow: 'hidden',
        }}
      >
        <Stack spacing={2} sx={{ p: 4, textAlign: 'center' }}>
          {/* A rounded square, not a disc — the frame's shape. The two colours
              are sampled from the export; neither is in the Figma variable
              set, so they are named as literals for the same reason
              `LoginCard`'s `ERROR_SURFACE`/`ERROR_EDGE` are. */}
          <Box
            aria-hidden
            sx={{
              alignSelf: 'center',
              width: 56,
              height: 56,
              borderRadius: `${radius.md}px`,
              display: 'grid',
              placeItems: 'center',
              backgroundColor: REVOKED_BADGE_SURFACE,
              color: REVOKED_BADGE_GLYPH,
            }}
          >
            <ErrorOutlineRoundedIcon sx={{ fontSize: 30 }} />
          </Box>
          {/* `h4`, not `h3`: the frame's `Fontsize/Heading/h4` is 32 at
              weight 600, which is exactly what the theme's fluid `h4`
              (24 → 32) reaches. `h3` tops out at 40 and was a step too large. */}
          <Typography variant="h4" component="h1" color="text.primary">
            Your API key has been revoked
          </Typography>
          <Typography variant="body1" sx={{ color: color.text.tertiary }}>
            Access to the Prices API has been suspended for this account. A new
            key will be available from the date below.
          </Typography>
        </Stack>

        <Stack spacing={2} sx={{ px: 4, pb: 4 }}>
          <RevokedPanel label="Next key available" align="center">
            {/* The same `Heading 4` as the card's title, measured equal on
                the frame — and `Text/Accent`. Was a hand-rolled 28px at
                weight 700, which is neither. */}
            <Typography
              variant="h4"
              data-testid="revoked-next-eligible"
              sx={{ color: color.text.accent }}
            >
              {nextEligible}
            </Typography>
          </RevokedPanel>

          <RevokedPanel label="Reason" labelVariant="caption">
            <Typography
              variant="body2"
              data-testid="revoked-reason"
              sx={{ color: color.text.secondary }}
            >
              Monthly quota exceeded repeatedly. Key was suspended to protect
              shared infrastructure.
            </Typography>
          </RevokedPanel>

          <Stack spacing={1}>
            {/* ⚠️ The frame's filled control names no destination, and this
                build has none to give it: `landing/links.ts` holds every
                off-page target the portal names and there is no support
                address among them. Rendered as the frame draws it and wired to
                nothing — the same standing gap as "Contact support" below and
                `Legal`'s two documents. Give it an `href` and it becomes a
                link without moving. */}
            {waiting ? (
              <Button
                type="button"
                variant="contained"
                fullWidth
                data-testid="revoked-contact"
                sx={REVOKED_PRIMARY}
              >
                Contact us about this decision
              </Button>
            ) : (
              // The period has rolled, so the action that changes something is
              // available. See the component docs.
              <Button
                component="a"
                href={issueUrl()}
                variant="contained"
                fullWidth
                data-testid="revoked-issue"
                sx={REVOKED_PRIMARY}
              >
                Get my API key
              </Button>
            )}
            <Button
              type="button"
              onClick={onSignOut}
              fullWidth
              data-testid="revoked-sign-out"
              sx={{
                minHeight: 44,
                ...theme.typography.body1,
                fontWeight: 500,
                color: color.text.primary,
                '&:hover': { backgroundColor: alpha(color.gray[50], 0.06) },
              }}
            >
              Sign out
            </Button>
          </Stack>
        </Stack>

        <Divider sx={{ borderColor: alpha(color.stroke.default, 0.45) }} />

        <Box
          sx={{
            backgroundColor: color.surface.grayAlt,
            px: 3,
            py: 2,
            textAlign: 'center',
          }}
        >
          {/* `Body/Extra Small/500` (12), `Text/Tertiary`. */}
          {/* `component="p"`: MUI renders `caption` as a `<span>`, which is
              inline and does not take the centred block's width — the two
              lines the frame draws would not wrap where it wraps them. */}
          <Typography
            variant="caption"
            component="p"
            sx={{ color: color.text.tertiary }}
          >
            After {nextEligible}, sign in again to receive a new key
            automatically. Questions?{' '}
            <Box component="span" sx={UNDERLINED}>
              Contact support.
            </Box>
          </Typography>
        </Box>
      </Box>
    </Box>
  );
}

/** The revoked card's filled control — the frame's brand-yellow pill. */
const REVOKED_PRIMARY = {
  minHeight: 48,
  // `Body/Regular/500` — the frame's button label is medium, where the theme's
  // `button` variant is 700. Weight only; size and face already match.
  fontWeight: 500,
  backgroundColor: color.surface.primary,
  color: color.black,
  '&:hover': { backgroundColor: color.primary[300] },
} as const;

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
 * right for 0191's weeks-long re-issue wait and absurd for minutes — and never a
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
 * Render the date a revoked key's replacement can be issued (task 0191).
 *
 * The pattern `describeWait` deliberately avoids — a calendar date — is the
 * right one here, because the wait is weeks: "1 September 2026" is what a
 * visitor who revoked on 3 August needs to read. The value arrives from the
 * revoke answer, from the reveal's `key_revoked` envelope (RFC 3339) or from
 * the `?issue=capped` landing URL (`YYYY-MM-DD`); in every case it is OUR
 * instant — the 1st of the next month, 00:00 UTC, under the period rule the
 * backend states — so it is rendered in UTC and not in the viewer's zone,
 * where "1 September 00:00 UTC" can read as 31 August.
 *
 * Sanitised, because one of the sources is a URL: anything that is not a date
 * at the front renders as "the start of the next quota period", which is true
 * of every capped issue whatever the URL said.
 */
function describeNextEligible(value: string | null | undefined): string {
  const match = value ? /^(\d{4})-(\d{2})-(\d{2})/.exec(value) : null;
  if (!match) return 'the start of the next quota period';
  const [, year, month, day] = match;
  const date = new Date(Date.UTC(Number(year), Number(month) - 1, Number(day)));
  // `Date.UTC` does not reject an impossible month or day — it rolls them
  // over, so `2026-13-45` comes back as a real date in February 2027 rather
  // than `NaN`. Reading the fields back is what tells a date from garbage.
  if (
    Number.isNaN(date.getTime()) ||
    date.getUTCMonth() + 1 !== Number(month) ||
    date.getUTCDate() !== Number(day)
  ) {
    return 'the start of the next quota period';
  }
  return date.toLocaleDateString('en-GB', {
    day: 'numeric',
    month: 'long',
    year: 'numeric',
    timeZone: 'UTC',
  });
}

/**
 * Render an RFC 3339 instant as a UTC time — "21 August 2026, 12:00 UTC" —
 * for the revocation instant (task 0191). UTC, not the viewer's zone, for the
 * same reason as `describeNextEligible`.
 *
 * `null` for a missing or unparseable value, and the caller renders the
 * revocation WITHOUT an instant: the backend sends no `revoked_at` when no
 * record carries a date, and every stand-in reads as a statement of fact the
 * page cannot make — "deactivated on just now", or (worse, via the
 * next-eligible phrasing) "deactivated on the start of the next quota
 * period".
 */
function describeUtcInstant(value: string | undefined): string | null {
  if (!value) return null;
  const at = new Date(value);
  if (Number.isNaN(at.getTime())) return null;
  const day = at.toLocaleDateString('en-GB', {
    day: 'numeric',
    month: 'long',
    year: 'numeric',
    timeZone: 'UTC',
  });
  const time = at.toLocaleTimeString('en-GB', {
    hour: '2-digit',
    minute: '2-digit',
    timeZone: 'UTC',
  });
  return `${day}, ${time} UTC`;
}

/**
 * The 1st of next month, in UTC — our quota period rule, computed.
 *
 * The backend states the same instant in `/usage`'s `resets_at`
 * (`portal/period.rs`), and the page prefers that value wherever it has it.
 * This is the fallback for the moment before that request answers, and it is
 * a computation rather than a guess: "the 1st of the month, 00:00 UTC" is OUR
 * definition, not something AWS reports (ADR 0010, correction #2).
 */
function describeNextPeriodStart(): string {
  const now = new Date();
  const next = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth() + 1, 1),
  );
  return next.toLocaleDateString('en-GB', {
    day: 'numeric',
    month: 'long',
    year: 'numeric',
    timeZone: 'UTC',
  });
}

/**
 * The circled glyph inside a dashboard button — the frame's treatment.
 *
 * Both controls carry one: a black disc with a yellow glyph on the yellow
 * button, a yellow disc with a black glyph on the dark one. The same shape the
 * landing page's `ArrowBadge` draws, and a real element rather than a bordered
 * icon for the same reason: the two variants differ by colour alone.
 *
 * `aria-hidden`, always — the word beside it is the label.
 */
function GlyphBadge({
  icon: Icon,
  tone,
}: {
  icon: typeof ContentCopyRoundedIcon;
  tone: 'onPrimary' | 'onDark';
}) {
  const onPrimary = tone === 'onPrimary';
  return (
    <Box
      aria-hidden
      sx={{
        width: 24,
        height: 24,
        flexShrink: 0,
        borderRadius: '50%',
        display: 'grid',
        placeItems: 'center',
        backgroundColor: onPrimary ? color.black : color.primary[400],
        color: onPrimary ? color.primary[400] : color.black,
      }}
    >
      <Icon sx={{ fontSize: 14 }} />
    </Box>
  );
}

/**
 * A key's timestamp as a plain UTC date — "13 April 2026".
 *
 * Date only, where `describeUtcInstant` gives date and time: the metadata row
 * states when a key came into being, and the minute it happened is noise
 * beside an id and an account. UTC for the reason task 0191 decision #15
 * gives: rendered in the viewer's zone, a key minted at 00:30 UTC on the 1st
 * would be dated to the previous month west of Greenwich.
 *
 * `null` for a missing or unparseable value, so the caller can drop the field
 * rather than print "Invalid Date".
 */
function describeUtcDay(value: string | null | undefined): string | null {
  if (!value) return null;
  const at = new Date(value);
  if (Number.isNaN(at.getTime())) return null;
  return at.toLocaleDateString('en-GB', {
    day: 'numeric',
    month: 'long',
    year: 'numeric',
    timeZone: 'UTC',
  });
}

/**
 * How long a disable takes to reach the data plane, in the words the page
 * uses. Measured under task 0180 item 8: a disabled key kept answering `200`
 * for ~25 s before the `403` arrived. Said out loud rather than hidden behind
 * "immediately", because a visitor revoking a LEAKED key is exactly the
 * person who must not stop worrying one second too early.
 */
const PROPAGATION_COPY = 'within about half a minute';

/**
 * How long the sign-in poll and the closed-window watch hold their verdict so
 * the popup's `postMessage` can still overtake it (see `afterGrace`). Longer
 * than a queued message task by orders of magnitude; shorter than a visitor
 * notices on top of the round-trip they just made.
 */
const POPUP_MESSAGE_GRACE_MS = 1500;

/**
 * How long after a revocation the propagation window is still worth stating
 * in the present tense. The window is ~25 s; five minutes is generous and
 * keeps the copy honest on the reveal path, which renders the revoked view
 * on every page load — days later, "until then treat it as live" would be
 * telling somebody to keep worrying about a key that died last week.
 */
const PROPAGATION_FRESH_MS = 5 * 60 * 1000;

/** Whether `revokedAt` (RFC 3339) is recent enough for the present tense. */
function revokedJustNow(revokedAt: string | undefined): boolean {
  if (!revokedAt) return false;
  const at = new Date(revokedAt).getTime();
  return !Number.isNaN(at) && Date.now() - at < PROPAGATION_FRESH_MS;
}

/** Whether `nextEligibleAt` (RFC 3339 or `YYYY-MM-DD`) is still ahead. */
function stillWaiting(nextEligibleAt: string): boolean {
  const at = new Date(
    nextEligibleAt.length === 10
      ? `${nextEligibleAt}T00:00:00Z`
      : nextEligibleAt,
  );
  // An unparseable date is treated as STILL WAITING: the safe direction is
  // to withhold the issue link (the server would only refuse it), never to
  // offer it on garbage.
  return Number.isNaN(at.getTime()) || at.getTime() > Date.now();
}

/**
 * The phrase that arms the regeneration.
 *
 * ⚠️ **`regenerate-key`, not task 0191's `delete-key` — Adam, 2026-08-26.**
 * The phrase follows the button: the control has said "Regenerate" since
 * 2026-08-25, and a dialog that demands the word `delete` under a heading that
 * says "Regenerate" is asking the visitor to agree to a different sentence
 * from the one they just read. 0191's REASON for having a typed phrase at all
 * — that this is destructive and must not be reachable by one stray click —
 * is untouched; only the string changed.
 */
const REWORK_CONFIRM_PHRASE = 'regenerate-key';

/**
 * The "Regenerate" confirmation (task 0191's dialog, task 0193's frame).
 *
 * A real modal since 0193: MUI's `Dialog`, so it takes the focus, traps it,
 * closes on `Escape` and dims the dashboard behind it — the frame draws a
 * floating card over a darkened page, and the inline `<section>` this replaced
 * could do none of that. It renders through a portal, OUTSIDE the dashboard
 * chrome's descendant rules, so every element here is styled locally; nothing
 * it draws can be inherited.
 *
 * **The WORDING is task 0191's and is still not re-decided here**, with the
 * two exceptions Adam made by hand and which are marked as such: the verb is
 * "regenerate" (the frame's, and the button's since 2026-08-25) and the typed
 * phrase is `regenerate-key`. Every load-bearing sentence is 0191's:
 *
 * - the current key is **deactivated**, within the propagation window rather
 *   than "immediately", so anything still using it breaks;
 * - **no new key is issued now** — regenerating is a revocation with a dated
 *   replacement, not a swap. If a swap handed out a fresh key (a fresh
 *   counter), this would be the button people press on the 20th of a heavy
 *   month.
 * - Confirm is **disabled until the visitor types the phrase**, and disabled
 *   again the moment it is pressed, so a double-click cannot fire two
 *   requests. The press is a same-origin `POST` (`revokeKey`) — no Discord
 *   round-trip, so a leaked key is killable while Discord is down.
 *
 * **Where the frame and this build disagree, this build is what ships.** The
 * frame's "Important" box reads "Rotation is limited to once per calendar
 * month. After regenerating you will not be able to rotate again until
 * <date>", which describes a product that hands you a new key on the spot.
 * This one does not: it deactivates and issues nothing until the period rolls
 * ([[0191]]'s model, and the settled rotation-vs-revocation answer). The box
 * keeps its position, its ring and its two-colour treatment, and states what
 * actually happens.
 *
 * On success the parent renders the revoked state with the exact date from the
 * backend's answer; on failure the dialog says so and stays open, with the key
 * still live — "revoked" is never shown for a key that still works.
 *
 * **The failure copy holds the other half of that invariant, and it is the
 * weaker half.** "Revoked" is decidable from the response; "still active" is
 * not. A `502` can be a control-plane refusal with nothing written, or a lost
 * response on an `UpdateApiKey` that landed — the page cannot tell, so it says
 * what it knows (the deactivation was not confirmed) and never asserts the key
 * is still working. The partly-succeeded case does not arrive here at all: the
 * backend answers `200 partial`, and the revoked view renders the warning.
 */
function ReplaceKey({
  onClose,
  onRevoked,
}: {
  onClose: () => void;
  onRevoked: (revoked: PortalKeyRevoked) => void;
}) {
  const [typed, setTyped] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const armed = typed === REWORK_CONFIRM_PHRASE;

  const onConfirm = () => {
    // Disabled from the first press, before the request is even made: a
    // second click in the window before the answer must not start a second
    // request. The state change re-renders the button disabled synchronously.
    if (!armed || submitting) return;
    setSubmitting(true);
    setFailure(null);
    revokeKey()
      .then((revoked) => {
        onRevoked({
          revoked: true,
          next_eligible_at: revoked.next_eligible_at,
          revoked_at: revoked.revoked_at,
          partial: revoked.partial,
        });
      })
      .catch((cause: unknown) => {
        setFailure(describeFailure(cause));
        setSubmitting(false);
      });
  };

  return (
    <Dialog
      open
      // Not dismissible mid-flight: a backdrop click or `Escape` while the
      // `POST` is in the air would unmount the one place the answer has to
      // land, and the visitor would be left not knowing whether their key
      // still works — the single thing this dialog exists to never do.
      onClose={submitting ? undefined : onClose}
      aria-labelledby="replace-key-title"
      aria-describedby="replace-key-warning"
      // `sm` (600), not `xs` (444): the frame's card is wide enough that the
      // warning runs two lines rather than four, and the height that buys
      // back is what keeps the whole dialog — badge, rule, input and both
      // controls — on one screen at laptop height without scrolling.
      maxWidth="sm"
      fullWidth
      slotProps={{
        paper: {
          sx: {
            // The card colour, not `MuiPaper`'s default darkest surface: the
            // frame floats this ABOVE the dashboard, and the one surface a
            // raised element takes here is the card's.
            backgroundColor: color.surface.gray,
            border: cardBorder,
            borderRadius: `${radius.lg}px`,
            p: { xs: 2.5, sm: 3 },
            m: 2,
          },
        },
        backdrop: { sx: { backgroundColor: alpha(color.black, 0.72) } },
      }}
    >
      {/* The testid rides the content, not the `Paper`: MUI 7 types
          `slotProps.paper` against `PaperProps`, which admits no `data-*`, and
          reaching the element would mean casting the slot's props — a lie in
          the type system bought for an attribute that works just as well one
          node in. `role="dialog"` and the labelling stay on the `Paper`, where
          the accessibility tree wants them. */}
      <Stack spacing={2.5} data-testid="replace-key-dialog">
        {/* The frame's badge: a cream disc with the brand's own darkest
            yellow as the glyph. `aria-hidden` — the heading under it is the
            label, and "autorenew icon" announced first is noise. */}
        <Box
          aria-hidden
          sx={{
            width: 56,
            height: 56,
            borderRadius: '50%',
            display: 'grid',
            placeItems: 'center',
            backgroundColor: color.primary[100],
            color: color.primary[900],
          }}
        >
          <AutorenewRoundedIcon sx={{ fontSize: 28 }} />
        </Box>

        <Typography
          id="replace-key-title"
          component="h2"
          sx={{
            fontFamily: font.primary,
            fontWeight: 700,
            fontSize: { xs: '1.5rem', sm: '1.75rem' },
            lineHeight: 1.15,
            color: color.text.primary,
          }}
        >
          Regenerate API key?
        </Typography>

        {/* One `data-testid` over BOTH halves of the warning, because both
            halves are the warning: the deactivation sentence and the rule in
            the ringed box are what task 0191 decided had to be said, and the
            tests that pin that copy read this subtree's text. Splitting the
            testid would let one half be deleted with the suite still green. */}
        <Box id="replace-key-warning" data-testid="replace-key-warning">
          <Stack spacing={2}>
            <Typography
              sx={{ ...theme.typography.body1, color: color.text.secondary }}
            >
              Regenerating <strong>deactivates the current one</strong>. It
              stops working {PROPAGATION_COPY} of your confirming — AWS takes
              that long to apply the change, so treat it as live until then —
              and anything still using it will break.
            </Typography>
            {/* Taller than the dashboard's strip (Adam, 2026-08-26): with the
                disc gone the text runs the full width of the box, and 12 px
                top and bottom against a three-line paragraph reads as a
                caption rather than the rule the visitor has to weigh. */}
            <NoticeStrip glyph={false} sx={{ py: 2.5 }}>
              <p>
                <strong>Important:</strong> regenerating is limited to once per
                quota period, and <strong>no new key is issued now</strong>. You
                can generate a new one at the start of the next quota period (
                <strong>the 1st of next month, 00:00 UTC</strong>), and until
                then you will not have a working key.
              </p>
            </NoticeStrip>
            <Typography
              sx={{ ...theme.typography.body2, color: color.text.tertiary }}
            >
              Do this if your key has leaked or you no longer trust where it is.
              If you only want a fresh key, wait for the next period instead.
            </Typography>
          </Stack>
        </Box>

        {/* The typed phrase, under the rule it confirms rather than beside the
            button it arms: the visitor should have read what they are agreeing
            to before there is anywhere to type. */}
        <Stack spacing={1} component="label">
          <Typography
            component="span"
            sx={{ ...theme.typography.body2, color: color.text.secondary }}
          >
            Type{' '}
            <Box
              component="code"
              sx={{
                fontFamily: font.mono,
                fontSize: '0.875em',
                color: color.text.accent,
                backgroundColor: color.surface.grayAlt,
                borderRadius: '4px',
                px: 0.75,
                py: 0.25,
              }}
            >
              {REWORK_CONFIRM_PHRASE}
            </Box>{' '}
            to confirm:
          </Typography>
          <Box
            component="input"
            type="text"
            value={typed}
            onChange={(event: ChangeEvent<HTMLInputElement>) =>
              setTyped(event.target.value)
            }
            autoComplete="off"
            spellCheck={false}
            disabled={submitting}
            data-testid="replace-key-phrase"
            sx={{
              width: '100%',
              minHeight: 44,
              boxSizing: 'border-box',
              px: 1.5,
              fontFamily: font.mono,
              fontSize: '0.9375rem',
              color: color.text.primary,
              backgroundColor: color.surface.grayAlt,
              border: `1px solid ${color.stroke.default}`,
              borderRadius: `${radius.chip}px`,
              '&:focus-visible': {
                outline: `2px solid ${color.stroke.action}`,
                outlineOffset: 2,
                borderColor: 'transparent',
              },
              '&:disabled': { opacity: 0.6 },
            }}
          />
        </Stack>

        {/* The frame's footer: cancel as quiet text, the destructive action in
            the brand fill on the right. Reversed at `xs` into a column with
            the primary action on top, because at 375 px a side-by-side pair
            puts the confirm under the thumb of somebody scrolling. */}
        <Stack
          direction={{ xs: 'column-reverse', sm: 'row' }}
          spacing={1.5}
          // The pair spans the card (Adam, 2026-08-26) rather than sitting
          // right-aligned as the frame draws it: equal halves, so neither
          // reading of "which one is the safe one" is decided by width. At
          // `xs` they stack, and the destructive one stays on TOP of the
          // reversed column — the thumb rests at the bottom of a phone.
          sx={{ alignItems: 'stretch', pt: 0.5, '& > *': { flex: 1 } }}
        >
          <Button
            type="button"
            onClick={onClose}
            disabled={submitting}
            sx={{
              ...theme.typography.body2,
              fontWeight: 700,
              color: color.text.primary,
              '&:hover': { backgroundColor: alpha(color.gray[50], 0.08) },
            }}
          >
            Cancel
          </Button>
          <Button
            type="button"
            variant="contained"
            onClick={onConfirm}
            disabled={!armed || submitting}
            data-testid="replace-key-confirm"
            sx={{
              ...theme.typography.body2,
              fontWeight: 700,
              backgroundColor: color.surface.primary,
              color: color.black,
              '&:hover': { backgroundColor: color.primary[300] },
              '&.Mui-disabled': {
                backgroundColor: color.surface.grayAlt,
                color: color.text.tertiary,
              },
            }}
          >
            {submitting ? 'Deactivating…' : 'Regenerate'}
          </Button>
        </Stack>

        {failure && (
          <Typography
            data-testid="replace-key-failed"
            role="alert"
            sx={{ ...theme.typography.body2, color: color.text.error }}
          >
            Could not confirm the deactivation — your key may or may not have
            been switched off:{' '}
            {/* The face is set here for the same reason the notice's margin
                is: the dashboard chrome's `& code` rule does not reach a
                dialog rendered through a portal, and a failure reason in the
                body face is the one string on this screen a visitor may have
                to copy into a support message. */}
            <Box
              component="code"
              sx={{ fontFamily: font.mono, overflowWrap: 'anywhere' }}
            >
              {failure}
            </Box>
            . Close this and reload the page to see where it stands, then try
            again if it is still live.
          </Typography>
        )}
      </Stack>
    </Dialog>
  );
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
function ApiKey({
  onKey,
  onRevoked,
  onRevokedState,
  onKeyAbsent,
  session,
  rateLimit,
  quota,
  resetsAt,
}: {
  onKey?: () => void;
  /** Task 0191: the key on screen was just deactivated. A fact, no data. */
  onRevoked?: () => void;
  /**
   * The revoked key's details, learned ON MOUNT from `GET /key`.
   *
   * ⚠️ Deliberately NOT fired by the confirmation dialog. A revocation that
   * just happened in this page load keeps the ordinary dashboard: task 0191
   * made the usage section say "your key was deactivated" at exactly that
   * moment, and a partial revocation needs the Regenerate control still on
   * screen to retry. The revoked CARD is for arriving at a dashboard whose key
   * was already gone — which is the state Adam asked for on 2026-08-26.
   *
   * Separate from `onRevoked`, which counts revocations that happened in this
   * page load and exists to re-ask `/usage`.
   */
  onRevokedState?: (revoked: PortalKeyRevoked) => void;
  /**
   * Whether the account has NO key — `GET /key` answered `404 no_key`.
   *
   * Distinct from `onKey`, which reports the opposite fact and only ever fires
   * in the affirmative: the dashboard needs to know the difference between
   * "there is no key" and "we have not been told yet", because the two cards
   * beside this one render empty for the first and render their loading state
   * for the second.
   *
   * Reported from HERE and not derived from `/usage` beside it, though that
   * endpoint also answers `no_key`: the two answers come from different caches
   * and can disagree for a moment, and a dashboard where one tile empties a
   * beat before the other reads as a rendering bug.
   */
  onKeyAbsent?: (absent: boolean) => void;
  session: PortalSession;
  /** The free plan's per-second limit, from `/config`. */
  rateLimit?: number;
  /**
   * The monthly quota, as `/usage` reported it to the panel below — `null`
   * where AWS has recorded nothing yet, `undefined` before it has answered.
   * Lifted rather than fetched again: `GetUsage` is a control-plane call and
   * this page already spends one (task 0194 owns that budget).
   */
  quota?: number | null;
  /**
   * When the current quota period ends, RFC 3339, as `/usage` reported it —
   * and so the instant a key revoked today becomes re-issuable (task 0191:
   * one key per period, the cap decided against the same `Period`). `null`
   * until the panel below has an answer.
   */
  resetsAt?: string | null;
}) {
  type KeyView =
    | { state: 'loading' }
    | { state: 'ok'; key: PortalKey }
    | { state: 'none' }
    | { state: 'revoked'; revoked: PortalKeyRevoked }
    | { state: 'failed'; reason: string };

  const [view, setView] = useState<KeyView>({ state: 'loading' });
  const [error, setError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);
  // ⚠️ `copied` used to be set once and never cleared, so "Copied." stayed
  // under the key for the life of the mount and a SECOND copy gave no sign it
  // had worked — the visitor whose paste went to the wrong window presses
  // again and watches nothing happen. Same two-second window the quick start's
  // `CopyButton` uses, so the two controls behave alike.
  useEffect(() => {
    if (!copied) return;
    const clearing = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(clearing);
  }, [copied]);

  // The issue round-trip's landing state, read once and stripped from the URL
  // — see `useOneShotParams`. `wait_secs` only means anything alongside
  // `issue=too_young`.
  const {
    issue,
    wait_secs: waitSecs,
    next_eligible_at: nextEligibleAt,
  } = useOneShotParams(ISSUE_PARAMS);
  /**
   * The SIGN-IN round-trip's refusal, when it lands on a visitor who is
   * already signed in.
   *
   * ⚠️ **It rendered nowhere at all until 2026-08-27.** ADR 0010 grants the
   * dashboard and the reveal to the session alone — "works forever" — so an
   * account that holds a key and later leaves the guild keeps reading it, and
   * the callback rightly leaves that session standing. But `RootRoute` sends
   * every authenticated visitor to `/dashboard` with the query attached, and
   * `?signin=…` is read only by `LoginView`, on the signed-OUT branch. The
   * refusal was therefore invisible: someone who had left the Stellar Discord
   * signed in, was refused, and met an ordinary dashboard that said nothing.
   *
   * Only the two membership verdicts are read here. `cancelled` and `failed`
   * are about a round-trip that never reached a verdict and belong to the
   * card that offers the button; the dashboard has no button to retry them
   * with. The wording is the issue flow's own, because it is the same two
   * verdicts about the same person with the same remedy — and it is stripped
   * from the URL like every other landing state.
   */
  const { signin } = useOneShotParams(SIGNIN_PARAMS);
  const refusedNotMember = issue === 'not_member' || signin === 'not_member';
  const refusedUnknown = issue === 'unknown' || signin === 'unknown';
  // Whether THIS landing is itself proof a key exists: the backend created
  // one before it redirected.
  const landedWithKey = issue === 'ok';
  /**
   * The first-login treatment — the frame Adam sent for "I just got my first
   * key". True only while the round-trip's `?issue=ok` is still in the URL,
   * which `useOneShotParams` strips on the next render, so a refresh returns
   * the ordinary card.
   */
  const justIssued = landedWithKey && view.state === 'ok';
  /**
   * The `Dashboard - no key` card: the account has no key and this load did not
   * arrive from a completed issue round-trip.
   *
   * Named because three things now key off it — the branch that renders the
   * card, the metadata row it suppresses, and (through `onKeyAbsent`) the two
   * tiles beside it — and three copies of the same condition is how they drift.
   */
  const emptyKeyCard = view.state === 'none' && !landedWithKey;
  /** `createdDate` and `lastUpdatedDate`, as dates — absent where AWS's are. */
  const issuedOn =
    view.state === 'ok' ? describeUtcDay(view.key.created_at) : null;
  const updatedOn =
    view.state === 'ok' ? describeUtcDay(view.key.last_updated_at) : null;
  const [replacing, setReplacing] = useState(false);

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

  // Same ref treatment, same reason: `load` must not re-create itself because
  // a caller passed a fresh arrow, and this component should not depend on
  // every caller remembering to memoise a prop.
  const onRevokedStateRef = useRef(onRevokedState);
  useEffect(() => {
    onRevokedStateRef.current = onRevokedState;
  });

  // Same ref treatment, same reason.
  const onKeyAbsentRef = useRef(onKeyAbsent);
  useEffect(() => {
    onKeyAbsentRef.current = onKeyAbsent;
  });

  // ⚠️ Read through a ref inside `load`, which is a `useCallback([])` — a
  // closure that captures its scope ONCE. The effect below already depends on
  // `landedWithKey` because it can change after mount (`useOneShotParams`
  // resolves the URL and strips it), so reading it directly in `load` would
  // pin the value from first render: an `?issue=ok` landing whose listing has
  // not caught up would report the key as absent and empty the two cards
  // beside it, for the one visitor who has definitely just been given one.
  const landedWithKeyRef = useRef(landedWithKey);
  useEffect(() => {
    landedWithKeyRef.current = landedWithKey;
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
        if (key && 'revoked' in key) {
          // Task 0191: the owner killed it. No value, no issue link — the
          // date instead. Nothing is reported upward: there is no key on
          // screen for the usage section to reason about, though the
          // counter it shows is still the revoked key's (preserved).
          setView({ state: 'revoked', revoked: key });
          onRevokedStateRef.current?.(key);
          // A revoked key is not an ABSENT one: the account had one, the
          // counter it spent is still real, and the revoked card replaces the
          // whole dashboard anyway.
          onKeyAbsentRef.current?.(false);
        } else if (key) {
          setView({ state: 'ok', key });
          // Masked on arrival, always: what nobody asked for is the
          // credential appearing on screen while they were looking elsewhere.
          setRevealed(false);
          // Tell the dashboard a key exists (task 0188's usage section reads
          // it) — the FACT only, never the value or the id: nothing outside
          // this component needs the credential, so nothing outside it gets
          // to hold one.
          onKeyRef.current?.();
          onKeyAbsentRef.current?.(false);
        } else {
          setView({ state: 'none' });
          // ⚠️ NOT while `?issue=ok` is on the URL: the backend created a key
          // during the round-trip that just ended and `GetApiKeys` has simply
          // not caught up (see `keys/mod.rs`). Emptying the cards beside this
          // one for a visitor who has just been given their first key is the
          // worst moment to show them a blank dashboard.
          onKeyAbsentRef.current?.(!landedWithKeyRef.current);
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
    // `?issue=ok` is itself proof a key exists — the
    // backend created or replaced one before it redirected — even though
    // `GetApiKeys` may not list it for a moment yet. Reporting the fact NOW
    // keeps the usage section beside this one from telling a visitor who has
    // just been given their first key that they have none and should issue
    // one.
    if (landedWithKey) {
      onKeyRef.current?.();
      onKeyAbsentRef.current?.(false);
    }
    load();
    return () => cancelInFlight.current?.();
  }, [load, landedWithKey]);

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
    <DashboardCard
      // The first-login frame (Figma `843:2356`) retitles the card for the one
      // page load that follows an issue round-trip: the panel is no longer
      // "API Key", it is the delivery of one. Every later visit is the plain
      // card, because a key that has existed for months greeting its owner
      // with "Your API Key is ready" reads as though it were issued again.
      title={justIssued ? 'Your API Key is ready' : 'API Key'}
      // ⚠️ **"Just issued" for every key that works** — Adam's call against the
      // frame (2026-08-25), replacing a rule that said "Just issued" for the
      // first 24 hours and "Active" after it. That rule is why his own key,
      // minted days ago, kept reading "Active".
      //
      // The word is now a claim the page cannot support for an older key. It
      // is a status pill and nothing acts on it, so the cost is a wrong
      // adjective rather than a wrong instruction. To put the honest rule
      // back: `Date.now() - new Date(view.key.created_at).getTime() < 24h`,
      // with "Active" as the other arm — `created_at` is on the response for
      // exactly this kind of question.
      //
      // "Not issued" is the red pill the design gives the empty state — the
      // `emptyKeyCard` one, NOT every `none`: while `?issue=ok` is on the URL
      // the key exists and the listing has not caught up, and a red "Not
      // issued" over "your key was created" is the page contradicting itself
      // about the one fact the visitor came for. This was the one reader of
      // `view.state === 'none'` that had not moved to the named condition.
      status={
        emptyKeyCard
          ? { label: 'Not issued', tone: 'bad' }
          : view.state === 'ok'
            ? { label: 'Just issued', tone: 'ok' }
            : undefined
      }
    >
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
        <Typography
          data-testid="issue-ok"
          variant="body1"
          sx={{ color: color.text.secondary }}
        >
          Welcome! Your API key has been generated. Copy it below and make your
          first request — the Quick Start guide has everything you need.
        </Typography>
      )}
      {/* Task 0191: eligible, but the key was revoked this quota period, so
          no new one is issued until the date. Worded as a wait with a date,
          like `too_young` — and, like the revoke dialog said it would be,
          without a link that would only be refused. */}
      {issue === 'capped' && (
        <p data-testid="issue-capped">
          No new key yet: you deactivated your previous key this quota period,
          and a replacement can be issued from{' '}
          <strong>{describeNextEligible(nextEligibleAt)}</strong>. Until then
          you do not have a working key.
        </p>
      )}
      {refusedNotMember && (
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
      {refusedUnknown && (
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
      {view.state === 'none' && landedWithKey && (
        <p data-testid="issue-ok-settling">
          Your key was created, and is taking a moment to appear.{' '}
          <button type="button" onClick={reload}>
            Check again
          </button>
        </p>
      )}

      {/* Task 0191: revoked by the owner. The date comes from the backend;
          the issue link appears only once it has passed, because before that
          the round-trip would only be refused — and a dead value is never
          shown again. */}
      {view.state === 'revoked' && (
        <div data-testid="key-revoked">
          {/* Task 0191: the backend could not disable every key under this
              name. The visitor's own key is off — that is what `Partial`
              means — but a duplicate from an earlier double-submit may still
              answer, so the page must not let the paragraph below stand
              alone. First, because it is the sentence that changes what they
              should do next. */}
          {view.revoked.partial && (
            <p data-testid="revoke-partial">
              <strong>One of your keys could not be deactivated.</strong> A
              duplicate under this name may still work against <code>/v1/</code>
              . Reload the page and press <strong>Regenerate</strong> again — it
              retries every key, and it will not cost you a second revocation.
            </p>
          )}
          <DeactivationSentence revoked={view.revoked} />
          {/* Neither sentence below is true of a PARTIAL revocation, so
              neither renders for one. "You do not have a working key" is
              false — the duplicate that refused to be disabled is a working
              key, and the issue path adopts it rather than refusing, so the
              cap the backend computed does not describe what happens next
              either. The warning above already carries the only instruction
              that applies: reload and press Regenerate again. */}
          {view.revoked.partial ? null : stillWaiting(
              view.revoked.next_eligible_at,
            ) ? (
            <p>
              You can generate a new key from{' '}
              <strong>
                {describeNextEligible(view.revoked.next_eligible_at)}
              </strong>
              . Until then you do not have a working key.
            </p>
          ) : (
            <p>
              A new key can be issued now:{' '}
              <a href={issueUrl()}>Get my API key</a>.
            </p>
          )}
        </div>
      )}

      {emptyKeyCard && (
        <>
          {/* The `Dashboard - no key` frame (Adam, 2026-08-26): a red strip and
              one full-width control, and nothing else in the card.

              ⚠️ **This DROPS two sentences that were here** — [`Prerequisites`]
              (task 0189's eligibility wording) and 0187's "One key, on the free
              plan…". Neither is re-worded anywhere; they are simply not on this
              card any more, and the prerequisites are still stated in full on
              the landing page, before the visitor authenticates, which is where
              the epic's criterion puts them. Flagged rather than done quietly,
              because the rule this slice works under is that it re-decides no
              other slice's copy: this is a frame saying what the card contains,
              not this slice deciding those sentences were wrong.

              What the strip's own sentence adds is the DIAGNOSIS. "You have no
              key" is a state; "issuance can fail during sign-in" is the reason a
              signed-in visitor might be looking at it, and it is the half that
              tells them pressing the button below is worth doing rather than a
              repeat of something that already failed. */}
          <NoticeStrip tone="error">
            <p data-testid="no-key-notice">
              No API key found for your account. This can happen if key issuance
              failed during sign-in.{' '}
              {/* The numeric Discord id, on the one card that ends in a
                  support request. It is the account key (ADR 0010) and task
                  0186's acceptance criterion puts it on screen; the metadata
                  row that carries it elsewhere is hidden on this card, so
                  without this sentence it was not even on hover here. */}
              If you contact us, quote your Discord ID{' '}
              <code data-testid="no-key-discord-id">{session.user_id}</code>.
            </p>
          </NoticeStrip>
          {/* A link, not a button: issuing is an OAuth round-trip (see
              `issueUrl`), and only a top-level navigation can carry it.

              `data-variant="cta"` widens it to the card, which is the one place
              the frame draws this control as a full-width bar. The chrome's
              rule for issue links otherwise pins them to `flex-start` on
              purpose — a stretched yellow row reads as a banner rather than a
              control — and that reasoning still holds everywhere it is NOT the
              only thing in the card. Here it is the only thing in the card, and
              a banner that is also the single action is just the action. */}
          <a href={issueUrl()} data-variant="cta">
            <GlyphBadge icon={AddRoundedIcon} tone="onPrimary" />
            Generate API Key
          </a>
        </>
      )}

      {view.state === 'ok' && (
        <>
          {/* The design's key row (`landing/DashboardPanel.tsx`) — the same
              component the landing page's preview draws, so the thing a
              visitor was shown before signing in is the thing they land on.
              Only the presentation moved: the mask, the toggle, the copy
              button and every word around them are tasks 0187's and 0189's.

              Masked by default. The mask is a fixed run of dots, not a
              prefix-and-suffix of the real value: showing the first and last
              few characters of a credential is a habit borrowed from card
              numbers, where the rest is high-entropy. Here it would leak part
              of the secret for no benefit anyone asked for. */}
          {/* ⚠️ **The first-login card masks too, since 2026-08-26 (Adam).**
              It briefly did not: this slice read the frame as drawing the
              credential in the clear and argued the friction was not worth it
              seconds after an OAuth round-trip. Re-reading the frame, the
              first-login box holds a run of dots and a "Show key" control —
              so 0187's rule has no exception after all, and the one place the
              mask was lifted is now the one place it was wrong. The card that
              delivers a credential is exactly the card most likely to be on a
              shared screen. */}
          {/* No label inside the box: the card's own header says "API Key" one
              line above it, and the frame draws the ring bare. */}
          <KeyField
            testId="api-key"
            value={
              revealed ? view.key.value : '••••••••••••••••••••••••••••••••'
            }
            // ⚠️ **One control, both cards, inside the ring** (Adam,
            // 2026-08-26). The ordinary card used to spell this "Reveal" and
            // put it in the row below, between "Copy key" and "Regenerate" —
            // so the two dashboards a visitor sees on consecutive days named
            // the same act differently and put it in different places. It
            // belongs beside the value it uncovers, and the first-login
            // frame's word wins.
            //
            // A bare `<button>`, so the chrome's rule draws it as the dark
            // pill the frame draws.
            inlineAction={
              <button
                type="button"
                data-testid="show-key"
                onClick={() => setRevealed((was) => !was)}
              >
                {revealed ? 'Hide key' : 'Show key'}
              </button>
            }
            actions={
              justIssued ? (
                <>
                  <button type="button" data-variant="primary" onClick={onCopy}>
                    <GlyphBadge
                      icon={ContentCopyRoundedIcon}
                      tone="onPrimary"
                    />
                    Copy key
                  </button>
                  {/* The frame's second control, and a link because it leaves
                      the page. Task 0163 has not landed, so `QUICKSTART` is
                      still the OpenAPI document — one constant, one diff when
                      it does. */}
                  <a href={QUICKSTART} data-variant="quiet">
                    View quick start
                    <ArrowBadge variant="onLight" />
                  </a>
                </>
              ) : (
                <>
                  {/* `data-variant` rather than a class: the descendant rule
                      in the dashboard's chrome styles every bare `<button>`
                      the same, and copying the key is the card's primary
                      action — the design gives it the brand fill. The
                      attribute changes no wording and no role. */}
                  <button type="button" data-variant="primary" onClick={onCopy}>
                    <GlyphBadge
                      icon={ContentCopyRoundedIcon}
                      tone="onPrimary"
                    />
                    Copy key
                  </button>
                  {/* The frame's second control, in the row the frame puts it
                      in. It opens the confirmation (`ReplaceKey`) — never the
                      round-trip, which is reached only from the armed confirm
                      inside it. */}
                  {/* ⚠️ **"Regenerate" is Adam's word, chosen on 2026-08-25
                      over task 0191's "Replace my key…"** — the frame's, and
                      the one the button now carries. On 2026-08-26 the dialog
                      followed it: its heading, its confirm button and its
                      typed phrase (`regenerate-key`) all say the same verb,
                      because a dialog that demanded `delete-key` under a
                      button marked "Regenerate" asked the visitor to agree to
                      a sentence they had not read. The BEHAVIOUR is unchanged
                      and is still not what the word implies: pressing this
                      deactivates the key and issues nothing until the period
                      rolls, and the dialog says so in task 0191's wording.
                      Where the frame's word and the build disagree, the
                      dialog is the one telling the truth. */}
                  {!replacing && (
                    <button
                      type="button"
                      onClick={() => setReplacing(true)}
                      data-testid="replace-key-open"
                    >
                      <GlyphBadge icon={AutorenewRoundedIcon} tone="onDark" />
                      Regenerate
                    </button>
                  )}
                </>
              )
            }
          />
          {/* `role="status"` so the confirmation reaches a screen reader:
              the button's own label does not change, so without this the copy
              succeeds silently for anyone not looking at the text. */}
          {copied && <p role="status">Copied.</p>}
          {/* The frame's row under the buttons. "Issued" is honest ONLY on
              this path: `GET /key` returns no timestamps (see `MetaField`),
              but `?issue=ok` means the backend created this key during the
              round-trip that just ended, so "just now" and today's date are
              facts this page holds rather than a value it invented. The quota
              column appears only once `/usage` has answered — see `onQuota`. */}
          {justIssued && (
            <MetaRow>
              {/* "Just now" is what `?issue=ok` proves — the backend created
                  this key during the round-trip that just ended — and the date
                  beside it is `createdDate`, off the same listing the reveal
                  already made. Neither is read from this machine's clock. */}
              <MetaField label="Issued">
                Just now
                {issuedOn && ` · ${issuedOn}`}
              </MetaField>
              {quota !== undefined && quota !== null && (
                <MetaField label="Monthly quota">
                  {quota.toLocaleString('en-US')} requests
                </MetaField>
              )}
              {rateLimit !== undefined && (
                <MetaField label="Rate limit">{rateLimit} req/s</MetaField>
              )}
            </MetaRow>
          )}
          {/* ⚠️ Task 0189's "Send it as the `X-API-Key` header on `/v1/`
              requests." was here and is gone (Adam, 2026-08-25). It is the
              only place the dashboard said WHERE to put the key; the Quick
              Start behind the navbar link is now the only place a visitor
              learns that. */}
          {/* Task 0191. Beside the key and nowhere else: there is nothing to
              replace without one. A button that opens the confirmation,
              not a link into the round-trip — the round-trip is reached only
              from the armed confirm inside the dialog. */}
          {replacing && (
            <ReplaceKey
              onClose={() => setReplacing(false)}
              onRevoked={(revoked) => {
                setReplacing(false);
                setRevealed(false);
                setView({ state: 'revoked', revoked });
                onRevoked?.();
              }}
            />
          )}
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

      {/* The design's metadata row, with the fields the backend actually
        returns. "Issued" and "Last rotated" are in the mock and are NOT here:
        `GET /key` carries an id, a name and the value and no timestamps at
        all, so both would have to be invented — see the note on `MetaField`.

        Outside the `ok` branch on purpose. The Discord account is a property
        of the SESSION, not of a key, and task 0186's acceptance criterion is
        that the username and the ID are on screen for a signed-in visitor —
        which has to hold on the day their key issuance failed too. The two
        key fields inside it come and go with the key. */}
      {/* Hidden on the first-login card, which has a metadata row of its own
          (Issued · Monthly quota · Rate limit) inside the `ok` branch above —
          the frame for that screen shows those three and not these four, and
          rendering both put "Issued" on the card twice. */}
      {/* ⚠️ **Also hidden on the empty card** (Adam, 2026-08-26): the
          `Dashboard - no key` frame gives it a strip and a button and nothing
          else, and the metadata row was putting a lone "Discord account"
          column under them.

          This costs task 0186's acceptance criterion on THIS screen — "the
          username and the ID are on screen for a signed-in visitor" — which
          the row was deliberately placed outside the `ok` branch to satisfy,
          precisely so it would hold on the day a visitor's key issuance
          failed. The navbar still names the account; the numeric id, which
          lived in this column's `title`, is now nowhere on the empty
          dashboard. Flagged rather than absorbed: if 0186's criterion is to
          survive, it needs somewhere else to live, and that is a change to
          0186 rather than to this frame. */}
      <MetaRow hidden={justIssued || emptyKeyCard}>
        {view.state === 'ok' && (
          <MetaField label="Key ID">
            <code>{view.key.key_id}</code>
          </MetaField>
        )}
        {/* The frame's "Issued" and "Last rotated", now that `GET /key`
            carries both instants. Two departures from it, both deliberate:

            - **"Last updated", not the frame's "Last rotated".** The frame's
              label was taken on 2026-08-25 and reversed on 2026-08-27, on PR
              #249's review: the value is AWS's `lastUpdatedDate`, which the
              0191 audit measured a no-op patch and a console `description`
              edit both bumping — so on a key nobody has touched it is the
              issue date, and after a revocation it is the revocation. It is
              never a rotation, because this build has none (0191: "Nothing
              is rotated"), and both ends of the contract — `keys/mod.rs` and
              `api/portal.ts` — name the field for that reason and say the
              dashboard labels it "Last updated". A label the value cannot
              stand behind is the frame's to fix, not this page's to keep.
            - **Key name is gone.** It was not in the frame, and it is
              `discord-<userId>-key` — the Discord id in the column beside it,
              with a prefix. A column that restates its neighbour is what made
              this row five wide where the design has four. */}
        {view.state === 'ok' && issuedOn && (
          <MetaField label="Issued">{issuedOn}</MetaField>
        )}
        {view.state === 'ok' && updatedOn && (
          <MetaField label="Last updated">{updatedOn}</MetaField>
        )}

        {/* The handle alone, as the frame draws it — Adam, 2026-08-25.
            ⚠️ The numeric Discord id was here because it is task 0186's
            acceptance criterion ("the username and the ID are on screen") and
            because it, not the handle, is the account key (ADR 0010): a
            visitor can rename themselves, the id never changes. It is kept as
            the column's `title`, so it is still one hover away for a support
            thread, but it is no longer on screen — 0186's criterion is not
            met by this build any more. */}
        <MetaField label="Discord account">
          <Stack direction="row" spacing={1} alignItems="center">
            <Box
              aria-hidden
              sx={{
                flexShrink: 0,
                width: 22,
                height: 22,
                borderRadius: '6px',
                display: 'grid',
                placeItems: 'center',
                backgroundColor: '#5865f2',
                color: color.white,
              }}
            >
              <DiscordIcon sx={{ fontSize: 14 }} />
            </Box>
            <Box component="span" title={session.user_id}>
              {session.username}
            </Box>
          </Stack>
        </MetaField>
      </MetaRow>

      {/* The frame's yellow strip, under the metadata row it belongs to.
          The frame's words (Adam, 2026-08-25) were kept whole until
          2026-08-27:

              Key rotation is limited to once per calendar month.
              Next rotation available: <the 1st of next month>

          ⚠️ They describe the swap model task 0191 built and then reversed,
          and PR #249's review round found the same sentence pair on the
          landing page and sent both back to 0191's model; this strip is the
          third place and was found while driving the states in a browser.
          Nothing is rotated: pressing Regenerate deactivates the key and
          issues nothing, and a new key comes only with the next period — so
          the strip now says that, in 0191's terms (regenerate, quota period),
          and the dialog behind the button agrees with it instead of
          contradicting it. The date is the same instant either way.

          The date prefers `/usage`'s `resets_at`, which is the backend's own
          `Period`; without it the page computes the 1st of next month itself,
          because that IS the rule (`portal/period.rs`) rather than a number
          the server holds. */}
      {view.state === 'ok' && !justIssued && (
        <NoticeStrip>
          <p>
            Regenerating is limited to <strong>once per quota period</strong>{' '}
            and issues nothing now. A new key can be issued from:{' '}
            <strong>
              {describeUtcDay(resetsAt) ?? describeNextPeriodStart()}
            </strong>
          </p>
        </NoticeStrip>
      )}
    </DashboardCard>
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
/**
 * The quota-reached notice — task 0193, from the `Dashboard - limits` frame.
 *
 * The card above it already says *how much* is gone (100,000 of 100,000, a red
 * bar, "0 remaining"). None of that says what a developer at the ceiling
 * actually needs to know, which is what their integration is doing RIGHT NOW:
 * every request is being refused with `429` until the reset. A spent bar is a
 * fact about the past; this is the consequence, and it is the reason the state
 * gets its own strip instead of just a colour change.
 *
 * **New copy, and it belongs to this slice.** The brief's rule is that 0193
 * re-decides no wording another task owns, and this wording is not owned: the
 * quota-reached state is not in [[0188]]'s panel (which stops at three figures
 * and a reset rule) nor in [[0191]]'s revoke flow. The date and the status code
 * are both restatements of things already decided elsewhere — the reset rule is
 * 0188's, `HTTP 429` on throttle is the Rate Limit card's constant — and
 * neither is re-worded here, only repeated where it now bites.
 *
 * "Contact us" is plain text under [`Legal`]'s rule, the same as the Rate Limit
 * card's identical offer two columns away: there is still no commercial-plans
 * destination, and this sentence is read by somebody who has just been cut off
 * — the worst possible moment to hand out a link that 404s.
 */
function QuotaReachedNotice({ resetsAt }: { resetsAt: string }) {
  const at = new Date(resetsAt);
  const on = Number.isNaN(at.getTime())
    ? null
    : at.toLocaleDateString('en-GB', {
        day: 'numeric',
        month: 'long',
        timeZone: 'UTC',
      });

  return (
    <NoticeStrip tone="error">
      <p data-testid="usage-quota-reached">
        Monthly quota reached. API requests will return{' '}
        <strong>HTTP 429</strong>
        {/* The date is dropped rather than guessed if `resets_at` is
            unparseable: the sentence still says what is happening and when it
            stops being true is simply left unsaid, which beats "resets on
            Invalid Date" on the one card a cut-off developer is reading. */}
        {on ? <> until the quota resets on {on}</> : ' until the quota resets'}.{' '}
        {/* `<strong>`, so it takes the strip's emphasis colour (#e7000b) from
            the one place that decides it, rather than naming the hex a second
            time here — the same treatment `HTTP 429` gets, because the frame
            gives the two the same weight. Still underlined and still a
            `<span>` inside it and not an `<a>`: there is no commercial-plans
            destination, which is [`Legal`]'s rule. */}
        <strong>
          <Box component="span" sx={{ ...UNDERLINED, color: 'inherit' }}>
            Contact us for higher limits.
          </Box>
        </strong>
      </p>
    </NoticeStrip>
  );
}

function Usage({
  keyAbsent = false,
  keyOnScreen,
  revokedCount = 0,
  rateLimit,
  onUsage,
}: {
  /**
   * The account has no key at all — the frame gives this card an empty body.
   * See the dashboard's `keyAbsent`, which explains why a blank tile is the
   * right answer for this ONE state and for no other.
   */
  keyAbsent?: boolean;
  keyOnScreen: boolean;
  /** Task 0191: bumped by the dashboard on each in-page revoke. */
  revokedCount?: number;
  rateLimit?: number;
  /**
   * What this panel read, handed to the dashboard so the key card above can
   * state the quota and the date the next key becomes available without a
   * second `GetUsage` (task 0194 owns that budget). `null` where AWS has
   * recorded nothing yet, which is what keeps those fields absent rather than
   * zero or a guessed date.
   */
  onUsage?: (usage: PortalUsage | null) => void;
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
        // Up to the dashboard — see `onUsage`. `null` for a key AWS has no
        // row for yet, which is a different thing from zero.
        onUsage?.(usage ?? null);
      })
      .catch((error: unknown) => {
        // Through `describeFailure`, like the key section beside this one: an
        // expired session must read "sign out and sign in again" in BOTH
        // places, not as that sentence in one and a raw "answered 401" in the
        // other — two wordings for one cause on one screen reads as two bugs.
        if (live) setView({ state: 'failed', reason: describeFailure(error) });
      });
    // ⚠️ `onUsage` IS an inline arrow at the call site, so it is a new
    // function on every render of `Dashboard` — and naming it here would
    // re-create `load`, which the mount effect below depends on, and refetch
    // usage in a loop. It is left out on purpose: `load` reads nothing from
    // it that can go stale, because the callback only forwards this fetch's
    // own answer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // On mount only.
  useEffect(() => {
    load();
    return () => cancelInFlight.current?.();
  }, [load]);

  // Task 0191: after an in-page revoke, re-ask. The backend evicted its
  // cache for this caller, so this is one real read — and the answer may
  // legitimately still carry the revoked key's figures: `usage::fetch` keeps
  // reporting a revoked key's counter (it is preserved) for as long as the cap
  // holds, so the `ok` branch below renders an ordinary usage panel. Nothing in
  // THIS section marks the key dead; the key section above it renders the
  // `key-revoked` view, and that is where the visitor reads it. Only the
  // `no-key` branch here has revoke wording, for the case where AWS has no row
  // at all.
  const lastRevokedCount = useRef(0);
  useEffect(() => {
    if (revokedCount === lastRevokedCount.current) return;
    lastRevokedCount.current = revokedCount;
    load();
  }, [revokedCount, load]);

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
   * The reset caption, under the bar and left-aligned — "Resets 1 September".
   *
   * ⚠️ **This replaces two lines task 0188 decided**, at Adam's instruction on
   * 2026-08-25: the "Quota resets on the 1st of each month, 00:00 UTC — next
   * reset …" sentence and the "Last updated … AWS reports usage with a delay"
   * line, along with the Refresh button beside them. The frame has none of the
   * three, and the space they occupied is where task [[0232]]'s daily chart
   * goes.
   *
   * What is lost with them is 0188's lag disclosure — the panel no longer says
   * that a figure can trail reality by minutes. The date below still carries
   * the reset rule, which is the half the frame keeps; the lag is the half
   * that now goes unsaid.
   */
  const resetCaption = (resetsAt?: string) => {
    if (!resetsAt) return null;
    const at = new Date(resetsAt);
    if (Number.isNaN(at.getTime())) return null;
    // Day and month, no year — "Resets 1 September", the frame's "Resets May
    // 1" in the day-first order every other date on this page uses. The year
    // is never in doubt: the next reset is at most a month away.
    return `Resets ${at.toLocaleDateString('en-GB', {
      day: 'numeric',
      month: 'long',
      timeZone: 'UTC',
    })}`;
  };

  // The empty tile from the `Dashboard - no key` frame. Rendered BEFORE the
  // fetch's own view is consulted, so it does not matter whether `/usage` has
  // answered `no_key` yet or is still in flight — the key card has already
  // settled the question, and the two cannot disagree on screen.
  //
  // Still a titled card, not nothing: the frame keeps the header band, and a
  // dashboard that drops a panel entirely when you have no key would rearrange
  // itself the moment you got one.
  if (keyAbsent) return <DashboardCard title="Monthly Usage" />;

  return (
    <DashboardCard title="Monthly Usage">
      {view.state === 'loading' && <p>Loading your usage…</p>}

      {view.state === 'no-key' &&
        (keyOnScreen ? (
          // The endpoint said "no key" while a key is on this very screen:
          // the backend's short cache has not caught up with the issue yet.
          // "You have no key" would be false, so say what is actually
          // happening.
          <p>
            Your key is new — usage figures appear here with a delay after your
            first requests.
          </p>
        ) : revokedCount > 0 ? (
          // Task 0191: revoked in this page load, and AWS has no row for
          // the key. Not "issue one above" — the key section is already
          // saying when that becomes possible.
          <p data-testid="usage-after-revoke">
            Your key was deactivated; there is no usage to show for it.
          </p>
        ) : (
          <p>You have no API key yet — issue one above to see your usage.</p>
        ))}

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
          </>
        ) : (
          <>
            {/* The bar is task 0193's; the figures under it are task 0188's and
                are untouched — same labels, same `data-testid`s, same raw
                values. The meter adds the one thing three separate numbers do
                not give: the ratio, at a glance. The design puts it on the
                landing page's dashboard preview, and this is the same
                component, so the preview cannot promise a bar the real screen
                does not draw. */}
            {/* NO `resetLabel` here, unlike the landing page's preview. Task
                0188's `limits()` below already states the reset rule and the
                next date, and states it more precisely than a meter caption
                can ("the 1st of each month, 00:00 UTC" — our rule, not an AWS
                guarantee). Repeating the date two lines apart is the panel
                saying the same thing twice and inviting the two to drift. */}
            {/* Task 0188's three figures, in the design's arrangement. The
                numbers, their `data-testid`s and the raw values its tests read
                are unchanged — what went is the "Used: / Remaining: / Monthly
                limit:" labelling around them, which 0188 wrote when this panel
                was three unstyled paragraphs and which its own brief asked for
                "as numbers, not prose". Of its two STATEMENTS, the reset rule
                survives as the caption under the bar, and the lag line is the
                paragraph below the meter. */}
            <UsageMeter
              headline
              used={view.usage.used}
              limit={view.usage.limit}
              remaining={view.usage.remaining}
              resetLabel={resetCaption(view.usage.resets_at)}
            />
            {/* ⚠️ Task 0188's lag line, **back since 2026-08-27**. It was cut
                on 2026-08-25 with the reset-rule sentence and the Refresh
                button ("tutaj będą wykresy" — task 0232's chart takes the
                space), and PR #249's review sent it back: 0188 decided this
                wording once and says this slice "restyles it but does not
                re-decide it", `as_of` was still served and no longer read,
                and a developer who has just made calls and sees an unchanged
                number had nothing on screen explaining why. Rendered small,
                under the meter — the restyle — with 0188's sentence verbatim.
                `toUTCString` spells the zone "GMT"; the decided wording says
                UTC, so the suffix is corrected rather than the line frozen. */}
            <Typography
              component="p"
              variant="caption"
              data-testid="usage-as-of"
              sx={{ color: color.text.tertiary, m: 0 }}
            >
              Last updated{' '}
              {new Date(view.usage.as_of).toUTCString().replace(/GMT$/, 'UTC')}{' '}
              — AWS reports usage with a delay, so requests made in the last few
              minutes may not be counted yet.
            </Typography>
            {/* Only at the ceiling, and `>=` rather than `===`: AWS can count
                a burst PAST the quota (the gateway refuses on its own clock,
                not on ours), and a card that goes quiet again at 100,001 would
                be dropping the notice exactly when it is most true. */}
            {view.usage.limit !== null &&
              view.usage.limit > 0 &&
              view.usage.used >= view.usage.limit && (
                <QuotaReachedNotice resetsAt={view.usage.resets_at} />
              )}
          </>
        ))}

      {view.state === 'failed' && (
        <p>
          Could not load your usage: <code>{view.reason}</code>
        </p>
      )}
    </DashboardCard>
  );
}

/**
 * The Rate Limit card — the plan's two figures and what the gateway does when
 * you cross them.
 *
 * The per-second figure comes from `/config` (task 0188 put it there so it
 * cannot drift from what API Gateway actually enforces); the per-minute one is
 * that number times sixty, computed rather than written down for the same
 * reason. Both HTTP codes are properties of the gateway, not of a plan, so they
 * are constants.
 *
 * **The card always renders**, which is a change from the build that dropped
 * it whenever `/config` carried no limit (Adam, 2026-08-25: "brakuje całego
 * jednego kafelka"). A local run without `PORTAL_RATE_LIMIT` set was losing a
 * third of the dashboard, and a missing panel is a worse answer than a stated
 * one: the free plan's rate is 1 req/s (task 0157), the landing page says so
 * to every visitor before they sign in, and this card now says the same where
 * the deployment has not spoken.
 *
 * ⚠️ `/config` still WINS wherever it answers, which is every deployed
 * environment (`compute-stack.ts` passes `pricingApiFreePlanRateLimit`
 * unconditionally). The fallback is for the local case only, and if the free
 * plan's rate ever changes, this constant is one of the two places that must
 * change with it — the other being `FairAccess`.
 */
const FREE_PLAN_RATE_LIMIT = 1;

function RateLimitCard({
  rateLimit,
  keyAbsent = false,
}: {
  rateLimit?: number;
  /**
   * The account has no key — the frame empties this card too. See the
   * dashboard's `keyAbsent`.
   *
   * ⚠️ This card's figures do NOT depend on having a key: the free plan's rate
   * is the free plan's rate, and every other state renders them. Emptying it
   * here is the frame's call and it is defensible — a limit stated for a
   * credential that does not exist is a number with nothing to apply to, and
   * the frame wants nothing competing with the one button on the card above.
   * **The green "Active" pill goes with it**, and that part is not merely
   * defensible but required: "Active" is a claim about a key, and there is no
   * key to make it about.
   */
  keyAbsent?: boolean;
}) {
  const perSecond = rateLimit ?? FREE_PLAN_RATE_LIMIT;

  if (keyAbsent) return <DashboardCard title="Rate Limit" />;

  return (
    <DashboardCard title="Rate Limit" status={{ label: 'Active', tone: 'ok' }}>
      <Stack direction="row" spacing={4}>
        <Figure
          label="Per-second limit"
          value={perSecond}
          unit="req/s"
          testId="rate-limit"
        />
        <Figure
          label="Per-minute limit"
          value={perSecond * 60}
          unit="req/min"
        />
      </Stack>
      {/* Task 0188's sentence, kept verbatim and read only by assistive
          technology. The design abbreviates the figure to "1 req/s", which is
          right for the eye and wrong for a screen reader — "req" is not a word.
          Expanding an abbreviation is what this technique is for, and it lets
          0188's wording survive a change that was purely visual. */}
      <Box component="p" sx={visuallyHidden}>
        Rate limit: {perSecond} request{perSecond === 1 ? '' : 's'} per second.
      </Box>
      <Stack spacing={1} sx={{ pt: 1, borderTop: cardBorder }}>
        <ResponseRow label="Response on throttle" code="HTTP 429" />
        <ResponseRow label="Response on missing key" code="HTTP 403" />
      </Stack>
      {/* "Contact us" is plain text, not a link: there is no commercial-plans
          destination to point it at, and a link to a 404 beside the words
          "commercial plans" is worse than none. */}
      <Typography variant="body2" sx={{ color: color.text.secondary }}>
        Need higher limits? Contact us for commercial plans.
      </Typography>
    </DashboardCard>
  );
}

/** One big yellow number with its unit, from the Rate Limit card. */
function Figure({
  label,
  value,
  unit,
  testId,
}: {
  label: string;
  value: number;
  unit: string;
  testId?: string;
}) {
  return (
    <Stack spacing={0.5}>
      <Typography variant="body2" sx={{ color: color.text.tertiary }}>
        {label}
      </Typography>
      <Stack direction="row" spacing={1} alignItems="baseline">
        <Typography
          component="span"
          sx={{
            fontFamily: font.primary,
            fontWeight: 700,
            // 36px: measured 27px of cap height on the frame's "60", where
            // 2rem gives 23.
            fontSize: '2.25rem',
            lineHeight: 1,
            color: color.text.accent,
          }}
          data-testid={testId}
        >
          {value}
        </Typography>
        <Typography variant="body2" sx={{ color: color.text.secondary }}>
          {unit}
        </Typography>
      </Stack>
    </Stack>
  );
}

function ResponseRow({ label, code }: { label: string; code: string }) {
  return (
    <Stack direction="row" justifyContent="space-between" spacing={2}>
      {/* Tertiary, measured #a3a3a3 — the label names the status beside it and
          the status is the part with the colour. */}
      <Typography variant="body2" sx={{ color: color.text.tertiary }}>
        {label}
      </Typography>
      <Typography
        component="code"
        sx={{
          fontFamily: font.mono,
          fontSize: '0.875rem',
          fontWeight: 700,
          color: color.text.error,
        }}
      >
        {code}
      </Typography>
    </Stack>
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

/**
 * The portal panel — everything that talks to the backend, on one anchor.
 *
 * The `/config` probe used to live HERE and is now a prop. It moved up to
 * `LandingPage` (task 0193) because the answer decides something outside this
 * panel: whether the hero and the navbar render a "Get API Key" control at all.
 * While the portal is closed those controls would promise a key the backend
 * will not issue — the same objection task 0186 raised against putting a
 * sign-in button on a closed page — and the alternative to lifting it is a
 * second `/config` fetch per page load to answer a question one already
 * answered.
 */
/**
 * The wrapper that styles the plain markup tasks 0186 to 0192 own.
 *
 * Shared by the landing's status panel and the dashboard route, because both
 * render that markup and neither may rewrite it — see the long note inside.
 */
function PortalStatusChrome({ children }: { children: ReactNode }) {
  return (
    <Box
      sx={{
        width: '100%',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'stretch',
        gap: 3,

        // ---------------------------------------------------------------
        // Descendant styling, NOT a component rewrite.
        //
        // Everything below this heading is tasks 0186 to 0192's markup —
        // plain `<p>`, `<a>`, `<button>`, `<code>` — and every sentence in it
        // is copy those tasks decided. Task 0193's rule is that it re-decides
        // none of it, so this slice styles the ELEMENTS and leaves the tree
        // alone: no wording moves, no control changes role, no `data-testid`
        // is touched, and the 84 tests that pin that copy keep passing
        // unaltered.
        //
        // The alternative — rewriting each branch into MUI components — would
        // have meant retyping ~40 pieces of decided copy by hand, which is
        // exactly the way the reason behind a wording gets lost.
        //
        // It is deliberately scoped to this panel rather than put in
        // `CssBaseline`: a global bare-`<a>` rule would also repaint the
        // landing sections above, where every link is already a themed
        // component.
        // ---------------------------------------------------------------
        '& p': {
          ...theme.typography.body1,
          color: color.text.secondary,
          margin: 0,
        },
        '& a': {
          color: color.text.accent,
          textUnderlineOffset: '0.2em',
        },
        // ⚠️ `& p code`, NOT `& code`. The grey chip belongs to code quoted
        // inside a sentence — `X-API-Key`, `/v1/`, a failure reason — and to
        // nothing else. As `& code` it also painted the two places the frame
        // draws bare: the key itself, inside its yellow ring, and the Key ID
        // in the metadata row. Both tried to opt out with their own
        // `backgroundColor` and neither could: `.chrome code` is one class
        // plus one type, which outranks the single class Emotion puts on the
        // element's own `sx`. Narrowing the rule is the fix; raising the
        // other side's specificity would have been a fight with no end.
        '& p code': {
          fontFamily: font.mono,
          fontSize: '0.875em',
          backgroundColor: color.surface.gray,
          borderRadius: '4px',
          px: 0.75,
          py: 0.25,
          overflowWrap: 'anywhere',
        },
        // Everything else that is `<code>` keeps the face and nothing else.
        '& code': { fontFamily: font.mono, overflowWrap: 'anywhere' },
        // The first-login card's second control. A link, so it keeps `& a`'s
        // semantics, but the frame draws it as a button-height row of white
        // text with the circled arrow rather than as accent-coloured prose.
        '& a[data-variant="quiet"]': {
          display: 'inline-flex',
          alignItems: 'center',
          gap: 1,
          minHeight: 44,
          ...theme.typography.body2,
          fontWeight: 700,
          color: color.text.primary,
          textDecoration: 'none',
        },
        '& button[data-variant="primary"]': {
          backgroundColor: `${color.surface.primary} !important`,
          borderColor: 'transparent !important',
          color: `${color.black} !important`,
          '&:hover': { backgroundColor: `${color.primary[300]} !important` },
        },
        // ⚠️ **Plain `<button>` only — never a MUI one** (Adam, 2026-08-26).
        //
        // This rule is `.chrome button`: one class plus one type, which
        // outranks the single class Emotion puts on a component's own `sx`. It
        // was written for the bare markup tasks 0186–0192 left in this panel,
        // but it reached every MUI `Button` rendered inside the chrome too and
        // overrode what they asked for. The revoked card's "Sign out" is the
        // one that showed it: the frame draws a bare label and it came out as
        // a dark grey pill, because a `text` Button sets no background of its
        // own and had nothing to win the cascade with. Its yellow sibling
        // escaped only because `.MuiButton-contained` is two classes.
        //
        // Narrowed rather than fought with `!important` on the other side —
        // the same fix, and the same reasoning, as `& p code` above.
        '& button:not([class*="MuiButton-root"])': {
          ...theme.typography.body2,
          fontWeight: 700,
          cursor: 'pointer',
          display: 'inline-flex',
          alignItems: 'center',
          gap: 1,
          // 36 to match the frame, with the touch-target floor restored where
          // a finger is doing the pressing — the same trade the navbar's one
          // control makes, and for the same reason: 44 everywhere made the
          // dashboard's buttons visibly taller than the mock.
          minHeight: 36,
          '@media (pointer: coarse)': { minHeight: 44 },
          paddingInline: '16px',
          borderRadius: `${radius.pill}px`,
          // Measured: the frame's secondary button is a fill one step DARKER
          // than the card body it sits on, with no border at all. The old
          // `surface.gray` + hairline is now the colour of the body itself.
          border: 'none',
          backgroundColor: color.surface.grayAlt,
          color: color.text.primary,
          alignSelf: 'flex-start',
          '&:hover': { backgroundColor: color.black },
        },
        '& hr': {
          width: '100%',
          border: 0,
          borderTop: cardBorder,
          margin: 0,
        },
        // The link that starts the issue round-trip gets the brand button,
        // because it is the primary action of whichever state renders it — a
        // page whose whole purpose is "get a key" should not present that as
        // the same underlined text as a Discord invite three lines up.
        //
        // Matched on `href` rather than on a class or a wrapper: the URL comes
        // from `issueUrl()` in `api/portal.ts`, so the selector tracks the
        // route and not a styling hook somebody could remove without noticing.
        // It stays an `<a>` — an OAuth flow is a top-level navigation and
        // `fetch` cannot perform one, which is the reason task 0189 made it a
        // link in the first place.
        //
        // `signInUrl()` is deliberately NOT in this selector any more: since
        // the login card arrived, sign-in renders as a real MUI `Button` in
        // Discord's blurple, and two competing rules for one control is how a
        // button ends up yellow on one state and blurple on the next.
        '& a[href*="action=issue"]': {
          ...theme.typography.body2,
          fontWeight: 700,
          display: 'inline-flex',
          alignItems: 'center',
          minHeight: 44,
          // px strings, NOT bare numbers: inside `sx`, MUI runs
          // spacing-aware properties (`padding*`, `margin*`, `gap`) through
          // `theme.spacing`, so `paddingInline: 20` is 160px, not 20 — which
          // is what turned this button into a full-width yellow bar.
          paddingInline: '20px',
          borderRadius: `${radius.pill}px`,
          backgroundColor: color.surface.primary,
          color: color.black,
          textDecoration: 'none',
          // The panel is a column flex container, so without this the button
          // stretches to the full 720 px measure — a full-width yellow bar
          // that reads as a banner, not a control.
          alignSelf: 'flex-start',
          '&:hover': { backgroundColor: color.primary[300] },
          // The `+` badge the empty-key card puts inside this link. The rule
          // had no icon to space before, so it set no gap at all.
          gap: 1.5,
          // The empty-key card's bar, full width (Adam, 2026-08-26).
          // `stretch` rather than `width: 100%`: the panel is a column flex
          // container, so the cross-axis is what decides this, and a
          // percentage width would still be pinned left by `alignSelf` above.
          //
          // The rule above pins every OTHER issue link to `flex-start`, on the
          // reasoning that a stretched yellow row reads as a banner rather
          // than a control. That still holds where the link sits among other
          // things. Here it is the only thing in the card, so there is nothing
          // for it to be mistaken for.
          '&[data-variant="cta"]': {
            alignSelf: 'stretch',
            justifyContent: 'center',
            // Taller than the inline form (44) — measured off the frame, where
            // this bar is the card's whole body and carries the same weight as
            // the key's ringed box on the card it replaces.
            minHeight: 56,
          },
        },
      }}
    >
      {children}
    </Box>
  );
}

/**
 * The section's accessible name and its place in the document outline.
 *
 * Hidden rather than deleted: the Figma frames show no heading above the card,
 * because the card's own title already says where the visitor is — but a
 * section with no name is a landmark a screen-reader user cannot tell from any
 * other. Task 0185's wording, unchanged.
 */
function LoginHeading({ level }: { level: 'h1' | 'h2' }) {
  return (
    <Typography
      variant="h2"
      component={level}
      id={`${LOGIN_ANCHOR}-heading`}
      sx={visuallyHidden}
    >
      Stellar Prices API — API keys
    </Typography>
  );
}

function PortalStatus({ probe }: { probe: Probe }) {
  // Once the portal is open this band renders NOTHING. Its only job is the
  // answer a visitor gets when there is nothing to click — still asking, shut,
  // or unreachable.
  //
  // Task 0185's `Reached /api/config successfully — same-origin, no
  // API key, no CORS` line used to live here and is **deliberately gone**: it
  // was that slice's evidence that the bundle could reach its own backend,
  // written when this page had nothing else to show for it. The page now has
  // plenty — the sign-in control only appears because `/config` answered, and
  // task 0193's own brief is that this must not look like a debug harness. The
  // property it proved is covered by the tests that assert the relative URL and
  // by the gate below; a diagnostic sentence under a marketing page was the
  // last thing on it that read as scaffolding.
  const hasCard = probe.state !== 'ok' || !probe.config.enabled;
  if (!hasCard) return null;

  return (
    <LoginSection testId="portal-status" labelledBy={`${LOGIN_ANCHOR}-heading`}>
      {/* Level 2: the hero above owns this page's `h1`. */}
      <LoginHeading level="h2" />

      <PortalStatusChrome>
        {probe.state === 'loading' && (
          <LoginCard title={LOGIN_TITLE} subtitle={LOGIN_SUBTITLE}>
            <Callout variant="neutral" icon={<CircularProgress size={18} />}>
              <p>Checking whether the portal is open…</p>
            </Callout>
          </LoginCard>
        )}

        {/* The closed state is the one that ships. Task 0194 flips the flag,
          after task 0189's eligibility gate passes — so for now this is what
          every visitor sees, and it must say so plainly and offer nothing to
          click. No sign-in control: the backend answers those routes with an
          empty 404 while the flag is off, so a button here would be a button
          that cannot work.

          It is also the state task 0193 was told to STYLE rather than treat as
          an afterthought, because it may be what a visitor sees for weeks —
          so it gets the design's card, not a bare sentence. The card carries
          no Discord button, which is the same rule the hero follows. */}
        {probe.state === 'ok' && !probe.config.enabled && <ClosedPortalCard />}

        {/* No sign-in card here any more. Once the portal is open, signing in
            is the `/login` route's job and the landing's only part in it is the
            "Get API Key" control in the hero and the navbar. What stays on this
            panel is the three answers a visitor gets when there is nothing to
            click: still asking, shut, or unreachable. */}

        {/* A failure here is not cosmetic: it means the bundle could not reach
          its own backend, which is either the behaviour ordering in task 0184's
          routing table or the gate in task 0183. Show the reason rather than a
          spinner that never resolves.

          The error skin, not the neutral one — and the distinction is the same
          one task 0193 insists on between "could not verify" and "not a
          member": a visitor must be able to tell at a glance whether they are
          looking at something broken on our side or a rule about them. */}
        {probe.state === 'failed' && (
          <LoginCard title={LOGIN_TITLE} subtitle={LOGIN_SUBTITLE}>
            <Callout variant="error">
              <p>
                Could not reach the portal backend: <code>{probe.reason}</code>
              </p>
            </Callout>
          </LoginCard>
        )}
      </PortalStatusChrome>
    </LoginSection>
  );
}

/**
 * The landing page (task 0193) — the marketing frame from Figma, with the live
 * portal panel embedded in it as one more section.
 *
 * **Embedded, not a second route.** Task 0195 has not landed the per-prefix SPA
 * fallback yet, so a hard refresh on `/api/anything` resolves against S3
 * — which grants `s3:GetObject` and not `s3:ListBucket`, so a missing key comes
 * back as `403 AccessDenied` rather than a 404 the router could handle. Until
 * then this app has exactly one URL, and "go to the dashboard" is a scroll.
 *
 * The order is the Figma frame's order, and the portal sits after the sections
 * that explain what a key is for. A visitor who arrives already knowing has the
 * navbar's CTA, which jumps straight past all of it.
 */
/**
 * The landing page (task 0193) — the marketing frame from Figma.
 *
 * Signing in is no longer part of it: that moved to `/login` when the portal
 * grew a second page. What is left here is the status panel, which says the
 * one useful thing when there is nothing to click — still asking, shut, or
 * unreachable — and carries task 0185's same-origin evidence.
 */
function LandingPage({
  probe,
  canOfferKey,
}: {
  probe: Probe;
  canOfferKey: boolean;
}) {
  return (
    <>
      <Navbar canOfferKey={canOfferKey} />
      <Box component="main">
        {/* One section, one screen: the hero and the trust band together
            fill the viewport under the navbar. See `HeroSection`. */}
        <HeroSection canOfferKey={canOfferKey} />
        <Features />
        <UseCases />
        <Endpoints />
        <SelfService />
        <DeveloperDashboard />
        <FairAccess />
        <Documentation />
        <Faq />
        <FinalCta canOfferKey={canOfferKey} />
        <PortalStatus probe={probe} />
      </Box>
      <Footer canOfferKey={canOfferKey} />
    </>
  );
}

/** The `/config` probe, asked once for the whole app. */
function useConfigProbe(): Probe {
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

  return probe;
}

/**
 * The sentence for a session request that failed — one place, because the
 * login card and the dashboard both render it and the two used to disagree
 * (the dashboard rendered nothing: it redirected to the landing page).
 */
function SessionFailedCallout({
  session,
}: {
  session: Extract<SessionState, { state: 'failed' }>;
}) {
  return (
    <Callout variant="error">
      {session.while === 'signing-out' ? (
        <p data-testid="sign-out-failed">
          Could not sign you out: <code>{session.reason}</code>. You are still
          signed in on this device — try again, or close the browser.
        </p>
      ) : (
        <p data-testid="session-check-failed">
          Could not check your sign-in status: <code>{session.reason}</code>
        </p>
      )}
    </Callout>
  );
}

/**
 * What the three routes share: the portal's flag, the session, and the two
 * questions every route answers differently.
 */
type Gate = {
  probe: Probe;
  /** The portal is confirmed open. Not "not confirmed shut" — see `canOfferKey`. */
  open: boolean;
  session: SessionState;
  /** The session lookup has an answer, whatever it is. */
  settled: boolean;
  authenticated: boolean;
  onSignOut: () => void;
  /**
   * Re-ask `/auth/me`. The sign-in popup needs it: the session cookie appears
   * without this window ever navigating, so nothing else would tell the app
   * that the visitor is now signed in.
   */
  reloadSession: () => void;
};

/**
 * `/` — the landing page, and the junction the OAuth callback lands on.
 *
 * `portal/auth/mod.rs` redirects to `/api/` in **every** outcome and
 * says why: "when the portal grows a second page, the page it lands on decides
 * where to go next; this handler still will not." This is that page, and these
 * are the two decisions it makes.
 *
 * Both forwards carry `location.search` verbatim. The callback's `?issue=…`
 * and `?signin=…` are one-shot landing states owned by tasks 0189 and 0186,
 * read and stripped by whichever view renders them — dropping the query here
 * would silently delete a refusal the visitor is owed an explanation for.
 *
 * The redirects are `replace`, so Back from the dashboard goes to wherever the
 * visitor was before signing in rather than to a `/` that bounces them
 * straight forward again.
 */
function RootRoute({ gate }: { gate: Gate }) {
  const location = useLocation();
  const canOfferKey = gate.open;

  if (gate.open && gate.settled) {
    if (gate.authenticated) {
      return <Navigate to={`/dashboard${location.search}`} replace />;
    }
    // A signed-out visitor stays on the landing page — UNLESS they have just
    // come back from Discord, in which case the banner explaining what
    // happened belongs beside the button they will press again.
    //
    // ⚠️ `issue` counts as coming back from Discord too, and used not to: an
    // issue round-trip whose session expired mid-flight landed here with
    // `?issue=…` still on the URL, matched neither branch, and rendered the
    // marketing page — the one journey where the visitor is owed an answer
    // ends on the page that answers nothing. `/login` reads `signin` and the
    // dashboard reads `issue`, so a landing that carries only `issue` needs
    // the sign-in card first anyway; forwarding it keeps the query alive
    // through that hop instead of dropping the refusal on the floor.
    const landed = new URLSearchParams(location.search);
    if (landed.has('signin') || landed.has('issue')) {
      return <Navigate to={`/login${location.search}`} replace />;
    }
  }

  return <LandingPage probe={gate.probe} canOfferKey={canOfferKey} />;
}

/**
 * Task 0183's "the portal is not open yet" card.
 *
 * One component for what was the same markup in three places: the landing
 * page's login panel, the `/login` route, and — since 2026-08-27 — the
 * `?signin=not_open` landing, which is a deployment that has credentials but
 * cannot ask the eligibility question at all. That last one used to arrive as
 * `?signin=unknown` and render 0189's "we could not check your membership"
 * card: a transient Discord fault, claimed for a permanent state, with a retry
 * that could not succeed until an operator wired the parameters.
 *
 * The wording is 0183's and is not re-decided here.
 */
function ClosedPortalCard({ titleComponent }: { titleComponent?: 'h1' }) {
  return (
    <LoginCard
      title={LOGIN_TITLE}
      titleComponent={titleComponent}
      subtitle={LOGIN_SUBTITLE}
    >
      <Callout variant="neutral">
        <p data-testid="portal-closed">
          This is where you will sign in and issue an API key. It is not yet
          available.
        </p>
      </Callout>
    </LoginCard>
  );
}

/**
 * `/login` — the Figma login frame, on its own, with nothing else on the page.
 *
 * A visitor who is already signed in is sent to the dashboard rather than
 * shown a sign-in button that would start a round-trip they do not need.
 */
function LoginRoute({ gate }: { gate: Gate }) {
  const location = useLocation();
  if (gate.probe.state === 'loading') {
    return (
      <LoginSection back full>
        <LoginCard
          title={LOGIN_TITLE}
          titleComponent="h1"
          subtitle={LOGIN_SUBTITLE}
        >
          <Callout variant="neutral" icon={<CircularProgress size={18} />}>
            <p>Checking whether the portal is open…</p>
          </Callout>
        </LoginCard>
      </LoginSection>
    );
  }

  // Shut or unreachable. NOT a redirect to `/`: somebody who followed a link or
  // a bookmark to this URL is owed the reason, and bouncing them to a marketing
  // page they have to re-read is not one.
  if (!gate.open) {
    return (
      <LoginSection back full>
        {gate.probe.state === 'failed' ? (
          <LoginCard
            title={LOGIN_TITLE}
            titleComponent="h1"
            subtitle={LOGIN_SUBTITLE}
          >
            <Callout variant="error">
              <p>
                Could not reach the portal backend:{' '}
                <code>{gate.probe.reason}</code>
              </p>
            </Callout>
          </LoginCard>
        ) : (
          <ClosedPortalCard titleComponent="h1" />
        )}
      </LoginSection>
    );
  }

  if (gate.authenticated) {
    // The query rides along, like `RootRoute`'s forward: a sign-in that
    // issued a key lands its popup on `?issue=ok`, `LoginPanel` copies that
    // onto this URL, and the dashboard's first-login card is what reads it.
    return <Navigate to={`/dashboard${location.search}`} replace />;
  }

  return (
    <LoginSection back full>
      <LoginView session={gate.session} onSignedIn={gate.reloadSession} />
    </LoginSection>
  );
}

/**
 * `/dashboard` — the key and the usage panel.
 *
 * A visitor who is not signed in goes to `/api/`, per the brief. It
 * waits for `settled` first: redirecting while `/auth/me` is still in flight
 * would bounce every arrival from the OAuth callback straight back to the
 * landing page, which is the one journey this route exists to complete.
 */
function DashboardRoute({ gate }: { gate: Gate }) {
  if (gate.probe.state === 'loading' || (gate.open && !gate.settled)) {
    return (
      <LoginSection full>
        <LoginCard
          title={LOGIN_TITLE}
          titleComponent="h1"
          subtitle={LOGIN_SUBTITLE}
        >
          <Callout variant="neutral" icon={<CircularProgress size={18} />}>
            <p>Checking whether you are signed in…</p>
          </Callout>
        </LoginCard>
      </LoginSection>
    );
  }

  // ⚠️ "Not authenticated" and "could not find out" are different answers,
  // and only the first earns the redirect. A `/auth/me` that answered `502`
  // — or a sign-out that did — used to bounce a visitor with a key on screen
  // to the marketing page, whose "Get API Key" buttons then addressed them as
  // a stranger. The reason rendered only on `/login`, a page nobody on the
  // dashboard is sent to. "Backend unavailable" is one of the states this
  // task owes a screen; this is that screen for this route.
  if (gate.open && gate.session.state === 'failed') {
    const failed = gate.session;
    const signingOut = failed.while === 'signing-out';
    return (
      <LoginSection back full>
        <LoginCard
          title={LOGIN_TITLE}
          titleComponent="h1"
          subtitle={LOGIN_SUBTITLE}
        >
          <SessionFailedCallout session={failed} />
          <Button
            type="button"
            variant="contained"
            onClick={signingOut ? gate.onSignOut : gate.reloadSession}
          >
            {signingOut ? 'Try signing out again' : 'Try again'}
          </Button>
          {/* Signing in is a fresh top-level navigation, so it does not depend
              on the request that just failed — same reasoning as `LoginView`.
              Not offered after a failed sign-out: the visitor IS signed in. */}
          {!signingOut && (
            <DiscordButton href={signInUrl()}>
              Sign in with Discord
            </DiscordButton>
          )}
        </LoginCard>
      </LoginSection>
    );
  }

  if (!gate.open || !gate.authenticated) {
    return <Navigate to="/" replace />;
  }

  const rateLimit =
    gate.probe.state === 'ok'
      ? gate.probe.config.rate_limit_per_second
      : undefined;

  const session = (gate.session as { state: 'ok'; session: PortalSession })
    .session;

  return (
    <>
      {/* Its own bar, not the landing page's: this visitor has a key, so there
          is nothing here to offer them one. Sign-out lives in it, which is why
          `Dashboard` no longer takes an `onSignOut`. */}
      <DashboardNavbar username={session.username} onSignOut={gate.onSignOut} />
      <Box
        component="main"
        sx={{
          position: 'relative',
          overflow: 'hidden',
          // #212121, measured off the frame — the SAME floor the hero stands
          // on, not the darker `backgroundAlt` this used to take. The cards
          // are what step away from it (a #1a1a1a title band over a #272727
          // body); the page itself is the light one, which is why a card on
          // the old floor read as lighter than its surroundings and on the
          // real one reads as raised.
          backgroundColor: color.surface.background,
          // Fill the viewport even when the cards are short, so the footer sits
          // at the bottom of the screen rather than halfway up it.
          minHeight: 'calc(100dvh - 52px)',
          py: { xs: 4, md: 7 },
        }}
      >
        {/* The hero's rule grid, on the dashboard too — 80px, and measured at
            #323232 on the floor, which is `Stroke/Default` at a third. It is
            strongest behind the cards and gone by the foot of the page, so the
            mask is a radial centred on the upper third rather than the hero's
            wider one. Decorative, and never in the way of a click. */}
        <Box
          aria-hidden
          sx={{
            position: 'absolute',
            inset: 0,
            pointerEvents: 'none',
            backgroundImage: `
              linear-gradient(${alpha(color.stroke.default, 0.34)} 1px, transparent 1px),
              linear-gradient(90deg, ${alpha(color.stroke.default, 0.34)} 1px, transparent 1px)`,
            backgroundSize: '80px 80px, 80px 80px',
            maskImage:
              'radial-gradient(120% 70% at 50% 20%, #000 25%, transparent 80%)',
          }}
        />
        <Container sx={{ position: 'relative' }}>
          <PortalStatusChrome>
            <Dashboard
              session={session}
              rateLimit={rateLimit}
              onSignOut={gate.onSignOut}
            />
          </PortalStatusChrome>
        </Container>
      </Box>
      <Footer canOfferKey={false} />
    </>
  );
}

/**
 * `/quick-start` — the guide, reachable from the signed-in bar and from the
 * landing page's docs cards alike. It needs no session: it is documentation,
 * and a visitor deciding whether to sign in is exactly who should read it.
 * The bar it gets depends on who is looking — the signed-in bar (with "Quick
 * start" underlined) for somebody with a key, the landing bar otherwise.
 */
function QuickStartRoute({ gate }: { gate: Gate }) {
  const rateLimit =
    gate.probe.state === 'ok'
      ? gate.probe.config.rate_limit_per_second
      : undefined;
  const signedIn = gate.open && gate.authenticated;
  const username = signedIn
    ? (gate.session as { state: 'ok'; session: PortalSession }).session.username
    : undefined;

  return (
    <>
      {signedIn ? (
        <DashboardNavbar
          username={username}
          onSignOut={gate.onSignOut}
          current="quick-start"
        />
      ) : (
        // `inPage={false}`: this is the quick start, and the bar's three
        // links name sections of the LANDING page.
        <Navbar canOfferKey={gate.open} inPage={false} />
      )}
      <QuickStart rateLimit={rateLimit} />
      <Footer canOfferKey={signedIn} />
    </>
  );
}

/**
 * The API reference, as a chunk of its own (task 0195). Its renderer and the
 * OpenAPI parser it needs stay out of the bundle a first visit pays for — the
 * landing page links to `/docs` but most visitors never open it. It is a small
 * chunk; the split is for the landing page's sake, not the reference's.
 */
const ApiReference = lazy(() => import('../docs/ApiReference'));

/**
 * `/docs` — the API reference, the live OpenAPI document rendered in the
 * portal's own pieces (task 0195). Documentation like the quick start: no
 * session needed, and the bar depends on who is looking. The document is
 * fetched by the page itself; a spinner covers the chunk on the rare first
 * visit that starts here.
 */
function DocsRoute({ gate }: { gate: Gate }) {
  const signedIn = gate.open && gate.authenticated;
  const username = signedIn
    ? (gate.session as { state: 'ok'; session: PortalSession }).session.username
    : undefined;

  return (
    <>
      {signedIn ? (
        <DashboardNavbar
          username={username}
          onSignOut={gate.onSignOut}
          current="docs"
        />
      ) : (
        <Navbar canOfferKey={gate.open} inPage={false} />
      )}
      <Suspense
        fallback={
          <Stack alignItems="center" sx={{ py: 12 }} aria-busy="true">
            <CircularProgress
              size={28}
              aria-label="Loading the API reference"
            />
          </Stack>
        }
      >
        <ApiReference />
      </Suspense>
      <Footer canOfferKey={signedIn} />
    </>
  );
}

export function App() {
  const probe = useConfigProbe();
  const open = probe.state === 'ok' && probe.config.enabled;
  const { session, onSignOut, reload } = useSession(open);

  const gate: Gate = {
    probe,
    open,
    session,
    settled: session.state !== 'loading',
    authenticated: session.state === 'ok' && session.session.authenticated,
    onSignOut,
    reloadSession: reload,
  };

  return (
    <Routes>
      <Route path="/" element={<RootRoute gate={gate} />} />
      <Route path="/login" element={<LoginRoute gate={gate} />} />
      <Route path="/dashboard" element={<DashboardRoute gate={gate} />} />
      <Route path="/quick-start" element={<QuickStartRoute gate={gate} />} />
      <Route path="/docs" element={<DocsRoute gate={gate} />} />
      {/* Anything else is a URL this app never minted. On the shared host
          every extensionless path under `/api/` boots this bundle (the
          explorer's routing function rewrites it to `/api/index.html`), so
          a mistyped deep link arrives here rather than at S3 — and goes to
          `/` rather than to a 404 page the app does not have. */}
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

export default App;
