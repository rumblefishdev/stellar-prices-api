import CheckRoundedIcon from '@mui/icons-material/CheckRounded';
import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';

import { color, font, radius } from '../theme/tokens';
import {
  Section,
  SectionHeading,
  SectionLabel,
  cardBorder,
  cardSurface,
} from './primitives';

/**
 * "Reliable for every developer" — why the limits exist, and what they are.
 *
 * The three figures on the right are the same ones the dashboard shows a
 * signed-in visitor (task 0188), which is the reason they are stated here at
 * all: a developer deciding whether to build on this needs the quota BEFORE
 * they have a key, not after.
 *
 * ⚠️ **These are hard-coded and the dashboard's are not.** The dashboard reads
 * the rate limit from `/config` precisely so it cannot drift from what the
 * gateway enforces; a marketing section cannot, because it renders for visitors
 * with no session and often before the probe answers. If the free plan's limits
 * change, this file is one of the two places that must change with it.
 */

const REASONS: readonly string[] = [
  'Discord OAuth — no throwaway signups',
  '1 req/s per key — 2x CoinGecko free tier',
  '100,000 requests/month quota',
  'AWS API Gateway infrastructure',
];

const LIMITS: readonly {
  label: string;
  figure: string;
  unit: string;
  note: string;
}[] = [
  {
    label: 'Rate limit',
    figure: '1',
    unit: 'req / second',
    note: '60 requests per minute',
  },
  {
    label: 'Monthly quota',
    figure: '100K',
    unit: 'requests / mo',
    note: 'Resets on the 1st of each month',
  },
  {
    label: 'Cost',
    figure: '$0',
    unit: 'free tier',
    note: 'No credit card required',
  },
];

export function FairAccess() {
  return (
    <Section tone="alt" id="limits">
      <Stack
        direction={{ xs: 'column', md: 'row' }}
        spacing={{ xs: 4, md: 6 }}
        alignItems="flex-start"
      >
        <Stack spacing={3} sx={{ flex: 1, minWidth: 0, width: '100%' }}>
          <SectionHeading
            align="left"
            label="Fair Access"
            title={
              <>
                Reliable for
                <Box component="br" /> every developer
              </>
            }
            subtitle="Rate limits exist so no single key can degrade the experience for everyone else."
          />

          <Stack
            component="ul"
            spacing={1.5}
            sx={{ m: 0, p: 0, listStyle: 'none', width: '100%' }}
          >
            {REASONS.map((reason) => (
              <Stack
                component="li"
                key={reason}
                direction="row"
                spacing={2}
                alignItems="center"
                sx={{
                  p: 2,
                  borderRadius: `${radius.md}px`,
                  border: cardBorder,
                  backgroundColor: cardSurface('sunken'),
                }}
              >
                <Box
                  aria-hidden
                  sx={{
                    flexShrink: 0,
                    width: 28,
                    height: 28,
                    borderRadius: '8px',
                    display: 'grid',
                    placeItems: 'center',
                    backgroundColor: color.accent.emerald[100],
                    color: color.accent.emerald[900],
                  }}
                >
                  <CheckRoundedIcon sx={{ fontSize: 18 }} />
                </Box>
                <Typography variant="body1" color="text.primary">
                  {reason}
                </Typography>
              </Stack>
            ))}
          </Stack>
        </Stack>

        <Stack spacing={2} sx={{ flex: 1, minWidth: 0, width: '100%' }}>
          <SectionLabel>Free Tier Limits</SectionLabel>
          {LIMITS.map(({ label, figure, unit, note }) => (
            <Stack
              key={label}
              spacing={1}
              sx={{
                p: 2.5,
                borderRadius: `${radius.lg}px`,
                // The design's one warm surface: the brand's darkest tint
                // rather than the neutral card, which is what makes this
                // column read as the answer to the column beside it.
                backgroundColor: color.primary[950],
                border: `1px solid ${color.primary[900]}`,
              }}
            >
              <Typography
                sx={{
                  fontFamily: font.mono,
                  fontSize: '0.875rem',
                  color: color.text.secondary,
                }}
              >
                {label}
              </Typography>
              <Stack direction="row" spacing={1.5} alignItems="baseline">
                <Typography
                  component="span"
                  sx={{
                    fontFamily: font.primary,
                    fontWeight: 700,
                    fontSize: '2rem',
                    lineHeight: 1.1,
                    color: color.text.accent,
                  }}
                >
                  {figure}
                </Typography>
                <Typography variant="body1" color="text.primary">
                  {unit}
                </Typography>
              </Stack>
              <Typography
                sx={{
                  fontFamily: font.mono,
                  fontSize: '0.8125rem',
                  color: color.text.tertiary,
                }}
              >
                {note}
              </Typography>
            </Stack>
          ))}
        </Stack>
      </Stack>
    </Section>
  );
}
