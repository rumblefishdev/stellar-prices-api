import ChevronRightRoundedIcon from '@mui/icons-material/ChevronRightRounded';
import Box from '@mui/material/Box';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { useState, type KeyboardEvent, type ReactNode } from 'react';
import { Link as RouterLink } from 'react-router-dom';
import { PUBLIC_API_BASE_URL } from '../landing/links';

/* The three "What's next" badges, exported from the file (Adam's
   `Designs.zip`, `Layout/251`) — a pale-yellow disc with a brown glyph, one
   file each, exactly as the frame draws them. Real assets rather than the
   nearest MUI glyph in a hand-built circle: the bar chart, the document and
   the plus are the design's shapes, and approximating them here is how a
   page stops matching the file it came from. */
import apiReferenceIcon from '../assets/icons/next-api-reference.svg';
import higherLimitsIcon from '../assets/icons/next-higher-limits.svg';
import monitorUsageIcon from '../assets/icons/next-monitor-usage.svg';
import { color, font, radius } from '../theme/tokens';
import {
  DASHBOARD_ROUTE,
  LOGIN_ROUTE,
  STELLAR_DISCORD_INVITE,
  API_REFERENCE,
} from '../landing/links';
import {
  Code,
  DocCard,
  DocPage,
  DocSection,
  ValueStrip,
} from '../landing/DocPrimitives';
import { cardBorder } from '../landing/primitives';
import { panelBorder } from '../landing/DashboardPanel';

/**
 * The quick start (Figma `Quick start` frame, `918:644`): one long page that
 * gets a developer from a Discord account to a parsed response.
 *
 * **Everything on it is a still.** No request runs from this page and none
 * should — it is documentation. Neither does anything on the API reference
 * (task 0195): a "try it" needs the data routes to answer CORS, which is task
 * 0126's. Every snippet is a string the visitor copies, which is why the one
 * interactive thing here is the copy button.
 *
 * The PATHS, the fields and the error bodies are this repo's OpenAPI
 * document's, since 2026-08-31 (task 0194, Adam's call) — every value below
 * was read off the live API that day. Until then the page rendered the
 * DESIGN's surface (`/assets/native/price`, `/pools`, `/history`, a `liquidity`
 * field, a `source: "soroswap"`), which this API never served: a reader who
 * copied "First request" with a freshly issued key got API Gateway's
 * `403 Missing Authentication Token`, on the page whose whole promise is a
 * response in under five minutes. Task 0233 tracked the reconciliation; the
 * portal's half of it is done here.
 *
 * The HOST is ours — `PUBLIC_API_BASE_URL`, the API's own hostname — and not
 * the frame's `api.soroswap.finance`: a page that renders a credential must
 * not aim it at another domain.
 */

const BASE_URL = PUBLIC_API_BASE_URL;
const PLACEHOLDER_KEY = 'YOUR_API_KEY';

/** The sections, in page order. Doubles as the left-hand table of contents. */
const SECTIONS = [
  { id: 'prerequisites', label: 'Prerequisites' },
  { id: 'authentication', label: 'Authentication' },
  { id: 'base-url', label: 'Base URL' },
  { id: 'first-request', label: 'First request' },
  { id: 'response', label: 'Understanding the response' },
  { id: 'endpoints', label: 'Endpoints' },
  { id: 'errors', label: 'Error handling' },
  { id: 'rate-limits', label: 'Rate limits' },
  { id: 'sdk', label: 'SDK examples' },
  { id: 'next', label: "What's next" },
] as const;

/* -------------------------------------------------------------------------- */
/* Syntax colouring                                                           */
/* -------------------------------------------------------------------------- */

/**
 * The same five colours the hero's terminal uses, for the same reason: a
 * handful of fixed snippets does not justify a highlighter in the bundle, and
 * these are design tokens rather than somebody else's theme.
 */
const KEY = color.accent.violet[400];
const STR = color.accent.emerald[400];
const NUM = color.primary[400];
const MUTED = color.text.tertiary;
const PLAIN = color.text.secondary;

/** One coloured run. `c` omitted is the plain body colour. */
const Tok = ({ c = PLAIN, children }: { c?: string; children: ReactNode }) => (
  <Box component="span" sx={{ color: c }}>
    {children}
  </Box>
);

/**
 * A snippet is authored twice: once as coloured JSX for the eye, once as the
 * plain string the copy button writes. Deriving one from the other would need
 * a tokenizer; keeping both next to each other in one object is what keeps
 * them from drifting.
 *
 * ⚠️ Proximity is NOT what kept them from drifting — task 0194's review found
 * two snippets whose `view` declared `prices` and then used `price`, left by a
 * rename applied to `text` and to only some of the JSX identifiers. The JS one
 * threw `ReferenceError` for anyone who retyped what was on screen; the Rust
 * one did not compile. Both copy buttons wrote correct code, so nothing on the
 * page and nothing in the suite could see it. What keeps them together now is
 * `SNIPPET_TABLES` below and the spec that renders every `view` and compares
 * its text to `text` — the same shape as `base-path.spec.ts` over the two
 * copies of `BASE_PATH`.
 */
type Snippet = { text: string; view: ReactNode };

/**
 * The frame's segmented control above a snippet — cURL / JavaScript / Python
 * / Go. Real tabs (`role="tab"`, arrow keys), because that is what they are:
 * one panel, several views of it.
 */
