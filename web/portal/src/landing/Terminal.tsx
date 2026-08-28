import Box from '@mui/material/Box';
import type { ReactNode } from 'react';

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
            {/* Our host, not the frame's `api.soroswap.finance` — the same
                reasoning as `quickstart/QuickStart.tsx`'s `BASE_URL`: a page
                that renders a credential must not aim it at another domain.
                The path stays the design's; task 0233 reconciles it. */}
            <Tok c={NUM}>
              https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/prices
            </Tok>
            {' XLM-USDC \\\n-H '}
            <Tok c={KEY}>&quot;x-api-key: sf_live_k8mN2p...&quot;</Tok>
            {'\n\n'}
            <Tok c={MUTED}># 200 OK — 38ms</Tok>
            {'\n{\n'}
            <Tok c={KEY}>&quot;asset&quot;</Tok>
            {': '}
            <Tok c={STR}>&quot;XLM-USDC&quot;</Tok>
            {', '}
            <Tok c={KEY}>&quot;price&quot;</Tok>
            {': '}
            <Tok c={NUM}>0.0812</Tok>
            {', '}
            <Tok c={KEY}>&quot;change_24h&quot;</Tok>
            {': '}
            <Tok c={NUM}>+2.14</Tok>
            {', '}
            <Tok c={KEY}>&quot;volume_24h&quot;</Tok>
            {': '}
            <Tok c={NUM}>142891.50</Tok>
            {', '}
            <Tok c={KEY}>&quot;liquidity&quot;</Tok>
            {': '}
            <Tok c={NUM}>2400000</Tok>
            {', '}
            <Tok c={KEY}>&quot;source&quot;</Tok>
            {': '}
            <Tok c={STR}>&quot;soroswap&quot;</Tok>
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
