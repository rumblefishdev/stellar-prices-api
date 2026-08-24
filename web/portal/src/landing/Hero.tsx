import Box from '@mui/material/Box';
import Button from '@mui/material/Button';
import Container from '@mui/material/Container';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import { Link as RouterLink } from 'react-router-dom';

import rumblefishLogo from '../assets/rumblefish-logo.svg';
import { color, font, radius } from '../theme/tokens';
import { LOGIN_ROUTE, SWAGGER_UI } from './links';
import { ArrowBadge, cardBorder } from './primitives';
import { Terminal } from './Terminal';

/**
 * The first screen: what the API is, and the two things a developer does next.
 *
 * The task's story is "from the landing page to a working `curl` in under a
 * minute", and this section is the whole minute — the sentence that says what
 * the thing is, a real request beside it, and two controls. Everything below
 * this fold is elaboration.
 *
 * **"Get API Key" is an in-page anchor, not a link to the sign-in route.** The
 * portal ships CLOSED (task 0183): while `PORTAL_ENABLED` is false the backend
 * answers `/auth/*` with an empty 404, so a button pointing there would be a
 * button that cannot work — the exact thing task 0186 refused to render. The
 * anchor scrolls to the portal panel, which asks `/config` and then says
 * truthfully either "not yet available" or "sign in with Discord". That keeps
 * one control on the page in both states instead of a hero that has to know
 * which one it is in.
 */

/**
 * The `h1`. The page has exactly one, and it is this.
 *
 * `canOfferKey` is the `/config` probe's verdict, lifted to `LandingPage` — see
 * the note there. When it is false the primary call to action is not rendered
 * at all and "View documentation" takes its place as the filled button: a
 * closed portal still has an API worth reading about, and a hero left with a
 * single outlined control looks like something failed to load.
 */
export function Hero({ canOfferKey }: { canOfferKey: boolean }) {
  return (
    <Box
      component="header"
      sx={{
        position: 'relative',
        overflow: 'hidden',
        backgroundColor: color.surface.background,
        py: { xs: 7, md: 12 },
      }}
    >
      {/* The Figma frame's `Grid layers` — a faint rule grid under an elliptical
          glow. Painted with two CSS gradients rather than the exported vector:
          it is a texture nobody looks at directly, and shipping it as markup
          would put ~40 `<line>` elements in the accessibility tree for it.
          `pointer-events: none` so it never eats a click meant for the CTA. */}
      <Box
        aria-hidden
        sx={{
          position: 'absolute',
          inset: 0,
          pointerEvents: 'none',
          backgroundImage: `
            radial-gradient(60% 70% at 72% 40%, ${alpha(color.primary[400], 0.05)} 0%, transparent 70%),
            linear-gradient(${alpha(color.stroke.default, 0.12)} 1px, transparent 1px),
            linear-gradient(90deg, ${alpha(color.stroke.default, 0.12)} 1px, transparent 1px)`,
          backgroundSize: '100% 100%, 80px 80px, 80px 80px',
          // Fade the grid out before it reaches the edges, so it reads as
          // texture behind the content rather than as a table.
          maskImage:
            'radial-gradient(120% 100% at 50% 30%, #000 20%, transparent 75%)',
        }}
      />

      <Container sx={{ position: 'relative' }}>
        <Stack
          direction={{ xs: 'column', md: 'row' }}
          spacing={{ xs: 5, md: 8 }}
          alignItems={{ xs: 'stretch', md: 'center' }}
        >
          {/* 675 / 525 in the Figma frame — the copy column is the wider one.
              Equal halves put the terminal at ~600 px, which stretched the
              snippet into a single unreadable line. */}
          <Stack spacing={2.5} sx={{ flex: '1 1 56%', minWidth: 0 }}>
            <StatusBadge />

            <Typography variant="h2" component="h1" color="text.primary">
              Real-time prices for{' '}
              {/* The second line is the brand colour, and it is a `<span>`
                  inside the same heading rather than a second element: it is
                  one sentence, and splitting it would have a screen reader
                  announce two headings where a sighted reader sees one. */}
              {/* `display: block` from `md` up, so the headline breaks where
                  the design breaks it — "Real-time prices for" / "Stellar
                  developers". Letting it wrap naturally put "Stellar" on the
                  first line at 1440, which reads as a two-colour accident
                  rather than a two-line headline. Inline at `xs`, where the
                  line is too narrow for either arrangement to be a choice. */}
              <Box
                component="span"
                sx={{
                  color: color.text.accent,
                  display: { xs: 'inline', md: 'block' },
                }}
              >
                Stellar developers
              </Box>
            </Typography>

            <Typography
              variant="body1"
              sx={{ color: color.text.tertiary, maxWidth: 560 }}
            >
              Token prices, liquidity data and market insights for wallets, DEX
              aggregators and DeFi applications. Powered by Soroswap
              infrastructure.
            </Typography>

            <Stack
              direction={{ xs: 'column', sm: 'row' }}
              spacing={2}
              sx={{ pt: 1 }}
            >
              {canOfferKey && (
                <Button
                  variant="contained"
                  color="primary"
                  component={RouterLink}
                  to={LOGIN_ROUTE}
                  endIcon={<ArrowBadge variant="onPrimary" />}
                >
                  Get API Key
                </Button>
              )}
              <Button
                variant={canOfferKey ? 'outlined' : 'contained'}
                color="primary"
                href={SWAGGER_UI}
                endIcon={
                  <ArrowBadge variant={canOfferKey ? 'onDark' : 'onPrimary'} />
                }
                sx={
                  canOfferKey
                    ? {
                        backgroundColor: color.surface.gray,
                        borderColor: alpha(color.stroke.default, 0.45),
                        color: color.text.primary,
                        '&:hover': {
                          backgroundColor: color.surface.gray,
                          borderColor: color.stroke.default,
                        },
                      }
                    : undefined
                }
              >
                View documentation
              </Button>
            </Stack>
          </Stack>

          <Box sx={{ flex: '1 1 44%', minWidth: 0 }}>
            <Terminal />
          </Box>
        </Stack>
      </Container>
    </Box>
  );
}

