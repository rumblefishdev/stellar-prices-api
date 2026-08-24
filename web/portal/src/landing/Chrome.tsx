import Box from '@mui/material/Box';
import Button from '@mui/material/Button';
import Container from '@mui/material/Container';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import { Link as RouterLink } from 'react-router-dom';

import { color, font } from '../theme/tokens';
import { DASHBOARD_ROUTE, LOGIN_ROUTE, SWAGGER_UI } from './links';
import { ArrowBadge, cardBorder } from './primitives';

/**
 * The navbar and the footer.
 *
 * The labels are the design's, read off the exported frame. Two of the
 * footer's six point outside this repo — `Status` and `Contact` name
 * destinations that do not exist yet, so they are rendered as text until
 * somebody supplies a URL: a footer link to a 404 is worse than one that is
 * plainly not wired.
 */

/**
 * The SorobanScan lockup, and the Rumble Fish mark in the footer.
 *
 * Real SVGs, exported from the file with `download_assets` — they replace the
 * rasters an earlier pass recovered from a screenshot when the Figma seat had
 * no tool calls left. The lockup is two nodes in the design (an 18.4 × 18.8
 * circle mark and an 85.4 × 23.7 wordmark, 24px apart) and it stays two here:
 * the mark alone is what a narrow viewport gets.
 */
import rumblefishLogo from '../assets/rumblefish-logo.svg';
import sorobanScanIcon from '../assets/sorobanscan-icon.svg';
import sorobanScanWordmark from '../assets/sorobanscan-wordmark.svg';

/** In-page destinations, in the order the sections appear. */
const NAV = [
  { label: 'Features', href: '#features' },
  { label: 'Docs', href: SWAGGER_UI },
  { label: 'FAQ', href: '#faq' },
] as const;

export function Wordmark() {
  return (
    <Stack direction="row" spacing={0.75} alignItems="center">
      <Box
        component="img"
        src={sorobanScanIcon}
        // Decorative: the wordmark beside it carries the name, and announcing
        // the mark as well would say "SorobanScan" twice.
        alt=""
        aria-hidden
        sx={{ height: 19, width: 'auto', display: 'block' }}
      />
      <Box
        component="img"
        src={sorobanScanWordmark}
        // The product's name, so it is the `alt` text — not "logo", which tells
        // a screen-reader user the shape of the thing rather than what it says.
        alt="SorobanScan"
        sx={{ height: 24, width: 'auto', display: 'block' }}
      />
    </Stack>
  );
}

/** The footer's mark. Same provenance as the header's. */
export function RumbleFishMark({ height = 32 }: { height?: number }) {
  return (
    <Box
      component="img"
      src={rumblefishLogo}
      alt="Rumble Fish — software development"
      sx={{ height, width: 'auto', display: 'block' }}
    />
  );
}

export function Navbar({ canOfferKey }: { canOfferKey: boolean }) {
  return (
    <Box
      component="nav"
      aria-label="Primary"
      sx={{
        position: 'sticky',
        top: 0,
        // Above the hero's backdrop and the sticky Use Cases heading, below
        // nothing else — the page has no modals of its own.
        zIndex: 10,
        backgroundColor: alpha(color.surface.background, 0.85),
        // `backdrop-filter` is progressive: where it is unsupported the 85%
        // background alone is still opaque enough to read against.
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
          <Stack direction="row" alignItems="center" spacing={{ xs: 1, sm: 2 }}>
            {NAV.map(({ label, href }) => (
              <Link
                key={label}
                href={href}
                // Muted until hovered: three secondary links beside a yellow
                // CTA should not compete with it, which is what the design
                // does by giving them no colour of their own.
                sx={{
                  display: { xs: 'none', sm: 'inline' },
                  color: color.text.secondary,
                  fontFamily: font.secondary,
                  fontSize: '0.875rem',
                  fontWeight: 500,
                  '&:hover': { color: color.text.primary },
                }}
              >
                {label}
              </Link>
            ))}
            {/* Same rule as the hero's: no offer until the probe says the
                portal is open. See `LandingPage`. */}
            {canOfferKey && (
              <Button
                variant="contained"
                color="primary"
                // A router `Link` to `/login`, not a fragment: the login screen
                // is its own route now, and an in-app navigation keeps the
                // `/config` answer and the session lookup this app has already
                // paid for.
                component={RouterLink}
                to={LOGIN_ROUTE}
                endIcon={<ArrowBadge variant="onPrimary" />}
                sx={{
                  // The design's header button is 36 high with 12px of side
                  // padding — the theme's 44/20 default is a touch-target
                  // floor, and applying it here made the navbar's one control
                  // noticeably taller and wider than the mock.
                  minHeight: 36,
                  paddingInline: '12px',
                  fontSize: '0.9375rem',
                  '& .MuiButton-endIcon': { marginLeft: '8px' },
                  // The floor comes back where it actually matters. A finger
                  // has no business hitting a 36px target; a mouse does.
                  '@media (pointer: coarse)': { minHeight: 44 },
                }}
              >
                Get API Key
              </Button>
            )}
          </Stack>
        </Stack>
      </Container>
    </Box>
  );
}

export function Footer({ canOfferKey }: { canOfferKey: boolean }) {
  const links: { label: string; href?: string }[] = [
    { label: 'Documentation', href: SWAGGER_UI },
    // Only where there is a dashboard to reach. While the portal is shut this
    // link would land on `/dashboard`, which sends a visitor with no session
    // straight back to the page they clicked from.
    ...(canOfferKey ? [{ label: 'Dashboard', href: DASHBOARD_ROUTE }] : []),
    { label: 'Status' },
    { label: 'Contact' },
    { label: 'rumblefish.dev', href: 'https://rumblefish.dev' },
    { label: 'Privacy policy' },
  ];

  return (
    <Box
      component="footer"
      sx={{
        // #0f0f0f, measured. The footer sits on the darkest floor, not the
        // section grey — which is what separates it from the closing call to
        // action fading up into it.
        backgroundColor: color.surface.backgroundAlt,
        borderTop: cardBorder,
        py: 4,
      }}
    >
      <Container>
        <Stack
          direction={{ xs: 'column', md: 'row' }}
          spacing={2}
          alignItems={{ xs: 'flex-start', md: 'center' }}
          justifyContent="space-between"
        >
          <RumbleFishMark />
          <Stack
            direction="row"
            spacing={2}
            useFlexGap
            sx={{ flexWrap: 'wrap' }}
            component="nav"
            aria-label="Footer"
          >
            {links.map(({ label, href }) =>
              href ? (
                <Link
                  key={label}
                  href={href}
                  sx={{
                    color: color.text.secondary,
                    fontFamily: font.secondary,
                    fontSize: '0.875rem',
                    fontWeight: 500,
                    '&:hover': { color: color.text.primary },
                  }}
                >
                  {label}
                </Link>
              ) : (
                <Typography
                  key={label}
                  component="span"
                  sx={{
                    color: color.text.tertiary,
                    fontFamily: font.secondary,
                    fontSize: '0.875rem',
                    fontWeight: 500,
                  }}
                >
                  {label}
                </Typography>
              ),
            )}
          </Stack>
          <Typography variant="body2" sx={{ color: color.text.tertiary }}>
            © 2026 Rumble Fish. All rights reserved.
          </Typography>
        </Stack>
      </Container>
    </Box>
  );
}
