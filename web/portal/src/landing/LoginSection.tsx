import ChevronLeftRoundedIcon from '@mui/icons-material/ChevronLeftRounded';
import Box from '@mui/material/Box';
import Container from '@mui/material/Container';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import { alpha } from '@mui/material/styles';
import {
  createContext,
  useCallback,
  useContext,
  useLayoutEffect,
  useState,
  type ReactNode,
} from 'react';
import { Link as RouterLink } from 'react-router-dom';

import { color } from '../theme/tokens';
import { LOGIN_CARD_MAX_WIDTH } from './LoginCard';
import { LOGIN_ANCHOR } from './links';

/**
 * The section the "Get API Key" controls scroll to — the Figma login frame,
 * rendered in place rather than on a route of its own.
 *
 * **In place, deliberately.** Task 0195 has not landed the per-prefix SPA
 * fallback, so a hard refresh on `/api-tokens/login` resolves against S3, which
 * grants `s3:GetObject` and not `s3:ListBucket` — a missing key comes back as
 * `403 AccessDenied`, which the router never sees and cannot handle. Every
 * redirect in the OAuth flow also lands back on `/api-tokens/`, so a login
 * route would be a URL the flow returns *from* and never *to*. A section keeps
 * refresh, Back and the callback all working today; when 0195 lands, this is
 * the component a route would render.
 *
 * The backdrop repeats the hero's grid and glow, at the same 80 px pitch. That
 * is what makes the scroll read as arriving somewhere rather than as the page
 * running out — the two screens the reviewer looks at are visibly the same
 * product.
 */
/**
 * How a card tells the section "I am drawing the back link myself".
 *
 * ⚠️ Added 2026-08-26 (Adam): the OAuth error card puts "Back to landing"
 * INSIDE it, under "Try again with Discord", while every other login state
 * keeps it above the card. Both cannot render one — two links with the same
 * name and the same target on one page is the bug this whole arrangement
 * exists to avoid — and the state that decides lives three components down,
 * in `LoginPanel`, where the `?signin=…` outcome is held.
 *
 * A context rather than a prop threaded through `LoginRoute` → `LoginView` →
 * `LoginPanel`: those two middle components exist only to place the panel and
 * would be carrying a value neither of them reads.
 */
const BackLinkClaim = createContext<((owned: boolean) => void) | null>(null);

/**
 * Claim the section's back link for this card, for as long as `owned` holds.
 *
 * `useLayoutEffect`, not `useEffect`: the claim has to land before the browser
 * paints, or the section's own link is visible for a frame and the page
 * flickers a second one in and out.
 *
 * Call it UNCONDITIONALLY with a boolean — it is a hook, and a card that
 * claims on some renders and not others would break the rules of hooks.
 */
export function useOwnBackLink(owned: boolean) {
  const claim = useContext(BackLinkClaim);
  useLayoutEffect(() => {
    claim?.(owned);
    // Released on unmount as well as on `owned` going false, so a card that
    // disappears does not leave the section thinking somebody still has it.
    return () => claim?.(false);
  }, [claim, owned]);
}

