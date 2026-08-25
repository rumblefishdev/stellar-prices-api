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
  // The glow sits left, level with "Fair Access" — the mirror of the Endpoints
  // glow, which is what makes the two `alt` sections either side of the
  // dashboard band read as a pair.
  return (
    <Section tone="alt" id="limits" glow={{ at: '15% 20%' }}>
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
                    // A rounded SQUARE, and deliberately not the disc the
                    // feature grid and the dashboard claims take: in Figma
                    // these four ticks are squares. They are a checklist
                    // marker, not a category icon, and the design separates
                    // the two by shape.
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
          {/* Wrapped, because this `Stack` is a column flexbox and a flex
              item is stretched across the cross axis whatever its `display`
              is — the chip's own `inline-block` cannot save it, and it was
              coming out as wide as the cards below it. The wrapper takes the
              stretching; the chip inside it hugs its text and grows or
              shrinks with the words alone. The other three eyebrows on the
              page sit in Stacks with an explicit `alignItems`, which is why
              only this one was stretched — see `SectionLabel`. */}
          <Box>
            <SectionLabel tone="neutral">Free Tier Limits</SectionLabel>
          </Box>
          {LIMITS.map(({ label, figure, unit, note }) => (
            <Stack
              key={label}
              spacing={0.5}
              sx={{
                // Shorter than the design's card: 16px of padding and a half
                // step between the three lines, against 20px and a full step.
                // The column is three facts, and at the old height it ran
                // past the four reasons it answers.
                px: 2,
                py: 1.5,
                borderRadius: `${radius.lg}px`,
                // The design's one warm surface — measured #432205 (the
                // brand's darkest tint) with a #724311 hairline off the Fair
                // Access frame. It is what makes this column read as the
                // answer to the column beside it rather than a fifth card in
                // the same list. Only the CHIP above it went grey.
                backgroundColor: color.primary[950],
                border: `1px solid ${color.primary[900]}`,
              }}
            >
              <Typography
                sx={{
                  fontFamily: font.mono,
                  fontSize: '0.875rem',
                  // Grey, like the note under it: the label names the figure
                  // and should not compete with it.
                  color: color.text.tertiary,
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
                {/* White, per the design — the unit finishes the figure's
                    sentence ("1 req / second"), so it reads with it. The grey
                    is for the label above and the note below. */}
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
