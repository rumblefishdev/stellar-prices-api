import AutorenewRoundedIcon from '@mui/icons-material/AutorenewRounded';
import CodeRoundedIcon from '@mui/icons-material/CodeRounded';
import LockOutlinedIcon from '@mui/icons-material/LockOutlined';
import MonitorHeartOutlinedIcon from '@mui/icons-material/MonitorHeartOutlined';
import ShowChartRoundedIcon from '@mui/icons-material/ShowChartRounded';
import TrendingUpRoundedIcon from '@mui/icons-material/TrendingUpRounded';
import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import type { SvgIconComponent } from '@mui/icons-material';

import { color, radius } from '../theme/tokens';
import { Section, SectionHeading, cardBorder, cardSurface } from './primitives';

/**
 * "Everything you need to build Stellar applications" — six claims, one grid.
 *
 * The icons are **Material equivalents, not the exported Figma vectors.**
 * Reading the real SVGs out of the file needs `download_assets`, and the Figma
 * seat this was built against was out of monthly tool calls; the glyph shapes
 * were matched from the rendered frame instead. Swapping in the exported
 * assets is a one-line change per row and should happen — this note is here so
 * the next person knows these were chosen, not designed.
 *
 * The three-hue cycle (emerald, blue, violet) IS from the design, including
 * the pairing: the disc takes the accent's 900 shade and the glyph its 100,
 * which is what keeps six bright icons from competing with the yellow CTA
 * above them.
 */

type Feature = {
  icon: SvgIconComponent;
  accent: { disc: string; glyph: string };
  title: string;
  body: string;
};

const EMERALD = {
  disc: color.accent.emerald[900],
  glyph: color.accent.emerald[100],
};
const BLUE = { disc: color.accent.blue[900], glyph: color.accent.blue[100] };
const VIOLET = {
  disc: color.accent.violet[900],
  glyph: color.accent.violet[100],
};

const FEATURES: readonly Feature[] = [
  {
    icon: AutorenewRoundedIcon,
    accent: EMERALD,
    title: 'Live Prices',
    body: 'Real-time token prices for all Stellar assets. Sourced directly from Soroswap liquidity pools, updated on every block.',
  },
  {
    icon: ShowChartRoundedIcon,
    accent: BLUE,
    title: 'Liquidity Data',
    body: 'Pool reserves, trading depth and liquidity metrics. Essential for swap routing and price impact calculations.',
  },
  {
    icon: TrendingUpRoundedIcon,
    accent: VIOLET,
    title: 'Historical Data',
    body: 'Price history for charts and analytics. Build portfolio trackers and trading dashboards with full time-series data.',
  },
  {
    icon: MonitorHeartOutlinedIcon,
    accent: EMERALD,
    title: 'Fast Response Times',
    body: 'API Gateway caching keeps latency low for repeated lookups. Optimized for high-frequency applications like trading bots.',
  },
  {
    icon: LockOutlinedIcon,
    accent: BLUE,
    title: 'Secure Access',
    body: 'Every request requires an API key. Rate limiting and monthly quotas protect the service for all users.',
  },
  {
    icon: CodeRoundedIcon,
    accent: VIOLET,
    title: 'Developer Friendly',
    body: 'REST API with full OpenAPI specification. Swagger UI included. SDK examples in JavaScript, Python, Rust and Go.',
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

        {/* CSS grid rather than MUI's `Grid`: three equal columns that become
            one at 375 px is a single `repeat(auto-fit, …)` declaration, and the
            cards must stretch to the tallest in their row — which `Grid`'s
            item-level sizing does not give without a wrapper per cell. */}
        <Box
          sx={{
            display: 'grid',
            gap: 2,
            width: '100%',
            gridTemplateColumns: {
              xs: '1fr',
              sm: 'repeat(2, 1fr)',
              lg: 'repeat(3, 1fr)',
            },
          }}
        >
          {FEATURES.map(({ icon: Icon, accent, title, body }) => (
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
              <Box
                aria-hidden
                sx={{
                  width: 32,
                  height: 32,
                  borderRadius: '8px',
                  display: 'grid',
                  placeItems: 'center',
                  backgroundColor: accent.disc,
                  color: accent.glyph,
                }}
              >
                <Icon sx={{ fontSize: 18 }} />
              </Box>
              <Typography variant="h5" component="h3" color="text.primary">
                {title}
              </Typography>
              <Typography variant="body2" sx={{ color: color.text.tertiary }}>
                {body}
              </Typography>
            </Stack>
          ))}
        </Box>
      </Stack>
    </Section>
  );
}
