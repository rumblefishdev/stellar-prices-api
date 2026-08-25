import AddRoundedIcon from '@mui/icons-material/AddRounded';
import BarChartRoundedIcon from '@mui/icons-material/BarChartRounded';
import CheckRoundedIcon from '@mui/icons-material/CheckRounded';
import ChevronRightRoundedIcon from '@mui/icons-material/ChevronRightRounded';
import ContentCopyRoundedIcon from '@mui/icons-material/ContentCopyRounded';
import DescriptionOutlinedIcon from '@mui/icons-material/DescriptionOutlined';
import Box from '@mui/material/Box';
import Container from '@mui/material/Container';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import { useEffect, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Link as RouterLink } from 'react-router-dom';

import { color, font, radius } from '../theme/tokens';
import {
  DASHBOARD_ROUTE,
  LOGIN_ROUTE,
  STELLAR_DISCORD_INVITE,
  SWAGGER_UI,
} from '../landing/links';
import { cardBorder } from '../landing/primitives';
import { panelBorder } from '../landing/DashboardPanel';

/**
 * The quick start (Figma `Quick start` frame, `918:644`): one long page that
 * gets a developer from a Discord account to a parsed response.
 *
 * **Everything on it is a still.** No request runs from this page and none
 * should — it is documentation, and a "try it" button that needs the visitor's
 * real key is what Swagger UI (task 0195) is for. Every snippet is a string
 * the visitor copies, which is why the one interactive thing here is the
 * copy button.
 *
 * The base URL and the paths are the DESIGN's, not this repo's OpenAPI
 * document's — the same gap `landing/Endpoints.tsx` notes. Reconciling them is
 * a product decision about the public surface; this page renders what the
 * frame says so that the two do not diverge into a third answer, and keeps
 * both in one constant each so the reconciliation is a two-line diff.
 */

const BASE_URL = 'https://api.soroswap.finance/v1';
const PLACEHOLDER_KEY = 'sf_live_YOUR_KEY_HERE';

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

type SectionId = (typeof SECTIONS)[number]['id'];

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
 */
type Snippet = { text: string; view: ReactNode };

/* -------------------------------------------------------------------------- */
/* Copy                                                                       */
/* -------------------------------------------------------------------------- */

/**
 * The frame's "Copy" pill: a yellow disc with the glyph, the word beside it,
 * on the darkest surface. Turns into "Copied" for two seconds, then back —
 * long enough to be seen, short enough that pressing it twice reads as two
 * copies rather than one stuck state.
 *
 * `navigator.clipboard` is absent on an insecure origin and in jsdom; a
 * missing API becomes a visible "Select and copy" rather than an exception
 * out of an event handler, and the text is still on screen to select.
 */
function CopyButton({ text, label }: { text: string; label: string }) {
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle');

  useEffect(() => {
    if (state === 'idle') return;
    const t = setTimeout(() => setState('idle'), 2000);
    return () => clearTimeout(t);
  }, [state]);

  const onClick = () => {
    const clipboard = navigator.clipboard;
    if (!clipboard) {
      setState('failed');
      return;
    }
    clipboard
      .writeText(text)
      .then(() => setState('copied'))
      .catch(() => setState('failed'));
  };

  const caption =
    state === 'copied'
      ? 'Copied'
      : state === 'failed'
        ? 'Select and copy'
        : 'Copy';

  return (
    <Stack
      component="button"
      type="button"
      onClick={onClick}
      aria-label={`Copy ${label}`}
      direction="row"
      spacing={1}
      alignItems="center"
      sx={{
        flexShrink: 0,
        cursor: 'pointer',
        border: 'none',
        borderRadius: `${radius.pill}px`,
        px: 1.5,
        py: 0.75,
        backgroundColor: color.surface.gray,
        color: color.text.primary,
        fontFamily: font.secondary,
        fontSize: '0.875rem',
        fontWeight: 500,
        '&:hover': { backgroundColor: alpha(color.stroke.default, 0.5) },
        '&:focus-visible': {
          outline: `2px solid ${color.stroke.action}`,
          outlineOffset: 2,
        },
      }}
    >
      <Box
        aria-hidden
        sx={{
          width: 20,
          height: 20,
          borderRadius: '50%',
          display: 'grid',
          placeItems: 'center',
          backgroundColor: color.primary[400],
          color: color.black,
        }}
      >
        {state === 'copied' ? (
          <CheckRoundedIcon sx={{ fontSize: 13 }} />
        ) : (
          <ContentCopyRoundedIcon sx={{ fontSize: 12 }} />
        )}
      </Box>
      {/* `aria-live` so "Copied" is announced without moving focus. */}
      <Box component="span" aria-live="polite">
        {caption}
      </Box>
    </Stack>
  );
}

