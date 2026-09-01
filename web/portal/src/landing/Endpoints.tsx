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
 * The paths are the OpenAPI document's, since 2026-08-31 (task 0194, Adam's
 * call). Until then they were the design's — `/prices`, `/pools`,
 * `/history` — which this repo never served, and the difference was flagged
 * here rather than resolved; a visitor who copied the hero's curl got a
 * `403` for a route that did not exist. The list below is the seven routes
 * `verify-openapi-routes` asserts the gateway maps, in the document's order,
 * and the example beside it is the live `/v1/assets/native/price` shape.
 * `links.ts` is where the spec itself is linked from.
 */

type Method = 'GET' | 'POST';

const ENDPOINTS: readonly { method: Method; path: string; summary: string }[] =
  [
    { method: 'GET', path: '/v1/assets', summary: 'Assets, by volume' },
    { method: 'GET', path: '/v1/assets/{id}', summary: 'One asset' },
    {
      method: 'GET',
      path: '/v1/assets/{id}/price',
      summary: 'Price and 24h stats',
    },
    {
      method: 'GET',
      path: '/v1/assets/{id}/ohlcv',
      summary: 'Candles, 1m to 1M',
    },
    { method: 'GET', path: '/v1/oracles/{id}', summary: 'Oracle cross-check' },
    { method: 'GET', path: '/v1/backfill/status', summary: 'History coverage' },
    {
      method: 'POST',
      path: '/v1/prices/batch',
      summary: 'Many prices at once',
    },
  ];

/** The verb pill. `Get` in emerald as the design draws it; `Post` in the
 *  brand yellow, since the design had no second verb to draw. */
function MethodBadge({ method }: { method: Method }) {
  const post = method === 'POST';
  return (
    <Box
      component="span"
      sx={{
        flexShrink: 0,
        px: 1,
        py: 0.25,
        // A rounded rectangle, like every other chip on the page. Measured off
        // the mobile frame: a 29×24 chip whose left edge runs straight after
        // ~3px of arc, where `radius.pill` on a chip that height would be a
        // lozenge with no straight edge at all.
        borderRadius: `${radius.chip}px`,
        backgroundColor: post ? color.primary[100] : color.accent.emerald[100],
        color: post ? color.primary[900] : color.accent.emerald[900],
        fontFamily: font.secondary,
        fontWeight: 700,
        fontSize: '0.75rem',
      }}
    >
      {post ? 'Post' : 'Get'}
    </Box>
  );
}

export function Endpoints() {
  // The glow sits over the eyebrow and the headline, on the left, where the
  // design puts it — the endpoint list below stays unlit so the `Get` chips
  // keep their contrast.
  return (
    <Section tone="alt" id="endpoints" glow={{ at: '18% 22%' }}>
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
            {ENDPOINTS.map(({ method, path, summary }) => (
              <Stack
                component="li"
                key={path}
                direction="row"
                alignItems="center"
                spacing={1.5}
                // ⚠️ REQUIRED for the summary's `margin-left: auto` below. A
                // `Stack` spaces with `margin-left` on every child but the
                // first, through a selector specific enough to beat the
                // child's own `sx` — so the summary kept the Stack's 12px and
                // never moved to the right edge. `gap` leaves margins alone.
                useFlexGap
                sx={{
                  px: 2,
                  py: 1.5,
                  borderRadius: `${radius.md}px`,
                  border: cardBorder,
                  backgroundColor: cardSurface('sunken'),
                }}
              >
                <MethodBadge method={method} />
                <Typography
                  component="code"
                  sx={{
                    fontFamily: font.mono,
                    fontSize: '0.9375rem',
                    color: color.text.primary,
                    // The path is the identity of the row and never gives
                    // way; the summary yields to it when the row runs out of
                    // width.
                    flex: '0 0 auto',
                  }}
                >
                  {path}
                </Typography>
                {/* Hard against the row's right edge at every width — the
                    frame right-aligns it, desktop and mobile. Truncated with
                    an ellipsis rather than hidden where it does not fit, so a
                    375px screen loses the tail of "Historical prices", not
                    the whole column. */}
                <Typography
                  variant="body2"
                  sx={{
                    color: color.text.tertiary,
                    ml: 'auto',
                    pl: 1.5,
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    textAlign: 'right',
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
            {tok(
              color.text.tertiary,
              '// GET /v1/assets/native/price — 200 OK',
            )}
            {'\n{\n'}
            {tok(KEY, '"asset"')}: {tok(STR, '"native"')},{'\n'}
            {tok(KEY, '"price_usd"')}: {tok(STR, '"0.17735783908195"')},{'\n'}
            {tok(KEY, '"price_xlm"')}: {tok(STR, '"1"')},{'\n'}
            {tok(KEY, '"vwap_24h"')}: {tok(STR, '"0.17729898377938"')},{'\n'}
            {tok(KEY, '"volume_24h_usd"')}:{' '}
            {tok(STR, '"383736.40419055725213"')},{'\n'}
            {tok(KEY, '"change_24h_pct"')}: {tok(STR, '"-1.6635"')},{'\n'}
            {tok(KEY, '"sources"')}: {'{ '}
            {tok(KEY, '"aquarius"')}: {'{ '}
            {tok(KEY, '"price"')}: {tok(STR, '"0.1774"')},{' '}
            {tok(KEY, '"volume_24h"')}: {tok(STR, '"277436.70"')}
            {' }, '}
            {tok(KEY, '"sdex"')}: {'{…}, '}
            {tok(KEY, '"soroswap"')}: {'{…} },\n'}
            {tok(KEY, '"updated_at"')}: {tok(STR, '"2026-08-31T12:22:00Z"')}
            {'\n}'}
          </code>
        </Box>
      </WindowCard>
    </Box>
  );
}
