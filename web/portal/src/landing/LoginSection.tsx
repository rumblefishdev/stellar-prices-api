import ChevronLeftRoundedIcon from '@mui/icons-material/ChevronLeftRounded';
import Box from '@mui/material/Box';
import Container from '@mui/material/Container';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import { alpha } from '@mui/material/styles';
import type { ReactNode } from 'react';
import { Link as RouterLink } from 'react-router-dom';

import { color } from '../theme/tokens';
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
            radial-gradient(50% 60% at 50% 35%, ${alpha(color.primary[400], 0.06)} 0%, transparent 70%),
            linear-gradient(${alpha(color.stroke.default, 0.12)} 1px, transparent 1px),
            linear-gradient(90deg, ${alpha(color.stroke.default, 0.12)} 1px, transparent 1px)`,
          backgroundSize: '100% 100%, 80px 80px, 80px 80px',
          maskImage:
            'radial-gradient(100% 100% at 50% 40%, #000 25%, transparent 78%)',
        }}
      />

      <Container sx={{ position: 'relative', width: '100%' }}>
        <Stack spacing={3} alignItems="center">
          {/* Top-left in the design. A router `Link`, not an `<a href>`: this
              is an in-app navigation and a full document load here would throw
              away the `/config` answer and the session lookup the app has
              already made. */}
          {back && (
            <Box sx={{ alignSelf: 'flex-start' }}>
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

          {children}
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
export const visuallyHidden = {
  position: 'absolute',
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: 'hidden',
  clip: 'rect(0 0 0 0)',
  whiteSpace: 'nowrap',
  border: 0,
} as const;