export function LoginSection({
  children,
  testId,
  /**
   * `back` is only for the standalone `/login` route. On the landing page the
   * same section is the portal-status panel, and a "Back to landing" link on
   * the landing page is a link to the page you are already reading.
   */
  back = false,
  labelledBy,
  /**
   * `full` makes the section fill the viewport, which the `/login` route wants
   * and the landing's status panel does not — a full-height panel in the middle
   * of a scrolling page is a hole, not a screen.
   */
  full = false,
  compact = false,
}: {
  children: ReactNode;
  testId?: string;
  back?: boolean;
  full?: boolean;
  /**
   * The id of the heading that names this section, when one is hidden inside
   * it. Omitted on `/login`, where the card's own visible `h1` is the heading
   * and a second, hidden one would be a duplicate.
   */
  labelledBy?: string;
  /** Drop the section's vertical rhythm to a caption's worth. */
  compact?: boolean;
}) {
  // Whether a card inside this section is drawing the back link itself. See
  // `useOwnBackLink`.
  const [cardOwnsBackLink, setCardOwnsBackLink] = useState(false);
  const claim = useCallback((owned: boolean) => setCardOwnsBackLink(owned), []);

  return (
    <Box
      component="section"
      id={LOGIN_ANCHOR}
      data-testid={testId}
      aria-labelledby={labelledBy}
      sx={{
        position: 'relative',
        overflow: 'hidden',
        backgroundColor: color.surface.background,
        py: compact ? 3 : { xs: 6, md: 10 },
        scrollMarginTop: 52,
        ...(full && {
          minHeight: '100dvh',
          display: 'flex',
          alignItems: 'center',
        }),
      }}
    >
      <Box
        aria-hidden
        sx={{
          position: 'absolute',
          inset: 0,
          pointerEvents: 'none',
          backgroundImage: `
            radial-gradient(50% 60% at 50% 35%, ${alpha(color.primary[400], 0.085)} 0%, transparent 70%),
            linear-gradient(${alpha(color.stroke.default, 0.12)} 1px, transparent 1px),
            linear-gradient(90deg, ${alpha(color.stroke.default, 0.12)} 1px, transparent 1px)`,
          backgroundSize: '100% 100%, 80px 80px, 80px 80px',
          maskImage:
            'radial-gradient(100% 100% at 50% 40%, #000 25%, transparent 78%)',
        }}
      />

      <Container sx={{ position: 'relative', width: '100%' }}>
        <Stack spacing={3} alignItems="center">
          {/* Top-left in the design, and ABOVE the card — where it briefly
              was not: on 2026-08-26 it moved into the foot of the card and
              then straight back here, at Adam's instruction, because the frame
              puts it here on every login state. The card renders none of its
              own; this is the only one on the page.

              A router `Link`, not an `<a href>`: this is an in-app navigation
              and a full document load here would throw away the `/config`
              answer and the session lookup the app has already made. */}
          {back && !cardOwnsBackLink && (
            // ⚠️ **Above the card, aligned to its left edge — not beside it**
            // (Adam, 2026-08-26). `alignSelf: 'flex-start'` put the link
            // against the CONTAINER's edge, which at 1280 px leaves it
            // stranded hundreds of pixels away from the 464 px card it
            // belongs to and reads as page furniture rather than as this
            // screen's way back. Matching the card's own column puts it
            // where every frame draws it.
            <Box
              sx={{
                width: '100%',
                maxWidth: LOGIN_CARD_MAX_WIDTH,
                alignSelf: 'center',
              }}
            >
              <Link
                component={RouterLink}
                to="/"
                sx={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: 0.5,
                  color: color.text.tertiary,
                  textDecoration: 'none',
                  '&:hover': { color: color.text.primary },
                }}
              >
                <ChevronLeftRoundedIcon sx={{ fontSize: 20 }} />
                Back to landing
              </Link>
            </Box>
          )}

          <BackLinkClaim.Provider value={claim}>
            {children}
          </BackLinkClaim.Provider>
        </Stack>
      </Container>
    </Box>
  );
}

/**
 * Present to assistive technology, absent to the eye.
 *
 * The section needs an accessible name and the document outline needs its
 * level-2 heading; the design puts neither on screen, because the card's own
 * title already says where the visitor is. Hiding it with `clip` rather than
 * `display: none` is the difference between "not shown" and "not announced".
 */
/*
 * NOTE the `px` strings. Inside MUI's `sx`, a bare `width: 1` means **100%**,
 * not one pixel — the system treats unitless values at or below 1 as fractions
 * for width and height. Written as numbers, this "hidden" element was 100% x
 * 100%: `clip` still hid it, but it pushed the document 900px wider than the
 * viewport and gave the dashboard a horizontal scrollbar.
 */
export const visuallyHidden = {
  position: 'absolute',
  width: '1px',
  height: '1px',
  padding: 0,
  margin: '-1px',
  overflow: 'hidden',
  clip: 'rect(0 0 0 0)',
  whiteSpace: 'nowrap',
  border: 0,
} as const;