/* -------------------------------------------------------------------------- */
/* Layout pieces                                                              */
/* -------------------------------------------------------------------------- */

/** A section heading with its one-line lede, anchored for the TOC. */
function SectionTitle({
  id,
  title,
  lede,
}: {
  id: SectionId;
  title: string;
  lede: ReactNode;
}) {
  return (
    <Stack spacing={1.5} sx={{ scrollMarginTop: 80 }} id={id}>
      <Typography
        variant="h3"
        component="h2"
        id={`${id}-title`}
        color="text.primary"
      >
        {title}
      </Typography>
      <Typography variant="body1" sx={{ color: color.text.secondary }}>
        {lede}
      </Typography>
    </Stack>
  );
}

/** The wrapper every section takes: title, then a 24px gap, then the body. */
function DocSection({
  id,
  title,
  lede,
  children,
}: {
  id: SectionId;
  title: string;
  lede: ReactNode;
  children: ReactNode;
}) {
  return (
    <Stack component="section" aria-labelledby={`${id}-title`} spacing={3}>
      <SectionTitle id={id} title={title} lede={lede} />
      {children}
    </Stack>
  );
}

/**
 * The frame's card: a title band on the darkest surface, a body one step
 * lighter, and optionally the copy button in the band's right corner. The
 * shape the dashboard's cards take, at the smaller title size the frame gives
 * "Required header" and "curl".
 */