function LangTabs<T extends string>({
  id,
  options,
  value,
  onChange,
}: {
  id: string;
  options: readonly { key: T; label: string }[];
  value: T;
  onChange: (next: T) => void;
}) {
  const onKeyDown = (e: KeyboardEvent, index: number) => {
    const delta = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
    if (!delta) return;
    e.preventDefault();
    const next = options[(index + delta + options.length) % options.length];
    onChange(next.key);
    document.getElementById(`${id}-tab-${next.key}`)?.focus();
  };

  return (
    <Stack
      direction="row"
      role="tablist"
      aria-label="Language"
      sx={{
        alignSelf: 'flex-start',
        maxWidth: '100%',
        overflowX: 'auto',
        p: 0.5,
        borderRadius: `${radius.md}px`,
        border: cardBorder,
        backgroundColor: color.surface.grayAlt,
      }}
    >
      {options.map((o, i) => {
        const selected = o.key === value;
        return (
          <Box
            key={o.key}
            component="button"
            type="button"
            role="tab"
            id={`${id}-tab-${o.key}`}
            aria-selected={selected}
            aria-controls={`${id}-panel`}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(o.key)}
            onKeyDown={(e: KeyboardEvent) => onKeyDown(e, i)}
            sx={{
              cursor: 'pointer',
              border: 'none',
              borderRadius: `${radius.md - 4}px`,
              px: 2,
              py: 1,
              whiteSpace: 'nowrap',
              fontFamily: font.secondary,
              fontSize: '0.875rem',
              fontWeight: 500,
              color: selected ? color.text.primary : color.text.tertiary,
              backgroundColor: selected ? color.surface.gray : 'transparent',
              '&:hover': { color: color.text.primary },
              '&:focus-visible': {
                outline: `2px solid ${color.stroke.action}`,
                outlineOffset: -2,
              },
            }}
          >
            {o.label}
          </Box>
        );
      })}
    </Stack>
  );
}

/** Tabs plus the card they switch. */
function SnippetTabs<T extends string>({
  id,
  options,
  snippets,
  title,
}: {
  id: string;
  options: readonly { key: T; label: string }[];
  snippets: Record<T, Snippet>;
  title: (key: T) => string;
}) {
  const [lang, setLang] = useState<T>(options[0].key);
  const snippet = snippets[lang];
  return (
    <Stack spacing={2}>
      <LangTabs id={id} options={options} value={lang} onChange={setLang} />
      <Box
        id={`${id}-panel`}
        role="tabpanel"
        aria-labelledby={`${id}-tab-${lang}`}
      >
        <DocCard
          title={title(lang)}
          copy={{ text: snippet.text, label: `${title(lang)} example` }}
        >
          <Code>{snippet.view}</Code>
        </DocCard>
      </Box>
    </Stack>
  );
}

/* -------------------------------------------------------------------------- */
/* Snippets                                                                   */
/* -------------------------------------------------------------------------- */

const FIRST_REQUEST_LANGS = [
  { key: 'curl', label: 'cURL' },
  { key: 'js', label: 'JavaScript' },
  { key: 'python', label: 'Python' },
  { key: 'go', label: 'Go' },
] as const;
type FirstRequestLang = (typeof FIRST_REQUEST_LANGS)[number]['key'];

