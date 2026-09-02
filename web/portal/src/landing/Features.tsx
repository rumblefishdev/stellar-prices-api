import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';

import developerFriendlyIcon from '../assets/icons/feature-developer-friendly.svg';
import fastResponseIcon from '../assets/icons/feature-fast-response.svg';
import historicalDataIcon from '../assets/icons/feature-historical-data.svg';
import liquidityDataIcon from '../assets/icons/feature-liquidity-data.svg';
import livePricesIcon from '../assets/icons/feature-live-prices.svg';
import secureAccessIcon from '../assets/icons/feature-secure-access.svg';
import { color, radius } from '../theme/tokens';
import {
  CardRail,
  Section,
  SectionHeading,
  cardBorder,
  cardSurface,
} from './primitives';

/**
 * "Everything you need to build Stellar applications" — six claims, one grid.
 *
 * **The icons are the exported Figma vectors** (Adam's `whypricesapi.zip`),
 * not Material stand-ins. Two earlier passes guessed at them: the first
 * matched glyph shapes off a screenshot because the Figma seat had no tool
 * calls left, the second swapped in better-fitting Material icons. Both are
 * gone.
 *
 * Each file is the WHOLE 32×32 badge — the coloured disc is the first path in
 * the SVG and the glyph the second — so there is no tile to build here and no
 * colours to pair. That also corrects an inversion the hand-built tile had:
 * the design fills the disc with the accent's **100** shade and draws the
 * glyph in its **900**, and the code had it the other way round, which made
 * six dark discs where the frame has six pale ones.
 *
 * The three-hue cycle (emerald, blue, violet, repeating) is baked into the
 * files in the order the cards appear, which is how these six map onto the
 * six rows below.
 */

type Feature = { icon: string; title: string; body: string };

const FEATURES: readonly Feature[] = [
  {
    icon: livePricesIcon,
    title: 'Live Prices',
    body: 'Real-time token prices for all Stellar assets. Sourced directly from Soroswap liquidity pools, updated on every block.',
  },
  {
    icon: liquidityDataIcon,
    title: 'Liquidity Data',
    body: 'Pool reserves, trading depth and liquidity metrics. Essential for swap routing and price impact calculations.',
  },
  {
    icon: historicalDataIcon,
    title: 'Historical Data',
    body: 'Price history for charts and analytics. Build portfolio trackers and trading dashboards with full time-series data.',
  },
  {
    icon: fastResponseIcon,
    title: 'Fast Response Times',
    body: 'API Gateway caching keeps latency low for repeated lookups. Optimized for high-frequency applications like trading bots.',
  },
  {
    icon: secureAccessIcon,
    title: 'Secure Access',
    body: 'Every request requires an API key. Rate limiting and monthly quotas protect the service for all users.',
  },
  {
    icon: developerFriendlyIcon,
    title: 'Developer Friendly',
    body: 'REST API with a full OpenAPI specification, a browsable API reference, and copy-ready examples in four languages.',
  },
];

export function Features() {
  return (
    <Section tone="alt" id="features">
      <Stack spacing={{ xs: 4, md: 6 }} alignItems="center">
        <SectionHeading
          label="Why Prices API"
          title={
            <>
              Everything you need to build
              {/* A hard break, matching the design's two centred lines — but
                  only from `md` up. At 375 px the headline wraps on its own and
                  a forced break lands mid-phrase. */}
              <Box
                component="br"
                sx={{ display: { xs: 'none', md: 'block' } }}
              />{' '}
              Stellar applications
            </>
          }
          subtitle="Purpose-built for the Stellar ecosystem. Not a generic crypto data provider."
        />

        {/* Three across, two on a tablet, and a swipeable rail on a phone —
            the mobile frame shows one card with the next peeking at the
            edge, not six stacked. The cards stretch to the tallest in their
            row either way. */}
        <CardRail columns={{ sm: 'repeat(2, 1fr)', lg: 'repeat(3, 1fr)' }}>
          {FEATURES.map(({ icon, title, body }) => (
            <Stack
              key={title}
              spacing={2}
              sx={{
                p: 3,
                height: '100%',
                borderRadius: `${radius.lg}px`,
                border: cardBorder,
                backgroundColor: cardSurface('raised'),
              }}
            >
              {/* Decorative: the card's own heading says what it is, and the
                  badge carries its disc and glyph in one file at its design
                  size. */}
              <Box
                component="img"
                src={icon}
                alt=""
                aria-hidden
                sx={{ width: 32, height: 32, display: 'block' }}
              />
              <Typography variant="h5" component="h3" color="text.primary">
                {title}
              </Typography>
              <Typography variant="body2" sx={{ color: color.text.tertiary }}>
                {body}
              </Typography>
            </Stack>
          ))}
        </CardRail>
      </Stack>
    </Section>
  );
}
