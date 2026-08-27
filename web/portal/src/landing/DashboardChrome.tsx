import LogoutRoundedIcon from '@mui/icons-material/LogoutRounded';
import Box from '@mui/material/Box';
import Container from '@mui/material/Container';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import { Link as RouterLink } from 'react-router-dom';

import { color, font, radius } from '../theme/tokens';
import { DiscordIcon } from './DiscordIcon';
import { Wordmark } from './Chrome';
import { DASHBOARD_ROUTE, QUICKSTART_ROUTE, SWAGGER_UI } from './links';
import { cardBorder } from './primitives';

/**
 * The signed-in navbar (Figma `Dashboard` frame, `852:1499`).
 *
 * A different bar from the landing page's, not a variant of it: the landing
 * sells the API to somebody who has no key, and this one belongs to somebody
 * who does. It carries where they are (Dashboard, underlined), the two places
 * they go next, who they are signed in as, and the way out. There is no "Get
 * API Key" here, because they have one.
 */
export function DashboardNavbar({
  username,
  onSignOut,
  current = 'dashboard',
}: {
  username?: string;
  onSignOut: () => void;
  /** Which of the bar's own pages this is — the one that gets the underline. */
  current?: 'dashboard' | 'quick-start';
}) {
  const links = [
    {
      label: 'Dashboard',
      to: DASHBOARD_ROUTE,
      current: current === 'dashboard',
    },
    {
      label: 'Quick start',
      to: QUICKSTART_ROUTE,
      current: current === 'quick-start',
    },
    { label: 'OpenAPI Docs', href: SWAGGER_UI },
  ];

  return (
    <Box
      component="nav"
      aria-label="Dashboard"
      sx={{
        position: 'sticky',
        top: 0,
        zIndex: 10,
        backgroundColor: alpha(color.surface.backgroundAlt, 0.85),
        backdropFilter: 'blur(12px)',
        borderBottom: cardBorder,
      }}
    >
      <Container>
        <Stack
          direction="row"
          alignItems="center"
          justifyContent="space-between"
          sx={{ minHeight: 52, gap: 2 }}
        >
          <Wordmark />

          <Stack
            direction="row"
            alignItems="center"
            spacing={{ xs: 1.5, md: 3 }}
            useFlexGap
            sx={{ minWidth: 0, flexWrap: { xs: 'wrap', sm: 'nowrap' } }}
          >
            {links.map(({ label, to, href, current }) => (
              <Link
                key={label}
                {...(to ? { component: RouterLink, to } : { href })}
                // `aria-current`, not just an underline: "you are here" has to
                // reach a screen reader too, and a border-bottom does not.
                aria-current={current ? 'page' : undefined}
                sx={{
                  // ⚠️ Every non-current link used to be `display: none` at
                  // `xs`, which on a phone left the bar showing only where the
                  // visitor already was: no way back to the dashboard from the
                  // quick start, and "OpenAPI Docs" reachable from neither
                  // page. Three short labels fit — they wrap onto a second row
                  // rather than disappearing, and the row is what the 375px
                  // criterion asks for.
                  display: 'inline',
                  whiteSpace: 'nowrap',
                  fontFamily: font.secondary,
                  fontSize: '0.9375rem',
                  fontWeight: 500,
                  textDecoration: 'none',
                  color: current ? color.text.primary : color.text.tertiary,
                  // The frame's active tab: a BRAND-yellow rule, not a white
                  // one, and it runs past the word at both ends — 8px of side
                  // padding is what makes it longer than the label without
                  // moving the label itself off the design's baseline.
                  borderBottom: current
                    ? `2px solid ${color.primary[400]}`
                    : '2px solid transparent',
                  px: current ? 1 : 0,
                  pb: '4px',
                  '&:hover': { color: color.text.primary },
                }}
              >
                {label}
              </Link>
            ))}

            {/* The frame's vertical rule between "OpenAPI Docs" and the
                account. Decorative, so `aria-hidden`: what it separates is
                already two different things to a screen reader (a nav list and
                the signed-in identity), and announcing a pipe between them
                would be noise. */}
            {username && (
              <Box
                aria-hidden
                sx={{
                  display: { xs: 'none', sm: 'block' },
                  width: '1px',
                  height: 24,
                  flexShrink: 0,
                  backgroundColor: alpha(color.stroke.default, 0.6),
                }}
              />
            )}

            {username && (
              <Stack
                direction="row"
                spacing={1}
                alignItems="center"
                sx={{ minWidth: 0 }}
              >
                <Box
                  aria-hidden
                  sx={{
                    flexShrink: 0,
                    width: 26,
                    height: 26,
                    borderRadius: '8px',
                    display: 'grid',
                    placeItems: 'center',
                    backgroundColor: '#5865f2',
                    color: color.white,
                  }}
                >
                  <DiscordIcon sx={{ fontSize: 16 }} />
                </Box>
                <Typography
                  sx={{
                    fontFamily: font.secondary,
                    fontWeight: 700,
                    fontSize: '0.9375rem',
                    color: color.text.primary,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {username}
                </Typography>
              </Stack>
            )}

            {/* A real `<button>`: signing out is a POST to `/auth/logout`
                (task 0186), not a navigation, and the element has to say so. */}
            <Stack
              component="button"
              type="button"
              onClick={onSignOut}
              direction="row"
              spacing={1}
              alignItems="center"
              sx={{
                flexShrink: 0,
                background: 'none',
                border: 'none',
                cursor: 'pointer',
                p: 0,
                fontFamily: font.secondary,
                fontSize: '0.9375rem',
                fontWeight: 500,
                color: color.text.tertiary,
                '&:hover': { color: color.text.primary },
              }}
            >
              <Box
                component="span"
                sx={{ display: { xs: 'none', sm: 'inline' } }}
              >
                Sign out
              </Box>
              <Box
                aria-hidden
                sx={{
                  width: 24,
                  height: 24,
                  borderRadius: `${radius.pill}px`,
                  display: 'grid',
                  placeItems: 'center',
                  backgroundColor: color.primary[400],
                  color: color.black,
                }}
              >
                <LogoutRoundedIcon sx={{ fontSize: 14 }} />
              </Box>
            </Stack>
          </Stack>
        </Stack>
      </Container>
    </Box>
  );
}
