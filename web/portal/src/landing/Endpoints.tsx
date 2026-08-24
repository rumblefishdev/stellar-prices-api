import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';

import { color, font, radius } from '../theme/tokens';
import {
  Section,
  SectionHeading,
  WindowCard,
  cardBorder,
  cardSurface,
} from './primitives';

/**
 * "Clean REST API. Full OpenAPI spec." — the route list and one real response.
 *
 * The paths are the design's, not the OpenAPI document's, and that is worth
 * saying out loud: this repo serves `/v1/assets/{id}/price` and friends, while
 * the mock shows `/prices`, `/pools` and `/history`. Reconciling the two is a
 * product decision about the public surface, not a styling one, so this slice
 * renders what the design says and flags the difference rather than quietly
 * inventing a third answer. `links.ts` is where the real spec is linked from.
 */

const ENDPOINTS: readonly { path: string; summary: string }[] = [
  { path: '/prices', summary: 'All asset prices' },
  { path: '/prices/{asset}', summary: 'Single asset' },
  { path: '/pools', summary: 'Liquidity pools' },
  { path: '/pools/{id}/stats', summary: 'Pool statistics' },
  { path: '/history/{asset}', summary: 'Historical prices' },
];

/** The `Get` pill. One verb, so it is a constant rather than a prop. */
function MethodBadge() {
  return (
    <Box
      component="span"
      sx={{
        flexShrink: 0,
        px: 1,
        py: 0.25,
        borderRadius: `${radius.pill}px`,
        backgroundColor: color.accent.emerald[100],
        color: color.accent.emerald[900],
        fontFamily: font.secondary,
        fontWeight: 700,
        fontSize: '0.75rem',
      }}
    >
      Get
    </Box>
  );
}

export function Endpoints() {
  return (
    <Section tone="alt" id="endpoints">
      <Stack
        direction={{ xs: 'column', md: 'row' }}
        spacing={{ xs: 4, md: 6 }}
        alignItems="flex-start"
      >
        <Stack spacing={3} sx={{ flex: 1, minWidth: 0, width: '100%' }}>
          <SectionHeading
            align="left"
            label="Endpoints"
            title={
              <>
                Clean REST API.
                <Box component="br" /> Full OpenAPI spec.
              </>
            }
            subtitle="Every endpoint documented, every response typed."
          />

          <Stack
            component="ul"
            spacing={1.5}
            sx={{ m: 0, p: 0, listStyle: 'none', width: '100%' }}
          >
            {ENDPOINTS.map(({ path, summary }) => (
              <Stack
                component="li"
                key={path}
                direction="row"
                alignItems="center"
                spacing={1.5}
                sx={{
                  px: 2,
                  py: 1.5,
                  borderRadius: `${radius.md}px`,
                  border: cardBorder,
                  backgroundColor: cardSurface('sunken'),
                }}
              >
                <MethodBadge />
                <Typography
                  component="code"
                  sx={{
                    fontFamily: font.mono,
                    fontSize: '0.9375rem',
                    color: color.text.primary,
                    // The path is the identity of the row; the summary yields
                    // to it when the row runs out of width.
                    flex: '1 1 auto',
                    minWidth: 0,
                  }}
                >
                  {path}
                </Typography>
                <Typography
                  variant="body2"
                  sx={{
                    color: color.text.tertiary,
                    flexShrink: 0,
                    display: { xs: 'none', sm: 'block' },
                  }}
                >
                  {summary}
                </Typography>
              </Stack>
            ))}
          </Stack>
        </Stack>

        <Box sx={{ flex: 1, minWidth: 0, width: '100%' }}>
          <ExampleResponse />
        </Box>
      </Stack>
    </Section>
  );
}

/** The response beside the list. A still frame, like the hero's terminal. */
function ExampleResponse() {
  const KEY = color.accent.violet[400];
  const STR = color.accent.emerald[400];
  const NUM = color.primary[400];
  const tok = (c: string, text: string) => (
    <Box component="span" sx={{ color: c }}>
      {text}
    </Box>
  );

  return (
    <Box aria-hidden>
      <WindowCard title="Example Response" surface="sunken">
        <Box
          component="pre"
          sx={{
            m: 0,
            p: 3,
            fontFamily: font.mono,
            fontSize: '0.75rem',
            lineHeight: 1.9,
            color: color.text.primary,
            whiteSpace: 'pre-wrap',
            overflowWrap: 'anywhere',
          }}
        >
          <code>
            {tok(color.text.tertiary, '// GET /prices/XLM-USDC — 200 OK')}
            {'\n{\n'}
            {tok(KEY, '"asset"')}: {tok(STR, '"XLM-USDC"')},{'\n'}
            {tok(KEY, '"price"')}: {tok(NUM, '0.0812')},{'\n'}
            {tok(KEY, '"change_24h"')} {tok(NUM, '+2.14')},{'\n'}
            {tok(KEY, '"volume_24h"')}: {tok(NUM, '142891.50')},{'\n'}
            {tok(KEY, '"liquidity"')}: {tok(NUM, '2400000')},{'\n'}
            {tok(KEY, '"source"')}: {tok(STR, '"soroswap"')},{'\n'}
            {tok(KEY, '"updated_at"')}: {tok(STR, '"2026-04-13T14:23:51Z"')}
            {'\n}'}
          </code>
        </Box>
      </WindowCard>
    </Box>
  );
}
