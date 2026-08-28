import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';

import { color, font, radius } from '../theme/tokens';
import { Section, SectionHeading, cardBorder, cardSurface } from './primitives';

/**
 * "What can you build?" — six answers, numbered.
 *
 * The design's two-column split with a heading that stays put while the list
 * scrolls past it. `position: sticky` only from `md` up: at 375 px the columns
 * stack, and a sticky heading there pins a third of the viewport shut for the
 * whole section.
 *
 * The numbers are monospace and decorative — `aria-hidden`, because the list is
 * already a list and "01" read before every item is six syllables of nothing.
 */

const USE_CASES: readonly { title: string; body: string }[] = [
  {
    title: 'Wallet Applications',
    body: 'Display live token values in XLM, USD or any other asset. Keep users informed about their portfolio at a glance.',
  },
  {
    title: 'DEX Aggregators',
    body: 'Power swap routing and token comparisons. Use liquidity depth data to find optimal trade paths across Soroswap pools.',
  },
  {
    title: 'Portfolio Trackers',
    body: 'Calculate real-time portfolio value across all Stellar assets. Combine current prices with historical data for P&L.',
  },
  {
    title: 'Trading Bots',
    body: 'Automate market monitoring and arbitrage detection. Low-latency responses let you act on price movements as they happen.',
  },
  {
    title: 'Analytics Platforms',
    body: 'Build charts, track market trends and generate historical insights. Full time-series data for any Stellar asset pair.',
  },
  {
    title: 'Payment Applications',
    body: 'Estimate token values during transactions. Let users pay in their preferred asset with accurate, real-time conversion rates.',
  },
];

export function UseCases() {
  return (
    <Section tone="base" id="use-cases">
      <Stack
        direction={{ xs: 'column', md: 'row' }}
        spacing={{ xs: 4, md: 8 }}
        alignItems="flex-start"
      >
        <Box
          sx={{
            flex: { md: '0 0 38%' },
            position: { md: 'sticky' },
            top: { md: 96 },
          }}
        >
          <SectionHeading
            align="left"
            label="Use Cases"
            title="What can you build?"
            subtitle="From simple price displays to complex trading infrastructure."
          />
        </Box>

        <Stack
          component="ul"
          spacing={2}
          sx={{ flex: 1, m: 0, p: 0, listStyle: 'none', minWidth: 0 }}
        >
          {USE_CASES.map(({ title, body }, index) => (
            <Stack
              component="li"
              key={title}
              spacing={1}
              sx={{
                p: 3,
                borderRadius: `${radius.lg}px`,
                border: cardBorder,
                backgroundColor: cardSurface('deep'),
              }}
            >
              <Typography
                aria-hidden
                component="span"
                sx={{
                  fontFamily: font.mono,
                  fontSize: '0.75rem',
                  fontWeight: 500,
                  color: color.text.tertiary,
                }}
              >
                {String(index + 1).padStart(2, '0')}
              </Typography>
              <Typography variant="h5" component="h3" color="text.primary">
                {title}
              </Typography>
              <Typography variant="body2" sx={{ color: color.text.tertiary }}>
                {body}
              </Typography>
            </Stack>
          ))}
        </Stack>
      </Stack>
    </Section>
  );
}