function DocCard({
  title,
  copy,
  children,
  sx,
}: {
  title: ReactNode;
  copy?: { text: string; label: string };
  children: ReactNode;
  sx?: object;
}) {
  return (
    <Box
      sx={{
        borderRadius: `${radius.lg}px`,
        border: panelBorder,
        backgroundColor: color.surface.gray,
        overflow: 'hidden',
        ...sx,
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        spacing={2}
        sx={{
          px: 2,
          py: 1.5,
          backgroundColor: color.surface.grayAlt,
          borderBottom: panelBorder,
        }}
      >
        <Typography
          variant="h5"
          component="h3"
          color="text.primary"
          sx={{ minWidth: 0, overflowWrap: 'anywhere' }}
        >
          {title}
        </Typography>
        {copy && <CopyButton {...copy} />}
      </Stack>
      {children}
    </Box>
  );
}

/** A `<pre>` in the design's monospace, wrapping rather than widening at 375px. */
function Code({ children, sx }: { children: ReactNode; sx?: object }) {
  return (
    <Box
      component="pre"
      sx={{
        m: 0,
        p: 2,
        fontFamily: font.mono,
        fontSize: '0.8125rem',
        lineHeight: 1.7,
        color: PLAIN,
        whiteSpace: 'pre-wrap',
        overflowWrap: 'anywhere',
        ...sx,
      }}
    >
      <code>{children}</code>
    </Box>
  );
}

/**
 * A one-line value with a label and a copy button — the header and the base
 * URL. Its own shape rather than a `DocCard` with one line in it: the frame
 * draws these as a single dark strip, not a card with a band.
 */
function ValueStrip({
  label,
  value,
  copyLabel,
}: {
  label: string;
  value: string;
  copyLabel: string;
}) {
  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={2}
      sx={{
        p: 2,
        borderRadius: `${radius.md}px`,
        border: cardBorder,
        backgroundColor: color.surface.backgroundAlt,
      }}
    >
      <Typography
        variant="body1"
        sx={{ color: color.text.tertiary, flexShrink: 0 }}
      >
        {label}
      </Typography>
      <Box
        component="code"
        sx={{
          flex: 1,
          minWidth: 0,
          fontFamily: font.mono,
          fontSize: '0.9375rem',
          color: color.text.accent,
          overflowWrap: 'anywhere',
        }}
      >
        {value}
      </Box>
      <CopyButton text={value} label={copyLabel} />
    </Stack>
  );
}

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
    text: `curl ${BASE_URL}/prices/XLM-USDC \\\n  -H "x-api-key: ${PLACEHOLDER_KEY}"`,
    view: (
      <>
        <Tok c={MUTED}>curl </Tok>
        <Tok c={NUM}>{BASE_URL}/prices/XLM-USDC</Tok>
        {' \\\n  -H '}
        <Tok c={STR}>&quot;x-api-key: {PLACEHOLDER_KEY}&quot;</Tok>
      </>
    ),
  },
  js: {
    text: `const res = await fetch("${BASE_URL}/prices/XLM-USDC", {\n  headers: { "x-api-key": "${PLACEHOLDER_KEY}" }\n});\nconst price = await res.json();`,
    view: (
      <>
        <Tok c={KEY}>const</Tok> res = <Tok c={KEY}>await</Tok> fetch(
        <Tok c={STR}>&quot;{BASE_URL}/prices/XLM-USDC&quot;</Tok>
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
    text: `import requests\n\nres = requests.get(\n    "${BASE_URL}/prices/XLM-USDC",\n    headers={"x-api-key": "${PLACEHOLDER_KEY}"},\n)\nprice = res.json()`,
    view: (
      <>
        <Tok c={KEY}>import</Tok> requests{'\n\n'}res = requests.get(
        {'\n    '}
        <Tok c={STR}>&quot;{BASE_URL}/prices/XLM-USDC&quot;</Tok>
        {',\n    headers={'}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>
        {': '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>
        {'},\n)\nprice = res.json()'}
      </>
    ),
  },
  go: {
    text: `req, _ := http.NewRequest("GET", "${BASE_URL}/prices/XLM-USDC", nil)\nreq.Header.Set("x-api-key", "${PLACEHOLDER_KEY}")\nres, err := http.DefaultClient.Do(req)`,
    view: (
      <>
        req, _ := http.NewRequest(<Tok c={STR}>&quot;GET&quot;</Tok>,{' '}
        <Tok c={STR}>&quot;{BASE_URL}/prices/XLM-USDC&quot;</Tok>, nil){'\n'}
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

/** The 200 response, and what each field means — the two columns of the frame. */
const RESPONSE_FIELDS: readonly {
  key: string;
  value: ReactNode;
  raw: string;
  dot: string;
  meaning: string;
}[] = [
  {
    key: 'asset',
    value: <Tok c={STR}>&quot;XLM-USDC&quot;</Tok>,
    raw: '"XLM-USDC"',
    dot: KEY,
    meaning: 'Asset pair identifier',
  },
  {
    key: 'price',
    value: <Tok c={NUM}>0.0812</Tok>,
    raw: '0.0812',
    dot: NUM,
    meaning: 'Current price in quote asset (USDC)',
  },
  {
    key: 'price_usd',
    value: <Tok c={NUM}>0.0812</Tok>,
    raw: '0.0812',
    dot: NUM,
    meaning: 'Price expressed in USD',
  },
  {
    key: 'change_24h',
    value: <Tok c={NUM}>+2.14</Tok>,
    raw: '+2.14',
    dot: NUM,
    meaning: '% change over last 24 hours',
  },
  {
    key: 'volume_24h',
    value: <Tok c={NUM}>142891.50</Tok>,
    raw: '142891.50',
    dot: NUM,
    meaning: '24h trading volume in USD',
  },
  {
    key: 'liquidity',
    value: <Tok c={NUM}>2400000</Tok>,
    raw: '2400000',
    dot: NUM,
    meaning: 'Total pool liquidity in USD',
  },
  {
    key: 'source',
    value: <Tok c={STR}>&quot;soroswap&quot;</Tok>,
    raw: '"soroswap"',
    dot: STR,
    meaning: 'Always "soroswap" — data source',
  },
  {
    key: 'updated_at',
    value: <Tok c={STR}>&quot;2026-04-13T14:23:51Z&quot;</Tok>,
    raw: '"2026-04-13T14:23:51Z"',
    dot: STR,
    meaning: 'ISO 8601 timestamp of last update',
  },
];

const RESPONSE_TEXT = `{\n${RESPONSE_FIELDS.map(
  (f, i) =>
    `  "${f.key}": ${f.raw}${i < RESPONSE_FIELDS.length - 1 ? ',' : ''}`,
).join('\n')}\n}`;

/** The route list, each with the example response its row unfolds to. */
const ENDPOINTS: readonly {
  path: string;
  summary: string;
  example: ReactNode;
}[] = [
  {
    path: '/prices',
    summary: 'All asset prices',
    example: (
      <>
        <Tok c={MUTED}>{'// Returns array of all available asset prices'}</Tok>
        {'\n[{ '}
        <Tok c={KEY}>&quot;asset&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;XLM-USDC&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price&quot;</Tok>: <Tok c={NUM}>0.0812</Tok>, ...{' '}
        {'},\n { '}
        <Tok c={KEY}>&quot;asset&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;XLM-EURC&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price&quot;</Tok>: <Tok c={NUM}>0.0751</Tok>, ...{' '}
        {'}]'}
      </>
    ),
  },
  {
    path: '/prices/{asset}',
    summary: 'Single asset',
    example: (
      <>
        <Tok c={MUTED}>
          {'// Returns one asset — the object described above'}
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;asset&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;XLM-USDC&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price&quot;</Tok>: <Tok c={NUM}>0.0812</Tok>,{' '}
        <Tok c={KEY}>&quot;change_24h&quot;</Tok>: <Tok c={NUM}>+2.14</Tok>, ...{' '}
        {'}'}
      </>
    ),
  },
  {
    path: '/pools',
    summary: 'Liquidity pools',
    example: (
      <>
        <Tok c={MUTED}>{'// Returns array of liquidity pools'}</Tok>
        {'\n[{ '}
        <Tok c={KEY}>&quot;id&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;CB7X…Q4A&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;pair&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;XLM-USDC&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;liquidity&quot;</Tok>: <Tok c={NUM}>2400000</Tok>,
        ... {'}]'}
      </>
    ),
  },
  {
    path: '/pools/{id}/stats',
    summary: 'Pool statistics',
    example: (
      <>
        <Tok c={MUTED}>
          {'// Returns volume and fee statistics for one pool'}
        </Tok>
        {'\n{ '}
        <Tok c={KEY}>&quot;id&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;CB7X…Q4A&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;volume_24h&quot;</Tok>: <Tok c={NUM}>142891.50</Tok>,{' '}
        <Tok c={KEY}>&quot;fees_24h&quot;</Tok>: <Tok c={NUM}>428.67</Tok>, ...{' '}
        {'}'}
      </>
    ),
  },
  {
    path: '/history/{asset}',
    summary: 'Historical prices',
    example: (
      <>
        <Tok c={MUTED}>{'// Returns array of price points, oldest first'}</Tok>
        {'\n[{ '}
        <Tok c={KEY}>&quot;t&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;2026-04-13T14:00:00Z&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price&quot;</Tok>: <Tok c={NUM}>0.0809</Tok>{' '}
        {'},\n { '}
        <Tok c={KEY}>&quot;t&quot;</Tok>:{' '}
        <Tok c={STR}>&quot;2026-04-13T15:00:00Z&quot;</Tok>,{' '}
        <Tok c={KEY}>&quot;price&quot;</Tok>: <Tok c={NUM}>0.0812</Tok> {'}]'}
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
    status: 403,
    tone: 'error',
    when: 'Missing or invalid x-api-key header',
    fix: 'Check that your key is correct and the header name matches exactly.',
  },
  {
    status: 429,
    tone: 'warn',
    when: 'Rate limit exceeded (1 req/s) or monthly quota reached',
    fix: 'Slow down requests. Check the Retry-After header for the wait time. Monitor quota on your dashboard.',
  },
  {
    status: 404,
    tone: 'muted',
    when: 'Asset pair or pool ID not found',
    fix: 'Check the asset identifier format — use uppercase, e.g. XLM-USDC.',
  },
  {
    status: 500,
    tone: 'muted',
    when: 'Server error — temporary issue on our side',
    fix: 'Retry with exponential backoff. Check the status page for incidents.',
  },
];

const RATE_LIMIT_BODY = `// HTTP 429 Too Many Requests\n// Retry-After: 1\n{\n  "code": "RATE_LIMIT_EXCEEDED",\n  "message": "Request rate limit exceeded. Retry after 1 second."\n}`;

const SDK_LANGS = [
  { key: 'js', label: 'JavaScript' },
  { key: 'python', label: 'Python' },
  { key: 'rust', label: 'Rust' },
  { key: 'go', label: 'Go' },
] as const;
type SdkLang = (typeof SDK_LANGS)[number]['key'];

const SDK: Record<SdkLang, Snippet> = {
  js: {
    text: `const API_KEY = "${PLACEHOLDER_KEY}";\nconst BASE = "${BASE_URL}";\n\nasync function getPrices() {\n  const res = await fetch(\`\${BASE}/prices\`, {\n    headers: { "x-api-key": API_KEY }\n  });\n  if (!res.ok) throw new Error(\`HTTP \${res.status}\`);\n  return res.json();\n}\n\nconst prices = await getPrices();\nconsole.log(prices);`,
    view: (
      <>
        <Tok c={KEY}>const</Tok> API_KEY ={' '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>;{'\n'}
        <Tok c={KEY}>const</Tok> BASE ={' '}
        <Tok c={STR}>&quot;{BASE_URL}&quot;</Tok>;{'\n\n'}
        <Tok c={KEY}>async function</Tok> getPrices() {'{\n  '}
        <Tok c={KEY}>const</Tok> res = <Tok c={KEY}>await</Tok> fetch(
        <Tok c={STR}>`$&#123;BASE&#125;/prices`</Tok>, {'{\n    headers: { '}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>: API_KEY {'}\n  });\n  '}
        <Tok c={KEY}>if</Tok> (!res.ok) <Tok c={KEY}>throw new</Tok> Error(
        <Tok c={STR}>`HTTP $&#123;res.status&#125;`</Tok>);{'\n  '}
        <Tok c={KEY}>return</Tok> res.json();{'\n}\n\n'}
        <Tok c={KEY}>const</Tok> prices = <Tok c={KEY}>await</Tok> getPrices();
        {'\n'}
        console.log(prices);
      </>
    ),
  },
  python: {
    text: `import requests\n\nAPI_KEY = "${PLACEHOLDER_KEY}"\nBASE = "${BASE_URL}"\n\n\ndef get_prices():\n    res = requests.get(f"{BASE}/prices", headers={"x-api-key": API_KEY})\n    res.raise_for_status()\n    return res.json()\n\n\nprices = get_prices()\nprint(prices)`,
    view: (
      <>
        <Tok c={KEY}>import</Tok> requests{'\n\n'}API_KEY ={' '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>
        {'\n'}BASE = <Tok c={STR}>&quot;{BASE_URL}&quot;</Tok>
        {'\n\n\n'}
        <Tok c={KEY}>def</Tok> get_prices():{'\n    res = requests.get('}
        <Tok c={STR}>f&quot;&#123;BASE&#125;/prices&quot;</Tok>, headers={'{'}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>: API_KEY
        {'})\n    res.raise_for_status()\n    '}
        <Tok c={KEY}>return</Tok> res.json(){'\n\n\n'}prices = get_prices()
        {'\n'}print(prices)
      </>
    ),
  },
  rust: {
    text: `use reqwest::blocking::Client;\n\nconst API_KEY: &str = "${PLACEHOLDER_KEY}";\nconst BASE: &str = "${BASE_URL}";\n\nfn main() -> Result<(), reqwest::Error> {\n    let prices: serde_json::Value = Client::new()\n        .get(format!("{BASE}/prices"))\n        .header("x-api-key", API_KEY)\n        .send()?\n        .error_for_status()?\n        .json()?;\n    println!("{prices}");\n    Ok(())\n}`,
    view: (
      <>
        <Tok c={KEY}>use</Tok> reqwest::blocking::Client;{'\n\n'}
        <Tok c={KEY}>const</Tok> API_KEY: &amp;str ={' '}
        <Tok c={STR}>&quot;{PLACEHOLDER_KEY}&quot;</Tok>;{'\n'}
        <Tok c={KEY}>const</Tok> BASE: &amp;str ={' '}
        <Tok c={STR}>&quot;{BASE_URL}&quot;</Tok>;{'\n\n'}
        <Tok c={KEY}>fn</Tok> main() -&gt; Result&lt;(), reqwest::Error&gt;{' '}
        {'{\n    '}
        <Tok c={KEY}>let</Tok> prices: serde_json::Value = Client::new()
        {'\n        .get(format!('}
        <Tok c={STR}>&quot;&#123;BASE&#125;/prices&quot;</Tok>))
        {'\n        .header('}
        <Tok c={STR}>&quot;x-api-key&quot;</Tok>, API_KEY)
        {
          '\n        .send()?\n        .error_for_status()?\n        .json()?;\n    println!('
        }
        <Tok c={STR}>&quot;&#123;prices&#125;&quot;</Tok>);{'\n    Ok(())\n}'}
      </>
    ),
  },
  go: {
    text: `package main\n\nimport (\n\t"fmt"\n\t"io"\n\t"net/http"\n)\n\nconst apiKey = "${PLACEHOLDER_KEY}"\nconst base = "${BASE_URL}"\n\nfunc main() {\n\treq, _ := http.NewRequest("GET", base+"/prices", nil)\n\treq.Header.Set("x-api-key", apiKey)\n\tres, err := http.DefaultClient.Do(req)\n\tif err != nil {\n\t\tpanic(err)\n\t}\n\tdefer res.Body.Close()\n\tbody, _ := io.ReadAll(res.Body)\n\tfmt.Println(string(body))\n}`,
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
        <Tok c={STR}>&quot;/prices&quot;</Tok>, nil){'\n\treq.Header.Set('}
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
      lede="Fetch the current XLM/USDC price. Replace the key with your own from the dashboard."
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
        title="GET /v1/prices/XLM-USDC — 200 OK"
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

/** The `Get` pill — the same chip `landing/Endpoints.tsx` draws. */
function MethodBadge() {
  return (
    <Box
      component="span"
      sx={{
        flexShrink: 0,
        px: 1,
        py: 0.25,
        borderRadius: `${radius.chip}px`,
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
        {ENDPOINTS.map(({ path, summary, example }) => {
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
                <MethodBadge />
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
        {/* "Contact us" is yellow but not a link, for the reason the
            dashboard's Rate Limit card gives: there is no commercial-plans
            destination to point it at yet. */}
        Need higher limits?{' '}
        <Box component="span" sx={{ color: color.text.accent }}>
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
    icon: typeof AddRoundedIcon;
    title: string;
    body: string;
    to?: string;
    href?: string;
  }[] = [
    {
      icon: BarChartRoundedIcon,
      title: 'Monitor your usage',
      body: 'Track monthly requests against your quota and see daily request history on your dashboard.',
      to: DASHBOARD_ROUTE,
    },
    {
      icon: DescriptionOutlinedIcon,
      title: 'Full API reference',
      body: 'Explore all endpoints, parameters and response schemas in the interactive Swagger UI.',
      href: SWAGGER_UI,
    },
    {
      icon: AddRoundedIcon,
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
        {cards.map(({ icon: Icon, title, body, to, href }) => {
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
                aria-hidden
                sx={{
                  width: 32,
                  height: 32,
                  borderRadius: '50%',
                  display: 'grid',
                  placeItems: 'center',
                  backgroundColor: color.primary[100],
                  color: color.primary[950],
                }}
              >
                <Icon sx={{ fontSize: 18 }} />
              </Box>
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

/**
 * The left rail. Sticky from `md` up; on a phone it is a horizontal row of
 * chips above the title, because a vertical list of ten links at 375px is a
 * screen of navigation before any content.
 *
 * Which entry is current comes from an `IntersectionObserver` on the section
 * headings — the frame underlines "Prerequisites" because that is where the
 * page opens, not because it is a fixed choice.
 */
function Toc() {
  const [current, setCurrent] = useState<SectionId>(SECTIONS[0].id);

  useEffect(() => {
    if (typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver(
      (entries) => {
        const hit = entries
          .filter((e) => e.isIntersecting)
          .sort(
            (a, b) => a.boundingClientRect.top - b.boundingClientRect.top,
          )[0];
        if (hit) setCurrent(hit.target.id as SectionId);
      },
      { rootMargin: '-80px 0px -70% 0px' },
    );
    for (const { id } of SECTIONS) {
      const el = document.getElementById(id);
      if (el) observer.observe(el);
    }
    return () => observer.disconnect();
  }, []);

  return (
    <Box
      component="nav"
      aria-label="On this page"
      sx={{
        position: { md: 'sticky' },
        top: { md: 80 },
        alignSelf: 'flex-start',
        width: { xs: 'auto', md: 220 },
        flexShrink: 0,
        // The phone rail bleeds to the viewport edge, like `CardRail`.
        mx: { xs: -2.5, md: 0 },
        px: { xs: 2.5, md: 0 },
        overflowX: { xs: 'auto', md: 'visible' },
        scrollbarWidth: 'none',
        '&::-webkit-scrollbar': { display: 'none' },
      }}
    >
      <Stack
        component="ul"
        direction={{ xs: 'row', md: 'column' }}
        spacing={{ xs: 1, md: 0 }}
        sx={{ m: 0, p: 0, listStyle: 'none' }}
      >
        {SECTIONS.map(({ id, label }) => {
          const active = id === current;
          return (
            <li key={id}>
              <Link
                href={`#${id}`}
                aria-current={active ? 'location' : undefined}
                sx={{
                  display: 'block',
                  whiteSpace: 'nowrap',
                  py: { xs: 0.75, md: 1 },
                  px: { xs: 1.5, md: 0 },
                  borderRadius: { xs: `${radius.pill}px`, md: 0 },
                  border: { xs: cardBorder, md: 'none' },
                  borderBottom: {
                    md: `1px solid ${active ? color.text.primary : 'transparent'}`,
                  },
                  fontFamily: font.secondary,
                  fontSize: '0.875rem',
                  fontWeight: 500,
                  textDecoration: 'none',
                  color: active ? color.text.primary : color.text.tertiary,
                  backgroundColor: {
                    xs: active ? color.surface.gray : 'transparent',
                    md: 'transparent',
                  },
                  '&:hover': { color: color.text.primary },
                }}
              >
                {label}
              </Link>
            </li>
          );
        })}
      </Stack>
    </Box>
  );
}

/* -------------------------------------------------------------------------- */
/* Page                                                                       */
/* -------------------------------------------------------------------------- */

export function QuickStart({ rateLimit }: { rateLimit?: number }) {
  return (
    <Box
      component="main"
      sx={{
        position: 'relative',
        overflow: 'hidden',
        backgroundColor: color.surface.background,
        minHeight: 'calc(100dvh - 52px)',
        py: { xs: 5, md: 10 },
      }}
    >
      {/* The dashboard's rule grid and glow, strongest behind the title. */}
      <Box
        aria-hidden
        sx={{
          position: 'absolute',
          inset: 0,
          pointerEvents: 'none',
          backgroundImage: `
            radial-gradient(40% 30% at 22% 8%, ${alpha(color.primary[400], 0.12)} 0%, transparent 70%),
            linear-gradient(${alpha(color.stroke.default, 0.34)} 1px, transparent 1px),
            linear-gradient(90deg, ${alpha(color.stroke.default, 0.34)} 1px, transparent 1px)`,
          backgroundSize: '100% 100%, 80px 80px, 80px 80px',
          maskImage: 'linear-gradient(#000 0%, #000 600px, transparent 1400px)',
        }}
      />
      <Container sx={{ position: 'relative' }}>
        <Stack
          direction={{ xs: 'column', md: 'row' }}
          spacing={{ xs: 3, md: 10 }}
          alignItems="flex-start"
        >
          <Toc />
          <Stack spacing={{ xs: 8, md: 12 }} sx={{ minWidth: 0, flex: 1 }}>
            <Stack spacing={2} sx={{ maxWidth: 760 }}>
              <Typography variant="h2" component="h1" color="text.primary">
                Get your first response in under 5 minutes
              </Typography>
              <Typography
                variant="subtitle2"
                sx={{ color: color.text.secondary }}
              >
                Follow this guide to authenticate, make your first API call and
                understand the response. No SDK required — a terminal and an API
                key are all you need.
              </Typography>
            </Stack>
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
          </Stack>
        </Stack>
      </Container>
    </Box>
  );
}
