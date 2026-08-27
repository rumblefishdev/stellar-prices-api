import CloseRoundedIcon from '@mui/icons-material/CloseRounded';
import MenuRoundedIcon from '@mui/icons-material/MenuRounded';
import Box from '@mui/material/Box';
import Button from '@mui/material/Button';
import Container from '@mui/material/Container';
import Drawer from '@mui/material/Drawer';
import IconButton from '@mui/material/IconButton';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import { useState } from 'react';
import { Link as RouterLink } from 'react-router-dom';

import { color, font } from '../theme/tokens';
import { DASHBOARD_ROUTE, LANDING, LOGIN_ROUTE, SWAGGER_UI } from './links';
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
  // In-page, not the OpenAPI document: "Quick Start" names the four-step
  // section that gets a visitor from nothing to a key, and sending it out to
  // a JSON file would be a link that answers a different question.
  { label: 'Quick Start', href: '#get-started' },
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

export function Navbar({
  canOfferKey,
  /**
   * Whether the in-page sections the links name are on THIS page.
   *
   * ⚠️ The quick start renders this same bar for a signed-out visitor, and its
   * sections are `prerequisites`…`next` — none of `#features`, `#get-started`
   * or `#faq` exists there, so all three links did nothing at all when
   * clicked. Off the landing page they become links back to it, at the same
   * anchors, which is where those sections actually are.
   */
  inPage = true,
}: {
  canOfferKey: boolean;
  inPage?: boolean;
}) {
  const navHref = (href: string) => (inPage ? href : `${LANDING}${href}`);
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
            {NAV.map(({ label, href: anchor }) => (
              <Link
                key={label}
                href={navHref(anchor)}
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
                  // The mobile frame's bar is the wordmark and a menu button,
                  // nothing else; the call to action moves into the menu.
                  display: { xs: 'none', sm: 'inline-flex' },
                }}
              >
                Get API Key
              </Button>
            )}
            <MobileMenu canOfferKey={canOfferKey} inPage={inPage} />
          </Stack>
        </Stack>
      </Container>
    </Box>
  );
}

/**
 * The phone's navigation: a menu button in the bar, and a panel that drops
 * from the top with the three in-page links and the call to action.
 *
 * Below `sm` the bar hides its links (there is no room for three beside the
 * wordmark) and, without this, hid the only way to reach a section other
 * than scrolling. The mobile frame draws the closed state — wordmark left,
 * ≡ right — and not the open one; the panel is the plainest reading of it:
 * the same bar with the button turned into ✕, and the links stacked under
 * it at a size a thumb can hit. A top drawer rather than a side one so the
 * open menu is visibly the bar that was tapped, unfolded.
 *
 * Closed on every link: an in-page anchor scrolls the page behind the panel
 * and would otherwise leave the visitor looking at a menu over the section
 * they asked for.
 */
function MobileMenu({
  canOfferKey,
  inPage,
}: {
  canOfferKey: boolean;
  /** Same meaning as `Navbar`'s — the drawer holds the same three links. */
  inPage: boolean;
}) {
  const navHref = (href: string) => (inPage ? href : `${LANDING}${href}`);
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);

  return (
    <>
      <IconButton
        aria-label="Open menu"
        aria-expanded={open}
        aria-controls="mobile-menu"
        onClick={() => setOpen(true)}
        sx={{
          display: { xs: 'inline-flex', sm: 'none' },
          color: color.text.primary,
          // Flush with the bar's right edge, like the frame — the button's
          // own padding would otherwise indent the icon by 8px.
          mr: -1,
        }}
      >
        <MenuRoundedIcon />
      </IconButton>

      <Drawer
        id="mobile-menu"
        anchor="top"
        open={open}
        onClose={close}
        slotProps={{
          paper: {
            sx: {
              // The theme's Paper is a card: bordered on every side and
              // rounded. This is a panel hanging off the top edge.
              backgroundColor: color.surface.backgroundAlt,
              backgroundImage: 'none',
              border: 'none',
              borderBottom: cardBorder,
              borderRadius: 0,
            },
          },
        }}
      >
        <Container>
          <Stack
            direction="row"
            alignItems="center"
            justifyContent="space-between"
            sx={{ minHeight: 52 }}
          >
            <Wordmark />
            <IconButton
              aria-label="Close menu"
              onClick={close}
              sx={{ color: color.text.primary, mr: -1 }}
            >
              <CloseRoundedIcon />
            </IconButton>
          </Stack>
          <Stack
            component="nav"
            aria-label="Menu"
            spacing={0.5}
            sx={{ pt: 1, pb: 3 }}
          >
            {NAV.map(({ label, href: anchor }) => (
              <Link
                key={label}
                href={navHref(anchor)}
                onClick={close}
                sx={{
                  py: 1.5,
                  color: color.text.primary,
                  fontFamily: font.secondary,
                  fontSize: '1.125rem',
                  fontWeight: 500,
                  textDecoration: 'none',
                }}
              >
                {label}
              </Link>
            ))}
            {canOfferKey && (
              <Button
                variant="contained"
                color="primary"
                component={RouterLink}
                to={LOGIN_ROUTE}
                onClick={close}
                endIcon={<ArrowBadge variant="onPrimary" />}
                sx={{ mt: 1.5 }}
              >
                Get API Key
              </Button>
            )}
          </Stack>
        </Container>
      </Drawer>
    </>
  );
}

export function Footer({ canOfferKey }: { canOfferKey: boolean }) {
  const links: { label: string; href?: string; to?: string }[] = [
    { label: 'Documentation', href: SWAGGER_UI },
    // Only where there is a dashboard to reach. While the portal is shut this
    // link would land on `/dashboard`, which sends a visitor with no session
    // straight back to the page they clicked from.
    //
    // `to`, not `href`: `DASHBOARD_ROUTE` is a route relative to the router's
    // basename, and as a bare `href` the browser resolved it against the
    // domain root — `/dashboard` rather than `/api-tokens/dashboard`, which is
    // a path the deployment does not serve. Every other in-app destination on
    // the page already goes through `RouterLink`; this one did not.
    ...(canOfferKey ? [{ label: 'Dashboard', to: DASHBOARD_ROUTE }] : []),
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
        {/* Centred and stacked on a phone — mark, then the links one under
            another, then the copyright — per the mobile frame; a row between
            the margins on a desktop. */}
        <Stack
          direction={{ xs: 'column', md: 'row' }}
          spacing={{ xs: 3, md: 2 }}
          alignItems="center"
          justifyContent="space-between"
          sx={{ textAlign: { xs: 'center', md: 'left' } }}
        >
          <RumbleFishMark />
          <Stack
            direction={{ xs: 'column', md: 'row' }}
            spacing={{ xs: 1.5, md: 2 }}
            alignItems="center"
            useFlexGap
            sx={{ flexWrap: 'wrap' }}
            component="nav"
            aria-label="Footer"
          >
            {links.map(({ label, href, to }) =>
              href || to ? (
                <Link
                  key={label}
                  {...(to ? { component: RouterLink, to } : { href })}
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
