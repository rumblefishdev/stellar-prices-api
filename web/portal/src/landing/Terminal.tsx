import Box from '@mui/material/Box';
import type { ReactNode } from 'react';
import { PUBLIC_API_BASE_URL } from './links';

import { color, font } from '../theme/tokens';
import { WindowCard } from './primitives';

/**
 * The hero's terminal card — a still frame of one request and its response.
 *
 * **It is a picture, not a widget.** No fetch runs behind it and none should:
 * the landing page is what a signed-out visitor sees, a live call from here
 * would need a key nobody has yet, and a "live" panel that renders an error
 * for every first-time visitor is worse than a screenshot. It is marked
 * `aria-hidden` for the same reason a screenshot would be — the sentence above
 * it says what the API does, and a screen reader spelling out a curl
 * invocation character by character adds nothing a developer can act on.
 *
 * The colouring is hand-assembled rather than run through a highlighter
 * library. One fixed snippet does not justify a parser in the bundle, and the
 * six colours it uses are design tokens (`Accent/Violet/400` for JSON keys,
 * `Accent/Emerald/400` for strings, the brand yellow for numbers and the URL)
 * rather than a theme somebody else chose.
 */

/** One coloured run inside the snippet. */
const Tok = ({ c, children }: { c: string; children: ReactNode }) => (
  <Box component="span" sx={{ color: c }}>
    {children}
  </Box>
);

const KEY = color.accent.violet[400];
const STR = color.accent.emerald[400];
const NUM = color.primary[400];
const MUTED = color.text.tertiary;
const PLAIN = color.text.primary;

export function Terminal() {
  return (
    <Box aria-hidden>
      <WindowCard title="prices-api">
        <Box
          component="pre"
          sx={{
            m: 0,
            p: 3,
            fontFamily: font.mono,
            // 12 px in the frame. Held at 12 rather than scaled down on mobile:
            // below that a monospace snippet stops being readable and starts
            // being texture, and the card is allowed to scroll instead.
            fontSize: '0.75rem',
            lineHeight: 1.7,
            color: PLAIN,
            // `pre-wrap`, not `pre`. The authored newlines are kept — they are
            // what makes the snippet read as a session rather than a paragraph —
            // but the JSON response line is longer than the card and would
            // otherwise be clipped at exactly the point a developer is reading
            // for (`"source": "soroswap"`). The Figma text box wraps it too.
            whiteSpace: 'pre-wrap',
            overflowWrap: 'anywhere',
          }}
        >
          <code>
            {'$ curl '}
            {/* The real call, on the API's own hostname — not the frame's
                `api.soroswap.finance` (a page that renders a credential must
                not aim it at another domain) and, since 2026-08-31, not the
                design's `/prices XLM-USDC` either: that path does not exist,
                and a reader's first request 403'd. Fields and shape are the
                live `/v1/assets/native/price` answer, decimals as strings. */}
            <Tok c={NUM}>{PUBLIC_API_BASE_URL}/assets/native/price</Tok>
            {' \\\n-H '}
            <Tok c={KEY}>&quot;x-api-key: YOUR_API_KEY&quot;</Tok>
            {'\n\n'}
            <Tok c={MUTED}># 200 OK — 180ms</Tok>
            {'\n{\n'}
            <Tok c={KEY}>&quot;asset&quot;</Tok>
            {': '}
            <Tok c={STR}>&quot;native&quot;</Tok>
            {', '}
            <Tok c={KEY}>&quot;price_usd&quot;</Tok>
            {': '}
            <Tok c={STR}>&quot;0.1774&quot;</Tok>
            {', '}
            <Tok c={KEY}>&quot;vwap_24h&quot;</Tok>
            {': '}
            <Tok c={STR}>&quot;0.1773&quot;</Tok>
            {', '}
            <Tok c={KEY}>&quot;change_24h_pct&quot;</Tok>
            {': '}
            <Tok c={STR}>&quot;-1.66&quot;</Tok>
            {', '}
            <Tok c={KEY}>&quot;volume_24h_usd&quot;</Tok>
            {': '}
            <Tok c={STR}>&quot;383736.40&quot;</Tok>
            {', '}
            <Tok c={KEY}>&quot;sources&quot;</Tok>
            {': { '}
            <Tok c={KEY}>&quot;aquarius&quot;</Tok>
            {': {…}, '}
            <Tok c={KEY}>&quot;sdex&quot;</Tok>
            {': {…}, '}
            <Tok c={KEY}>&quot;soroswap&quot;</Tok>
            {': {…} }'}
            {'\n}'}
            {/* The block cursor. A static glyph, not an animation: a blinking
              caret next to a code sample is a distraction on a page whose job
              is to be read once, and `prefers-reduced-motion` would have to
              turn it off anyway. */}
            <Box
              component="span"
              sx={{
                display: 'inline-block',
                width: '0.55em',
                height: '1.1em',
                ml: '0.15em',
                verticalAlign: 'text-bottom',
                backgroundColor: color.primary[400],
              }}
            />
          </code>
        </Box>
      </WindowCard>
    </Box>
  );
}