/**
 * "Live on Stellar Mainnet".
 *
 * A static claim about the deployment, not a health indicator — there is no
 * probe behind it and the dot does not pulse. A green dot that is always green
 * is a decoration; a green dot that could be red would need a status endpoint
 * and an answer for what it shows while it is loading, which is a feature
 * nobody asked for on a marketing header.
 */
function StatusBadge() {
  return (
    <Stack
      direction="row"
      spacing={1}
      alignItems="center"
      sx={{
        alignSelf: 'flex-start',
        // Solid, borderless, and the full `Surface/Success` value — measured
        // #0d542b off the export. The earlier version washed it out to 55% and
        // added a ring the design does not have, which made the badge read as
        // a disabled chip rather than a status light.
        backgroundColor: color.surface.success,
        borderRadius: `${radius.pill}px`,
        px: 1.5,
        py: 0.5,
      }}
    >
      <Box
        aria-hidden
        sx={{
          width: 10,
          height: 10,
          borderRadius: '50%',
          backgroundColor: color.text.success,
        }}
      />
      <Typography
        component="span"
        sx={{
          fontFamily: font.secondary,
          fontSize: '0.9375rem',
          fontWeight: 500,
          color: color.text.success,
        }}
      >
        Live on Stellar Mainnet
      </Typography>
    </Stack>
  );
}

/**
 * The strip under the hero: who built it, and four claims about what it is.
 *
 * Separate from `Hero` because it is a separate frame in the design with its
 * own background, and because it is the first thing that would be cut if the
 * page needed to be shorter — a section that can be deleted in one line is
 * worth keeping in one component.
 */
export function TrustBand() {
  const claims = [
    'Production-ready',
    'API Gateway protected',
    'Stellar ecosystem',
    'OpenAPI docs',
  ];

  return (
    <Box
      sx={{
        backgroundColor: color.surface.background,
        borderTop: cardBorder,
        borderBottom: cardBorder,
        py: 3,
      }}
    >
      <Container>
        <Stack
          direction={{ xs: 'column', md: 'row' }}
          spacing={{ xs: 2, md: 4 }}
          alignItems="center"
          justifyContent="center"
          divider={
            <Box
              aria-hidden
              sx={{
                display: { xs: 'none', md: 'block' },
                width: '1px',
                alignSelf: 'stretch',
                backgroundColor: alpha(color.stroke.default, 0.45),
              }}
            />
          }
        >
          <Stack direction="row" spacing={1.5} alignItems="center">
            <Typography variant="body1" sx={{ color: color.text.tertiary }}>
              Built by
            </Typography>
            {/* The real mark, recovered from the Figma export — see the note
                in `Chrome.tsx`. It carries "Rumble Fish" as its `alt`, so the
                line still reads "Built by Rumble Fish" to a screen reader. */}
            <Box
              component="img"
              src={rumblefishLogo}
              alt="Rumble Fish"
              sx={{ height: 32, width: 'auto', display: 'block' }}
            />
          </Stack>

          <Stack
            direction="row"
            spacing={1}
            useFlexGap
            sx={{ flexWrap: 'wrap', justifyContent: 'center' }}
          >
            {claims.map((claim) => (
              <Typography
                key={claim}
                component="span"
                variant="body2"
                sx={{
                  color: color.text.secondary,
                  backgroundColor: color.surface.gray,
                  border: cardBorder,
                  borderRadius: `${radius.md}px`,
                  px: 1.5,
                  py: 1.5,
                }}
              >
                {claim}
              </Typography>
            ))}
          </Stack>
        </Stack>
      </Container>
    </Box>
  );
}