const FIRST_REQUEST: Record<FirstRequestLang, Snippet> = {
  curl: {
    text: `curl ${BASE_URL}/assets/native/price \\\n  -H "x-api-key: ${PLACEHOLDER_KEY}"`,
    view: (
      <>
        <Tok c={MUTED}>curl </Tok>
        <Tok c={NUM}>{BASE_URL}/assets/native/price</Tok>
        {' \\\n  -H '}
        <Tok c={STR}>&quot;x-api-key: {PLACEHOLDER_KEY}&quot;</Tok>
      </>
    ),
  },
  js: {
    text: `const res = await fetch("${BASE_URL}/assets/native/price", {\n  headers: { "x-api-key": "${PLACEHOLDER_KEY}" }\n});\nconst price = await res.json();`,
    view: (
      <>
        <Tok c={KEY}>const</Tok> res = <Tok c={KEY}>await</Tok> fetch(
        <Tok c={STR}>&quot;{BASE_URL}/assets/native/price&quot;</Tok>
        {', {\n  headers: { '}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>
        {': '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>
        {' }\n});\n'}
        <Tok c={KEY}>const</Tok> price = <Tok c={KEY}>await</Tok> res.json();
      </>
    ),
  },
  python: {
    text: `import requests\n\nres = requests.get(\n    "${BASE_URL}/assets/native/price",\n    headers={"x-api-key": "${PLACEHOLDER_KEY}"},\n)\nprice = res.json()`,
    view: (
      <>
        <Tok c={KEY}>import</Tok> requests{'\n\n'}res = requests.get(
        {'\n    '}
        <Tok c={STR}>&quot;{BASE_URL}/assets/native/price&quot;</Tok>
        {',\n    headers={'}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>
        {': '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>
        {'},\n)\nprice = res.json()'}
      </>
    ),
  },
  go: {
    text: `req, _ := http.NewRequest("GET", "${BASE_URL}/assets/native/price", nil)\nreq.Header.Set("x-api-key", "${PLACEHOLDER_KEY}")\nres, err := http.DefaultClient.Do(req)`,
    view: (
      <>
        req, _ := http.NewRequest(<Tok c={STR}>&quot;GET&quot;</Tok>,{' '}
        <Tok c={STR}>&quot;{BASE_URL}/assets/native/price&quot;</Tok>, nil)
        {'\n'}
        req.Header.Set(<Tok c={STR}>&quot;x-api-key&quot;</Tok>,{' '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>){'\n'}res, err :=
        http.DefaultClient.Do(req)
      </>
    ),
  },
};

const FIRST_REQUEST_TITLE: Record<FirstRequestLang, string> = {
  curl: 'curl',
  js: 'javascript',
  python: 'python',
  go: 'go',
};

/**
 * The 200 response, and what each field means — the two columns of the frame.
 *
 * Exported for `QuickStart.spec.tsx`, which ties `value` to `raw` the way it
 * ties every snippet's `view` to its `text`, and parses the assembled
 * `RESPONSE_TEXT` as JSON — the two properties this table promises a reader.
 */
export const RESPONSE_FIELDS: readonly {
  key: string;
  value: ReactNode;
  raw: string;
  dot: string;
  meaning: string;
}[] = [
  {
    key: 'asset',
    value: <Tok c={STR}>&quot;native&quot;</Tok>,
    raw: '"native"',
    dot: KEY,
    meaning:
      'The identifier you asked for — native, CODE:ISSUER, or a C… contract',
  },
  {
    key: 'price_usd',
    value: <Tok c={STR}>&quot;0.17735783908195&quot;</Tok>,
    raw: '"0.17735783908195"',
    dot: NUM,
    meaning: 'Current price in USD. A decimal string, never a float',
  },
  {
    key: 'price_xlm',
    value: <Tok c={STR}>&quot;1&quot;</Tok>,
    raw: '"1"',
    dot: NUM,
    meaning: 'The same price in XLM (1 for XLM itself)',
  },
  {
    key: 'vwap_24h',
    value: <Tok c={STR}>&quot;0.17729898377938&quot;</Tok>,
    raw: '"0.17729898377938"',
    dot: NUM,
    meaning: 'Volume-weighted average price over the last 24 hours',
  },
  {
    key: 'volume_24h_usd',
    value: <Tok c={STR}>&quot;383736.40419055725213&quot;</Tok>,
    raw: '"383736.40419055725213"',
    dot: NUM,
    meaning: '24h traded volume in USD, all venues combined',
  },
  {
    key: 'change_24h_pct',
    value: <Tok c={STR}>&quot;-1.6635&quot;</Tok>,
    raw: '"-1.6635"',
    dot: NUM,
    meaning: '% change over the last 24 hours',
  },
  {
    // All three venues spelled out, in `value` and `raw` alike. An earlier
    // version elided two of them as `{…}` — fine on screen, but `raw` feeds
    // the Copy button, and "Copy example response" then wrote a block no JSON
    // parser accepts (task 0194's PR review). The per-venue volumes sum to
    // `volume_24h_usd` above, as the real response's do.
    key: 'sources',
    value: (
      <>
        {'{\n    '}
        <Tok c={KEY}>&quot;aquarius&quot;</Tok>: {'{ '}
        <Tok c={KEY}>&quot;price&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1774&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;volume_24h&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;277436.70&quot;</Tok>
        {' },\n    '}
        <Tok c={KEY}>&quot;sdex&quot;</Tok>: {'{ '}
        <Tok c={KEY}>&quot;price&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1773&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;volume_24h&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;98211.53&quot;</Tok>
        {' },\n    '}
        <Tok c={KEY}>&quot;soroswap&quot;</Tok>: {'{ '}
        <Tok c={KEY}>&quot;price&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1775&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;volume_24h&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;8088.17&quot;</Tok>
        {' }\n  }'}
      </>
    ),
    raw: [
      '{',
      '    "aquarius": { "price": "0.1774", "volume_24h": "277436.70" },',
      '    "sdex": { "price": "0.1773", "volume_24h": "98211.53" },',
      '    "soroswap": { "price": "0.1775", "volume_24h": "8088.17" }',
      '  }',
    ].join('\n'),
    dot: STR,
    meaning: 'Per-venue price and 24h volume: aquarius, sdex, soroswap',
  },
  {
    key: 'updated_at',
    value: <Tok c={STR}>&quot;2026-08-31T12:22:00Z&quot;</Tok>,
    raw: '"2026-08-31T12:22:00Z"',
    dot: STR,
    meaning: 'When this price was last computed (ISO 8601, UTC)',
  },
];

export const RESPONSE_TEXT = `{\n${RESPONSE_FIELDS.map(
  (f, i) =>
    `  "${f.key}": ${f.raw}${i < RESPONSE_FIELDS.length - 1 ? ',' : ''}`,
).join('\n')}\n}`;

/** The route list, each with the example response its row unfolds to. */
type Method = 'GET' | 'POST';

const ENDPOINTS: readonly {
  method: Method;
  path: string;
  summary: string;
  example: ReactNode;
}[] = [
  {
    method: 'GET',
    path: '/assets',
    summary: 'Assets, by volume',
    example: (
      <>
        <Tok c={MUTED}>
          {
            '// Paginated by cursor. ?type=classic|soroban ?search=USDC ?limit=200 ?min_volume_usd='
          }
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;data&quot;</Tok>: [{'{ '}
        <Tok c={KEY}>&quot;asset_code&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;USDC&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;issuer_address&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;GA5Z…KZVN&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price_usd&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;1.0002&quot;</Tok>, ... {'}],\n  '}
        <Tok c={KEY}>&quot;cursor&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;eyJ2…&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;has_more&quot;</Tok>: <Tok c={NUM}>true</Tok> {'}'}
      </>
    ),
  },
  {
    method: 'GET',
    path: '/assets/{id}',
    summary: 'One asset',
    example: (
      <>
        <Tok c={MUTED}>
          {
            '// GET /assets/USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
          }
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;asset&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;USDC:GA5Z…KZVN&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;asset_kind&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;credit&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;code&quot;</Tok>: <Tok c={STR}>&quot;USDC&quot;</Tok>
        , <Tok c={KEY}>&quot;issuer&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;GA5Z…KZVN&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;is_active&quot;</Tok>: <Tok c={NUM}>true</Tok> {'}'}
      </>
    ),
  },
  {
    method: 'GET',
    path: '/assets/{id}/price',
    summary: 'Price and 24h stats',
    example: (
      <>
        <Tok c={MUTED}>
          {'// The object described above. ?min_volume_usd= drops thin venues'}
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;asset&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;native&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price_usd&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1774&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;vwap_24h&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1773&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;sources&quot;</Tok>: {'{…}'}, ... {'}'}
      </>
    ),
  },
  {
    method: 'GET',
    path: '/assets/{id}/ohlcv',
    summary: 'Candles, 1m to 1M',
    example: (
      <>
        <Tok c={MUTED}>
          {
            '// ?timeframe=1h|24h|7d|30d|1y|all ?granularity=1m|15m|1h|4h|1d|1w|1M ?start= ?end= ?base_currency=USD|XLM'
          }
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;asset&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;native&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;granularity&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;15m&quot;</Tok>, <Tok c={KEY}>&quot;data&quot;</Tok>:
        [{'{ '}
        <Tok c={KEY}>&quot;timestamp&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;2026-08-30T12:30:00Z&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;open&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1806&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;high&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1807&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;low&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1801&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;close&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1806&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;volume_quote_usd&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;1491.51&quot;</Tok>, ... {'}, ...] }'}
      </>
    ),
  },
  {
    method: 'GET',
    path: '/oracles/{id}',
    summary: 'Oracle cross-check',
    example: (
      <>
        <Tok c={MUTED}>
          {'// What on-chain oracles say, next to the market'}
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;asset&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;native&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;oracles&quot;</Tok>: [{'{ '}
        <Tok c={KEY}>&quot;name&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;reflector&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price_usd&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1770&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;updated_at&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;2026-08-31T12:20:00Z&quot;</Tok> {'}] }'}
      </>
    ),
  },
  {
    method: 'GET',
    path: '/backfill/status',
    summary: 'History coverage',
    example: (
      <>
        <Tok c={MUTED}>
          {
            '// How far back candles go, per source — check before a long ?start='
          }
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;realtime_tip_ledger&quot;</Tok>:{' '}
        <Tok c={NUM}>63795749</Tok>, <Tok c={KEY}>&quot;sdex&quot;</Tok>: {'{ '}
        <Tok c={KEY}>&quot;status&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;completed&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;earliest_data_available&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;2015-11-18T03:47:00Z&quot;</Tok>, ... {'}, '}
        <Tok c={KEY}>&quot;soroban_amm&quot;</Tok>: {'{…} }'}
      </>
    ),
  },
  {
    method: 'POST',
    path: '/prices/batch',
    summary: 'Many prices at once',
    example: (
      <>
        <Tok c={MUTED}>
          {
            '// Body: { "assets": ["native", "USDC:GA5Z…KZVN", "C…"] } — the same object as /price, per asset'
          }
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;prices&quot;</Tok>: [{'{ '}
        <Tok c={KEY}>&quot;asset&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;native&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price_usd&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;0.1774&quot;</Tok>, ... {'}, ...] }'}
      </>
    ),
  },
];

const ERROR_CODES: readonly {
  status: number;
  tone: 'error' | 'warn' | 'muted';
  when: string;
  fix: string;
}[] = [
  {
    status: 400,
    tone: 'muted',
    when: 'Malformed identifier or query — body { "code": "invalid_id" | "invalid_query", "message": … }',
    fix: 'The message names the parameter. Identifiers are native, CODE:ISSUER (uppercase code, G… issuer) or a C… contract; limit is 1–200; timeframe is one of 1h, 24h, 7d, 30d, 1y, all.',
  },
  {
    status: 403,
    tone: 'error',
    when: 'Missing or invalid x-api-key header — body { "message": "Forbidden" }, from the gateway',
    fix: 'Check that your key is correct and the header name matches exactly. A 403 with "Missing Authentication Token" means the PATH is wrong, not the key.',
  },
  {
    status: 404,
    tone: 'muted',
    when: 'No such asset, or no price for it yet — body { "code": "not_found", "message": … }',
    fix: 'List /assets (with ?search=) to find the identifier; an asset with no recent trades has no current price.',
  },
  {
    status: 429,
    tone: 'warn',
    when: 'Rate limit exceeded (1 req/s) or monthly quota reached',
    // No `Retry-After`: API Gateway's throttle response carries none, and
    // telling a reader to wait for a header that never comes is worse than
    // no advice. Measured — see `RATE_LIMIT_BODY`.
    fix: 'Slow down to 1 request per second and retry after a short pause — the response carries no Retry-After header. Monitor quota on your dashboard.',
  },
  {
    status: 500,
    tone: 'muted',
    when: 'Server error — temporary issue on our side',
    fix: 'Retry with exponential backoff. Check the status page for incidents.',
  },
];

/**
 * The 429 as API Gateway actually sends it — **measured on 2026-08-27**
 * against the production `pricing-api-free` plan (1 req/s, burst 5) with
 * 120 concurrent requests: status `429`, `x-amzn-errortype:
 * TooManyRequestsException`, `content-type: application/json`, body exactly
 * `{"message":"Too Many Requests"}`, and no `Retry-After`.
 *
 * ⚠️ This page used to render an invented contract here — a `Retry-After: 1`
 * header and a `RATE_LIMIT_EXCEEDED` code — behind a copy button. Neither
 * string exists in `packages/` or `infra/`; throttling is the gateway's, not
 * ours, and the gateway's body is the one above. Task 0193's decision #3:
 * the design said only "what headers to watch", so the concrete contract is
 * decided here, from a measurement, and not from the frame.
 *
 * Not measured: the MONTHLY quota's 429, which API Gateway documents as
 * `{"message":"Limit Exceeded"}` — producing it means spending the plan's
 * 100 000 requests. The page does not show a body it has not seen.
 */
const RATE_LIMIT_BODY = `// HTTP 429 Too Many Requests\n// x-amzn-errortype: TooManyRequestsException\n{\n  "message": "Too Many Requests"\n}`;

const SDK_LANGS = [
  { key: 'js', label: 'JavaScript' },
  { key: 'python', label: 'Python' },
  { key: 'rust', label: 'Rust' },
  { key: 'go', label: 'Go' },
] as const;
type SdkLang = (typeof SDK_LANGS)[number]['key'];

const SDK: Record<SdkLang, Snippet> = {
  js: {
    text: `const API_KEY = "${PLACEHOLDER_KEY}";\nconst BASE = "${BASE_URL}";\n\nasync function getPrice() {\n  const res = await fetch(\`\${BASE}/assets/native/price\`, {\n    headers: { "x-api-key": API_KEY }\n  });\n  if (!res.ok) throw new Error(\`HTTP \${res.status}\`);\n  return res.json();\n}\n\nconst price = await getPrice();\nconsole.log(price);`,
    view: (
      <>
        <Tok c={KEY}>const</Tok> API_KEY ={' '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>;{'\n'}
        <Tok c={KEY}>const</Tok> BASE ={' '}
        <Tok c={STR}>&quot;{BASE_URL}&quot;</Tok>;{'\n\n'}
        <Tok c={KEY}>async function</Tok> getPrice() {'{\n  '}
        <Tok c={KEY}>const</Tok> res = <Tok c={KEY}>await</Tok> fetch(
        <Tok c={STR}>`$&#123;BASE&#125;/assets/native/price`</Tok>,{' '}
        {'{\n    headers: { '}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>: API_KEY {'}\n  });\n  '}
        <Tok c={KEY}>if</Tok> (!res.ok) <Tok c={KEY}>throw new</Tok> Error(
        <Tok c={STR}>`HTTP $&#123;res.status&#125;`</Tok>);{'\n  '}
        <Tok c={KEY}>return</Tok> res.json();{'\n}\n\n'}
        <Tok c={KEY}>const</Tok> price = <Tok c={KEY}>await</Tok> getPrice();
        {'\n'}
        console.log(price);
      </>
    ),
  },
  python: {
    text: `import requests\n\nAPI_KEY = "${PLACEHOLDER_KEY}"\nBASE = "${BASE_URL}"\n\n\ndef get_price():\n    res = requests.get(f"{BASE}/assets/native/price", headers={"x-api-key": API_KEY})\n    res.raise_for_status()\n    return res.json()\n\n\nprice = get_price()\nprint(price)`,
    view: (
      <>
        <Tok c={KEY}>import</Tok> requests{'\n\n'}API_KEY ={' '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>
        {'\n'}BASE = <Tok c={STR}>&quot;{BASE_URL}&quot;</Tok>
        {'\n\n\n'}
        <Tok c={KEY}>def</Tok> get_price():{'\n    res = requests.get('}
        <Tok c={STR}>f&quot;&#123;BASE&#125;/assets/native/price&quot;</Tok>,
        headers={'{'}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>: API_KEY
        {'})\n    res.raise_for_status()\n    '}
        <Tok c={KEY}>return</Tok> res.json(){'\n\n\n'}price = get_price()
        {'\n'}print(price)
      </>
    ),
  },
  rust: {
    text: `use reqwest::blocking::Client;\n\nconst API_KEY: &str = "${PLACEHOLDER_KEY}";\nconst BASE: &str = "${BASE_URL}";\n\nfn main() -> Result<(), reqwest::Error> {\n    let price: serde_json::Value = Client::new()\n        .get(format!("{BASE}/assets/native/price"))\n        .header("x-api-key", API_KEY)\n        .send()?\n        .error_for_status()?\n        .json()?;\n    println!("{price}");\n    Ok(())\n}`,
    view: (
      <>
        <Tok c={KEY}>use</Tok> reqwest::blocking::Client;{'\n\n'}
        <Tok c={KEY}>const</Tok> API_KEY: &amp;str ={' '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>;{'\n'}
        <Tok c={KEY}>const</Tok> BASE: &amp;str ={' '}
        <Tok c={STR}>&quot;{BASE_URL}&quot;</Tok>;{'\n\n'}
        <Tok c={KEY}>fn</Tok> main() -&gt; Result&lt;(), reqwest::Error&gt;{' '}
        {'{\n    '}
        <Tok c={KEY}>let</Tok> price: serde_json::Value = Client::new()
        {'\n        .get(format!('}
        <Tok c={STR}>&quot;&#123;BASE&#125;/assets/native/price&quot;</Tok>))
        {'\n        .header('}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>, API_KEY)
        {
          '\n        .send()?\n        .error_for_status()?\n        .json()?;\n    println!('
        }
        <Tok c={STR}>&quot;&#123;price&#125;&quot;</Tok>);{'\n    Ok(())\n}'}
      </>
    ),
  },
  go: {
    text: `package main\n\nimport (\n\t"fmt"\n\t"io"\n\t"net/http"\n)\n\nconst apiKey = "${PLACEHOLDER_KEY}"\nconst base = "${BASE_URL}"\n\nfunc main() {\n\treq, _ := http.NewRequest("GET", base+"/assets/native/price", nil)\n\treq.Header.Set("x-api-key", apiKey)\n\tres, err := http.DefaultClient.Do(req)\n\tif err != nil {\n\t\tpanic(err)\n\t}\n\tdefer res.Body.Close()\n\tbody, _ := io.ReadAll(res.Body)\n\tfmt.Println(string(body))\n}`,
    view: (
      <>
        <Tok c={KEY}>package</Tok> main{'\n\n'}
        <Tok c={KEY}>import</Tok> ({'\n\t'}
        <Tok c={STR}>&quot;fmt&quot;</Tok>
        {'\n\t'}
        <Tok c={STR}>&quot;io&quot;</Tok>
        {'\n\t'}
        <Tok c={STR}>&quot;net/http&quot;</Tok>
        {'\n)\n\n'}
        <Tok c={KEY}>const</Tok> apiKey ={' '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>
        {'\n'}
        <Tok c={KEY}>const</Tok> base ={' '}
        <Tok c={STR}>&quot;{BASE_URL}&quot;</Tok>
        {'\n\n'}
        <Tok c={KEY}>func</Tok> main() {'{\n\treq, _ := http.NewRequest('}
        <Tok c={STR}>&quot;GET&quot;</Tok>, base+
        <Tok c={STR}>&quot;/assets/native/price&quot;</Tok>, nil)
        {'\n\treq.Header.Set('}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>, apiKey)
        {'\n\tres, err := http.DefaultClient.Do(req)\n\t'}
        <Tok c={KEY}>if</Tok> err != nil {'{\n\t\tpanic(err)\n\t}\n\t'}
        <Tok c={KEY}>defer</Tok> res.Body.Close()
        {'\n\tbody, _ := io.ReadAll(res.Body)\n\tfmt.Println(string(body))\n}'}
      </>
    ),
  },
};

const SDK_TITLE: Record<SdkLang, string> = {
  js: 'javascript — fetch all prices',
  python: 'python — fetch all prices',
  rust: 'rust — fetch all prices',
  go: 'go — fetch all prices',
};

/**
 * Every `Snippet` on this page, for the drift test — see the note on `Snippet`.
 *
 * Exported for the spec alone. A new snippet table that is not listed here is
 * a snippet whose two halves nothing compares, so the list is the thing to
 * extend when one is added.
 */
export const SNIPPET_TABLES: Record<string, Record<string, Snippet>> = {
  FIRST_REQUEST,
  SDK,
};

/* -------------------------------------------------------------------------- */
/* Sections                                                                   */
/* -------------------------------------------------------------------------- */

function Prerequisites() {
  const steps: { title: string; body: ReactNode }[] = [
    {
      title: 'A Discord account',
      body: (
        <>
          API keys are issued via Discord OAuth. You need a Discord account that
          is a member of the{' '}
          <Link
            href={STELLAR_DISCORD_INVITE}
            target="_blank"
            rel="noreferrer"
            sx={{ color: 'inherit', textDecoration: 'underline' }}
          >
            Stellar Discord server
          </Link>
          .
        </>
      ),
    },
    {
      title: 'An API key',
      body: (
        <>
          Sign in at the{' '}
          <Link
            component={RouterLink}
            to={LOGIN_ROUTE}
            sx={{ color: 'inherit', textDecoration: 'underline' }}
          >
            developer portal
          </Link>{' '}
          to receive your key instantly. It will be shown on screen and always
          accessible from your dashboard.
        </>
      ),
    },
    {
      title: 'An HTTP client',
      body: "Any HTTP client works — curl, Postman, Insomnia, or your language's built-in fetch. All examples in this guide use curl.",
    },
  ];

  return (
    <Stack
      component="section"
      aria-labelledby="prerequisites-title"
      spacing={3}
      id="prerequisites"
      sx={{ scrollMarginTop: 80 }}
    >
      <Typography
        variant="h3"
        component="h2"
        id="prerequisites-title"
        color="text.primary"
      >
        Prerequisites
      </Typography>
      <Stack component="ol" spacing={3} sx={{ m: 0, p: 0, listStyle: 'none' }}>
        {steps.map(({ title, body }, i) => (
          <Stack component="li" key={title} direction="row" spacing={2}>
            {/* The numbered ring: `aria-hidden` because it is an `<ol>` and
                the list already counts for a screen reader. */}
            <Box
              aria-hidden
              sx={{
                flexShrink: 0,
                width: 44,
                height: 44,
                borderRadius: '50%',
                border: `1px solid ${color.primary[400]}`,
                display: 'grid',
                placeItems: 'center',
                fontFamily: font.mono,
                fontSize: '0.875rem',
                fontWeight: 500,
                color: color.primary[400],
              }}
            >
              {String(i + 1).padStart(2, '0')}
            </Box>
            <Stack spacing={0.75} sx={{ pt: 0.75 }}>
              <Typography variant="h5" component="h3" color="text.primary">
                {title}
              </Typography>
              <Typography variant="body1" sx={{ color: color.text.tertiary }}>
                {body}
              </Typography>
            </Stack>
          </Stack>
        ))}
      </Stack>
    </Stack>
  );
}

function Authentication() {
  const header = `x-api-key: ${PLACEHOLDER_KEY}`;
  return (
    <DocSection
      id="authentication"
      title="Authentication"
      lede="Every request must include your API key in the x-api-key header. There is no other authentication method."
    >
      <DocCard title="Required header">
        <Stack spacing={2} sx={{ p: 2 }}>
          <ValueStrip label="Header" value={header} copyLabel="header" />
          <Box
            sx={{
              display: 'grid',
              gap: 2,
              gridTemplateColumns: { xs: '1fr', md: '1fr 1fr' },
            }}
          >
            <Verdict tone="ok" label="Correct">
              -H &quot;x-api-key: sf_live_k8mN...&quot;
            </Verdict>
            <Verdict tone="bad" label="Wrong — returns 403">
              -H &quot;Authorization: Bearer sf_live...&quot;
            </Verdict>
          </Box>
        </Stack>
      </DocCard>
    </DocSection>
  );
}

/** The green "Correct" and red "Wrong" boxes under the required header. */
function Verdict({
  tone,
  label,
  children,
}: {
  tone: 'ok' | 'bad';
  label: string;
  children: ReactNode;
}) {
  const ok = tone === 'ok';
  return (
    <Stack
      spacing={1}
      sx={{
        p: 2,
        borderRadius: `${radius.md}px`,
        border: `1px solid ${ok ? color.green[400] : color.red[400]}`,
        backgroundColor: ok ? '#052e16' : '#3b0a0a',
      }}
    >
      <Typography
        variant="body2"
        sx={{ color: ok ? color.text.success : color.text.error }}
      >
        {label}
      </Typography>
      <Box
        component="code"
        sx={{
          fontFamily: font.mono,
          fontSize: '0.875rem',
          color: color.text.accent,
          overflowWrap: 'anywhere',
        }}
      >
        {children}
      </Box>
    </Stack>
  );
}

function BaseUrl() {
  return (
    <DocSection
      id="base-url"
      title="Base URL"
      lede="All API endpoints are relative to this base URL. The API is versioned — the current version is v1."
    >
      <ValueStrip label="Base URL" value={BASE_URL} copyLabel="base URL" />
    </DocSection>
  );
}

function FirstRequest() {
  return (
    <DocSection
      id="first-request"
      title="First request"
      lede="Fetch the current XLM price in USD. Replace the key with your own from the dashboard; the identifier is native, CODE:ISSUER or a contract address."
    >
      <SnippetTabs
        id="first-request-lang"
        options={FIRST_REQUEST_LANGS}
        snippets={FIRST_REQUEST}
        title={(k) => FIRST_REQUEST_TITLE[k]}
      />
    </DocSection>
  );
}

function Response() {
  return (
    <DocSection
      id="response"
      title="Understanding the response"
      lede="A successful request returns HTTP 200 with a JSON body. Here is what each field means."
    >
      <DocCard
        title="GET /v1/assets/native/price — 200 OK"
        copy={{ text: RESPONSE_TEXT, label: 'example response' }}
      >
        <Box
          sx={{
            display: 'grid',
            gridTemplateColumns: { xs: '1fr', md: '1fr 1fr' },
            gap: { xs: 0, md: 3 },
          }}
        >
          <Code>
            {RESPONSE_FIELDS.map((f, i) => (
              <Box component="span" key={f.key} sx={{ display: 'block' }}>
                <Tok c={KEY}>&quot;{f.key}&quot;</Tok>: {f.value}
                {i < RESPONSE_FIELDS.length - 1 ? ',' : ''}
              </Box>
            ))}
          </Code>
          <Stack
            component="ul"
            spacing={0.5}
            sx={{
              m: 0,
              p: 2,
              pt: { xs: 0, md: 2 },
              listStyle: 'none',
              fontFamily: font.mono,
              fontSize: '0.8125rem',
              lineHeight: 1.7,
              color: color.text.primary,
            }}
          >
            {RESPONSE_FIELDS.map((f) => (
              <Stack
                component="li"
                key={f.key}
                direction="row"
                spacing={1.25}
                alignItems="center"
              >
                <Box
                  aria-hidden
                  sx={{
                    flexShrink: 0,
                    width: 12,
                    height: 12,
                    borderRadius: '50%',
                    backgroundColor: f.dot,
                  }}
                />
                {/* The key is read out here so a screen reader gets "asset:
                    Asset pair identifier" rather than a dot and a sentence. */}
                <Box
                  component="span"
                  sx={{
                    position: 'absolute',
                    clip: 'rect(0 0 0 0)',
                    width: 1,
                    height: 1,
                    overflow: 'hidden',
                  }}
                >
                  {f.key}:
                </Box>
                <span>{f.meaning}</span>
              </Stack>
            ))}
          </Stack>
        </Box>
      </DocCard>
    </DocSection>
  );
}

/** The verb pill — the same chip `landing/Endpoints.tsx` draws. */
function MethodBadge({ method }: { method: Method }) {
  const post = method === 'POST';
  return (
    <Box
      component="span"
      sx={{
        flexShrink: 0,
        px: 1,
        py: 0.25,
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

function Endpoints() {
  // The first row open, as the frame draws it. One at a time: the rows are
  // a list to scan, and five open examples is a wall of JSON.
  const [open, setOpen] = useState<string | null>(ENDPOINTS[0].path);

  return (
    <DocSection
      id="endpoints"
      title="Endpoints"
      lede="All endpoints require the x-api-key header. Click any endpoint to see an example response."
    >
      <Stack spacing={1.5}>
        {ENDPOINTS.map(({ method, path, summary, example }) => {
          const expanded = open === path;
          const panelId = `endpoint-${path.replace(/[^a-z]+/gi, '-')}`;
          return (
            <Box
              key={path}
              sx={{
                borderRadius: `${radius.md}px`,
                border: panelBorder,
                backgroundColor: color.surface.grayAlt,
                overflow: 'hidden',
              }}
            >
              <Stack
                component="button"
                type="button"
                aria-expanded={expanded}
                aria-controls={panelId}
                onClick={() => setOpen(expanded ? null : path)}
                direction="row"
                alignItems="center"
                spacing={1.5}
                sx={{
                  width: '100%',
                  cursor: 'pointer',
                  border: 'none',
                  background: 'none',
                  color: color.text.primary,
                  px: 2,
                  py: 1.5,
                  textAlign: 'left',
                  '&:focus-visible': {
                    outline: `2px solid ${color.stroke.action}`,
                    outlineOffset: -2,
                  },
                }}
              >
                <MethodBadge method={method} />
                <Box
                  component="code"
                  sx={{
                    flex: 1,
                    minWidth: 0,
                    fontFamily: font.mono,
                    fontSize: '0.9375rem',
                    fontWeight: 700,
                    overflowWrap: 'anywhere',
                  }}
                >
                  {path}
                </Box>
                <Typography
                  variant="body1"
                  component="span"
                  sx={{
                    color: color.text.tertiary,
                    display: { xs: 'none', sm: 'inline' },
                  }}
                >
                  {summary}
                </Typography>
                <ChevronRightRoundedIcon
                  aria-hidden
                  sx={{
                    fontSize: 20,
                    color: color.text.tertiary,
                    transform: expanded ? 'rotate(90deg)' : 'none',
                    transition: 'transform 150ms',
                  }}
                />
              </Stack>
              {expanded && (
                <Box id={panelId}>
                  <Typography
                    variant="body2"
                    sx={{
                      px: 2,
                      color: color.text.tertiary,
                      display: { xs: 'block', sm: 'none' },
                    }}
                  >
                    {summary}
                  </Typography>
                  <Code sx={{ pt: 1 }}>{example}</Code>
                </Box>
              )}
            </Box>
          );
        })}
      </Stack>
    </DocSection>
  );
}

function Errors() {
  const statusColor = {
    error: color.text.error,
    warn: color.text.accent,
    muted: color.text.tertiary,
  };
  const cell = {
    px: 2,
    py: 1.5,
    textAlign: 'left',
    verticalAlign: 'top',
    fontFamily: font.secondary,
    fontSize: '0.875rem',
    lineHeight: 1.5,
    borderBottom: panelBorder,
  } as const;

  return (
    <DocSection
      id="errors"
      title="Error handling"
      lede="All errors return a JSON body with a code and message field."
    >
      <DocCard title="HTTP error codes">
        {/* A real table, scrolling inside its card at 375px rather than
            pushing the page sideways. */}
        <Box sx={{ overflowX: 'auto' }}>
          <Box
            component="table"
            sx={{
              width: '100%',
              minWidth: 560,
              borderCollapse: 'collapse',
              color: color.text.primary,
              '& th': {
                ...cell,
                fontWeight: 500,
                backgroundColor: color.surface.backgroundAlt,
              },
              '& td': cell,
              '& tr:last-child td': { borderBottom: 'none' },
            }}
          >
            <thead>
              <tr>
                <th scope="col">Status</th>
                <th scope="col">When</th>
                <th scope="col">How to fix</th>
              </tr>
            </thead>
            <tbody>
              {ERROR_CODES.map(({ status, tone, when, fix }) => (
                <tr key={status}>
                  <td>
                    <Box
                      component="code"
                      sx={{
                        fontFamily: font.mono,
                        color: statusColor[tone],
                      }}
                    >
                      {status}
                    </Box>
                  </td>
                  <td>{when}</td>
                  <td>{fix}</td>
                </tr>
              ))}
            </tbody>
          </Box>
        </Box>
      </DocCard>

      <Stack spacing={2}>
        <Typography variant="h5" component="h3" color="text.primary">
          429 response body
        </Typography>
        <DocCard
          title="JSON"
          copy={{ text: RATE_LIMIT_BODY, label: '429 response body' }}
        >
          <Code>
            <Tok c={MUTED}>{'// HTTP 429 Too Many Requests'}</Tok>
            {'\n'}
            <Tok c={MUTED}>{'// Retry-After: 1'}</Tok>
            {'\n{\n  '}
            <Tok c={KEY}>&quot;code&quot;</Tok>:{' '}
            <Tok c={STR}>&quot;RATE_LIMIT_EXCEEDED&quot;</Tok>,{'\n  '}
            <Tok c={KEY}>&quot;message&quot;</Tok>:{' '}
            <Tok c={STR}>
              &quot;Request rate limit exceeded. Retry after 1 second.&quot;
            </Tok>
            {'\n}'}
          </Code>
        </DocCard>
      </Stack>
    </DocSection>
  );
}

/**
 * Rate limits as figures. The per-second figure is the portal's
 * `rate_limit_per_second` when the config probe has answered; the frame's
 * "1" otherwise. The quota and the reset rule are task 0188's numbers.
 */
function RateLimits({ rateLimit }: { rateLimit?: number }) {
  const perSecond = rateLimit ?? 1;
  const figures = [
    {
      label: 'Rate limit',
      value: String(perSecond),
      unit: 'req / second',
      note: `${perSecond * 60} requests per minute`,
    },
    {
      label: 'Monthly quota',
      value: '100K',
      unit: 'requests / mo',
      note: 'Resets on the 1st of each month',
    },
    {
      label: 'Cost',
      value: '$0',
      unit: 'free tier',
      note: 'No credit card required',
    },
  ];

  return (
    <DocSection
      id="rate-limits"
      title="Rate limits"
      lede="Self-service keys have two limits enforced independently. Exceeding either returns HTTP 429."
    >
      <Box
        sx={{
          display: 'grid',
          gap: 2,
          gridTemplateColumns: { xs: '1fr', sm: 'repeat(3, 1fr)' },
        }}
      >
        {figures.map(({ label, value, unit, note }) => (
          <Stack
            key={label}
            spacing={1.5}
            sx={{
              p: 2,
              borderRadius: `${radius.lg}px`,
              border: panelBorder,
              backgroundColor: color.surface.grayAlt,
            }}
          >
            <Typography
              variant="overline"
              component="p"
              sx={{ color: color.text.tertiary }}
            >
              {label}
            </Typography>
            <Stack direction="row" spacing={1.5} alignItems="baseline">
              <Typography
                component="span"
                sx={{
                  fontFamily: font.primary,
                  fontWeight: 700,
                  fontSize: '2.25rem',
                  lineHeight: 1,
                  color: color.text.accent,
                }}
              >
                {value}
              </Typography>
              <Typography variant="body1" sx={{ color: color.text.primary }}>
                {unit}
              </Typography>
            </Stack>
            <Typography
              variant="overline"
              component="p"
              sx={{ color: color.text.tertiary }}
            >
              {note}
            </Typography>
          </Stack>
        ))}
      </Box>
      <Typography
        variant="body1"
        sx={{
          pt: 2,
          borderTop: cardBorder,
          color: color.text.tertiary,
        }}
      >
        {/* Underlined, as the frame draws it — but still NOT a link, for the
            reason the dashboard's Rate Limit card gives: there is no
            commercial-plans destination to point it at yet, and an underline
            that leads to a 404 is worse than one that leads nowhere. The
            underline is the design's emphasis; the `<a>` arrives with the
            address. */}
        Need higher limits?{' '}
        <Box
          component="span"
          sx={{
            color: color.text.accent,
            textDecoration: 'underline',
            textUnderlineOffset: '0.2em',
          }}
        >
          Contact us
        </Box>{' '}
        for commercial plans.
      </Typography>
    </DocSection>
  );
}

function Sdk() {
  return (
    <DocSection
      id="sdk"
      title="SDK examples"
      lede="No official SDK yet — the API is simple enough that any HTTP client works. Full examples in all four languages below."
    >
      <SnippetTabs
        id="sdk-lang"
        options={SDK_LANGS}
        snippets={SDK}
        title={(k) => SDK_TITLE[k]}
      />
    </DocSection>
  );
}

function WhatsNext() {
  const cards: {
    icon: string;
    title: string;
    body: string;
    to?: string;
    href?: string;
  }[] = [
    {
      icon: monitorUsageIcon,
      title: 'Monitor your usage',
      body: 'Track monthly requests against your quota and see daily request history on your dashboard.',
      to: DASHBOARD_ROUTE,
    },
    {
      icon: apiReferenceIcon,
      title: 'Full API reference',
      body: 'Explore every endpoint, parameter and response schema in the API reference.',
      href: API_REFERENCE,
    },
    {
      icon: higherLimitsIcon,
      title: 'Higher limits',
      body: 'Need more than 100K requests/month? Contact us for commercial plans — no in-app upgrade flow.',
    },
  ];

  return (
    <DocSection
      id="next"
      title="What's next"
      lede="Now that you have your first response, here is where to go from here."
    >
      <Box
        sx={{
          display: 'grid',
          gap: 2,
          gridTemplateColumns: { xs: '1fr', sm: 'repeat(3, 1fr)' },
        }}
      >
        {cards.map(({ icon, title, body, to, href }) => {
          const linkProps = to
            ? { component: RouterLink, to }
            : href
              ? { component: 'a', href }
              : {};
          return (
            <Stack
              key={title}
              {...linkProps}
              spacing={1.5}
              sx={{
                p: 2,
                borderRadius: `${radius.lg}px`,
                border: panelBorder,
                backgroundColor: color.surface.grayAlt,
                textDecoration: 'none',
                color: 'inherit',
                ...((to || href) && {
                  '&:hover': { borderColor: color.primary[400] },
                  '&:focus-visible': {
                    outline: `2px solid ${color.stroke.action}`,
                    outlineOffset: 2,
                  },
                }),
              }}
            >
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
              <Typography variant="body1" sx={{ color: color.text.tertiary }}>
                {body}
              </Typography>
            </Stack>
          );
        })}
      </Box>
    </DocSection>
  );
}

/* -------------------------------------------------------------------------- */
/* Table of contents                                                          */
/* -------------------------------------------------------------------------- */

/* -------------------------------------------------------------------------- */
/* Page                                                                       */
/* -------------------------------------------------------------------------- */

export function QuickStart({ rateLimit }: { rateLimit?: number }) {
  return (
    <DocPage
      sections={SECTIONS}
      title="Get your first response in under 5 minutes"
      lede="Follow this guide to authenticate, make your first API call and understand the response. No SDK required — a terminal and an API key are all you need."
    >
      <Prerequisites />
      <Authentication />
      <BaseUrl />
      <FirstRequest />
      <Response />
      <Endpoints />
      <Errors />
      <RateLimits rateLimit={rateLimit} />
      <Sdk />
      <WhatsNext />
    </DocPage>
  );
}
