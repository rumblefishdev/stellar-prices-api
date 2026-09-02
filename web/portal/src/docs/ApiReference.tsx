import ChevronRightRoundedIcon from '@mui/icons-material/ChevronRightRounded';
import LockOutlinedIcon from '@mui/icons-material/LockOutlined';
import Box from '@mui/material/Box';
import Button from '@mui/material/Button';
import CircularProgress from '@mui/material/CircularProgress';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';

import { panelBorder } from '../landing/DashboardPanel';
import {
  Code,
  DocCard,
  DocPage,
  DocSection,
  ValueStrip,
  type TocEntry,
} from '../landing/DocPrimitives';
import { OPENAPI_JSON } from '../landing/links';
import { cardBorder } from '../landing/primitives';
import { color, font, radius } from '../theme/tokens';
import {
  apiKeyHeader,
  deref,
  describe,
  displaySummary,
  exampleOf,
  scrubInternal,
  groupByTag,
  linkedComponent,
  primaryMedia,
  requiresKey,
  schemaId,
  statusTone,
  tagId,
  typeLabel,
  type Method,
  type OpenApiDocument,
  type Operation,
  type OperationEntry,
  type Parameter,
  type Schema,
  type TagGroup,
} from './openapi';

/**
 * The API reference (task 0195): the live OpenAPI document, rendered in the
 * portal's own design system in the shape Swagger UI gave the world.
 *
 * **A viewer over the served document, never a copy.** The page fetches
 * {@link OPENAPI_JSON} when it opens, so what a reader sees is what the
 * deployed handler describes — the drift task 0124 spent a task preventing.
 * On the shared host that fetch is cross-origin (the page on
 * `sorobanscan.rumblefish.dev`, the document on the API's own hostname),
 * which is why `GET /api-docs-json` answers `Access-Control-Allow-Origin: *`
 * (`packages/prices-api/src/lib.rs`).
 *
 * **Swagger's shape, the portal's clothes.** Operations grouped under their
 * tags, one collapsible row per operation with the method badge, the path
 * and the summary, parameters and responses inside, the schemas in their own
 * accordion at the foot — the layout every developer already knows how to
 * read. Drawn with the quick start's pieces (`landing/DocPrimitives.tsx`, off
 * the Figma `Quick start` frame) rather than with `swagger-ui-react`, which
 * this page used for a day: its stylesheet is written for a white page, its
 * layout does not bend to another, and with "Try it out" off it still
 * renders every parameter as a disabled form. That last one read as a broken
 * dropdown, which it was not, and could not be styled into anything else.
 *
 * **Nothing here sends a request.** The data routes answer no CORS yet
 * (task 0126), so a "try it" button would fail on preflight and read as a
 * broken API. Examples are built from the document's schemas and `example`s;
 * the quick start is where a reader copies a real request from.
 */

/* -------------------------------------------------------------------------- */
/* Loading the document                                                       */
/* -------------------------------------------------------------------------- */

type Loaded =
  | { state: 'loading' }
  | { state: 'ok'; doc: OpenApiDocument }
  | { state: 'error'; message: string };

/**
 * Fetch the document. `accept` alone — no credentials and no custom header,
 * so the request is a simple one and needs no preflight even cross-origin.
 */
export function useOpenApi(url: string): {
  loaded: Loaded;
  reload: () => void;
} {
  const [loaded, setLoaded] = useState<Loaded>({ state: 'loading' });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoaded({ state: 'loading' });
    fetch(url, { headers: { accept: 'application/json' } })
      .then(async (response) => {
        if (!response.ok) {
          throw new Error(`${url} answered ${response.status}`);
        }
        const doc = (await response.json()) as OpenApiDocument;
        if (!doc || typeof doc !== 'object' || !doc.paths) {
          throw new Error(`${url} did not return an OpenAPI document`);
        }
        return doc;
      })
      .then(
        (doc) => {
          if (!cancelled) setLoaded({ state: 'ok', doc });
        },
        (error: unknown) => {
          if (!cancelled) {
            setLoaded({
              state: 'error',
              message:
                error instanceof Error
                  ? error.message
                  : `${url} could not be loaded`,
            });
          }
        },
      );
    return () => {
      cancelled = true;
    };
  }, [url, attempt]);

  const reload = useCallback(() => setAttempt((n) => n + 1), []);
  return { loaded, reload };
}

/* -------------------------------------------------------------------------- */
/* Small pieces                                                               */
/* -------------------------------------------------------------------------- */

const KEY = color.accent.violet[400];
const STR = color.accent.emerald[400];
const NUM = color.primary[400];

/**
 * The document's descriptions are Markdown by the OpenAPI spec and, from
 * `utoipa`, are the handler's doc comments — inline code in backticks,
 * `**emphasis**`, the odd `* ` bullet list. That much is rendered; a full
 * Markdown engine for three constructs would be a dependency that outweighs
 * the text it formats. Anything else is shown as written.
 */
function Prose({ text, sx }: { text: string | undefined; sx?: object }) {
  if (!text) return null;
  const blocks = scrubInternal(text).split(/\n{2,}/);
  return (
    <Stack spacing={1} sx={sx}>
      {blocks.map((block, i) => {
        const items = listItems(block);
        if (items) {
          return (
            <Box component="ul" key={i} sx={{ m: 0, pl: 2.5 }}>
              {items.map((item, j) => (
                <li key={j}>{inline(item)}</li>
              ))}
            </Box>
          );
        }
        return (
          <p key={i} style={{ margin: 0 }}>
            {inline(block.split('\n').join(' '))}
          </p>
        );
      })}
    </Stack>
  );
}

/**
 * The items of a bullet-list block, or `undefined` when the block is prose.
 * A block is a list when its first line is a bullet; later lines that are
 * not bullets are the previous item wrapped, the way rustdoc wraps at 80.
 */
function listItems(block: string): string[] | undefined {
  const bullet = /^\s*[*-]\s+/;
  const lines = block.split('\n');
  if (!bullet.test(lines[0])) return undefined;
  const items: string[] = [];
  for (const line of lines) {
    if (bullet.test(line)) items.push(line.replace(bullet, ''));
    else items[items.length - 1] += ` ${line.trim()}`;
  }
  return items;
}

/** Backticks become `code`, double asterisks become `strong`. */
function inline(text: string): ReactNode {
  const parts = scrubInternal(text).split(/(`[^`]+`|\*\*[^*]+\*\*)/g);
  return parts.map((part, i) => {
    if (part.startsWith('`') && part.endsWith('`')) {
      return (
        <Box
          component="code"
          key={i}
          sx={{
            fontFamily: font.mono,
            fontSize: '0.85em',
            color: color.text.accent,
            backgroundColor: alpha(color.white, 0.05),
            borderRadius: `${radius.chip / 2}px`,
            px: 0.5,
          }}
        >
          {part.slice(1, -1)}
        </Box>
      );
    }
    if (part.startsWith('**') && part.endsWith('**')) {
      return <strong key={i}>{part.slice(2, -2)}</strong>;
    }
    return <Fragment key={i}>{part}</Fragment>;
  });
}

/** Pretty-printed JSON in the quick start's three colours. */
function JsonView({ value }: { value: unknown }) {
  const text = JSON.stringify(value, null, 2) ?? 'null';
  const tokens = text.split(
    /("(?:\\.|[^"\\])*"\s*:|"(?:\\.|[^"\\])*"|\b(?:true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g,
  );
  return (
    <>
      {tokens.map((token, i) => {
        if (!token) return null;
        let c: string | undefined;
        if (/^".*"\s*:$/.test(token)) c = KEY;
        else if (token.startsWith('"')) c = STR;
        else if (/^(true|false|null)$/.test(token)) c = color.text.tertiary;
        else if (/^-?\d/.test(token)) c = NUM;
        return c ? (
          <Box component="span" key={i} sx={{ color: c }}>
            {token}
          </Box>
        ) : (
          <Fragment key={i}>{token}</Fragment>
        );
      })}
    </>
  );
}

/** A DocCard holding an example document, with its copy button. */
function ExampleCard({
  title,
  value,
  label,
}: {
  title: ReactNode;
  value: unknown;
  label: string;
}) {
  const text = JSON.stringify(value, null, 2) ?? 'null';
  return (
    <DocCard title={title} copy={{ text, label }}>
      <Code>
        <JsonView value={value} />
      </Code>
    </DocCard>
  );
}

/**
 * The method badge, in the palette the quick start already gave GET and
 * POST — the convention (blue GET, green POST, red DELETE) is Swagger's, the
 * colours are the design's.
 */
const METHOD_STYLE: Record<Method, { bg: string; fg: string }> = {
  get: { bg: color.accent.emerald[100], fg: color.accent.emerald[900] },
  post: { bg: color.primary[100], fg: color.primary[900] },
  put: { bg: color.accent.violet[100], fg: color.accent.violet[900] },
  patch: { bg: color.accent.violet[100], fg: color.accent.violet[900] },
  delete: { bg: color.red[100], fg: color.red[950] },
  head: { bg: color.gray[50], fg: color.gray[900] },
  options: { bg: color.gray[50], fg: color.gray[900] },
  trace: { bg: color.gray[50], fg: color.gray[900] },
};

function MethodBadge({ method }: { method: Method }) {
  const { bg, fg } = METHOD_STYLE[method];
  return (
    <Box
      component="span"
      sx={{
        flexShrink: 0,
        minWidth: 56,
        textAlign: 'center',
        px: 1,
        py: 0.25,
        borderRadius: `${radius.chip}px`,
        backgroundColor: bg,
        color: fg,
        fontFamily: font.secondary,
        fontWeight: 700,
        fontSize: '0.75rem',
        letterSpacing: '0.02em',
      }}
    >
      {method.toUpperCase()}
    </Box>
  );
}

/** A small mono chip — an enum member, a parameter's location, a status. */
function Chip({
  children,
  tone = 'muted',
}: {
  children: ReactNode;
  tone?: 'muted' | 'success' | 'warn' | 'error' | 'accent';
}) {
  const palette = {
    muted: { bg: alpha(color.white, 0.06), fg: color.text.secondary },
    success: { bg: color.surface.success, fg: color.text.success },
    warn: { bg: color.primary[950], fg: color.primary[300] },
    error: { bg: color.red[950], fg: color.red[300] },
    accent: { bg: alpha(color.primary[400], 0.15), fg: color.text.accent },
  }[tone];
  return (
    <Box
      component="span"
      sx={{
        display: 'inline-block',
        px: 0.75,
        py: 0.125,
        borderRadius: `${radius.chip}px`,
        backgroundColor: palette.bg,
        color: palette.fg,
        fontFamily: font.mono,
        fontSize: '0.75rem',
        lineHeight: 1.6,
        whiteSpace: 'nowrap',
      }}
    >
      {children}
    </Box>
  );
}

/** A type label, linked to its schema row when it names one. */
function TypeRef({
  doc,
  schema,
}: {
  doc: OpenApiDocument;
  schema: Schema | undefined;
}) {
  const label = typeLabel(doc, schema);
  const component = linkedComponent(schema);
  const mono = {
    fontFamily: font.mono,
    fontSize: '0.8125rem',
    overflowWrap: 'anywhere' as const,
  };
  if (!component) {
    return (
      <Box component="span" sx={{ ...mono, color: color.text.secondary }}>
        {label}
      </Box>
    );
  }
  return (
    <Link
      href={`#${schemaId(component)}`}
      sx={{
        ...mono,
        color: color.text.accent,
        textDecorationColor: alpha(color.text.accent, 0.4),
      }}
    >
      {label}
    </Link>
  );
}

/**
 * The collapsible row every operation and every schema is drawn as — the
 * quick start's endpoint row, generalised. `id` is both the anchor the rail
 * links to and what `useOpenRows` opens when the URL names it.
 */
function Row({
  id,
  open,
  onToggle,
  header,
  children,
}: {
  id: string;
  open: boolean;
  onToggle: () => void;
  header: ReactNode;
  children: ReactNode;
}) {
  const panelId = `${id}-panel`;
  return (
    <Box
      id={id}
      sx={{
        scrollMarginTop: 80,
        borderRadius: `${radius.md}px`,
        border: panelBorder,
        backgroundColor: color.surface.grayAlt,
        overflow: 'hidden',
      }}
    >
      <Stack
        component="button"
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={onToggle}
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
          '&:hover': { backgroundColor: alpha(color.white, 0.03) },
          '&:focus-visible': {
            outline: `2px solid ${color.stroke.action}`,
            outlineOffset: -2,
          },
        }}
      >
        {header}
        <ChevronRightRoundedIcon
          aria-hidden
          sx={{
            fontSize: 20,
            flexShrink: 0,
            color: color.text.tertiary,
            transform: open ? 'rotate(90deg)' : 'none',
            transition: 'transform 150ms',
          }}
        />
      </Stack>
      {open && (
        <Box id={panelId} sx={{ borderTop: panelBorder, px: 2, py: 2 }}>
          {children}
        </Box>
      )}
    </Box>
  );
}

/**
 * The id named by the URL's hash, or `''`.
 *
 * `decodeURIComponent` THROWS on a malformed escape — `#100%off` is enough —
 * and this is read inside a `useState` initializer, i.e. during render, with
 * no error boundary above it (`app.tsx` has none). Unguarded, a hash somebody
 * pasted wrong unmounted the whole page to blank instead of merely failing to
 * pre-open a row. An undecodable hash names no row we published, so the raw
 * text is the honest fallback: it opens nothing and renders everything.
 */
function hashId(): string {
  const raw = window.location.hash.replace(/^#/, '');
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

/** Which rows are open. A row named in the URL's hash opens itself. */
function useOpenRows(): {
  isOpen: (id: string) => boolean;
  toggle: (id: string) => void;
} {
  const [open, setOpen] = useState<ReadonlySet<string>>(() => {
    const id = hashId();
    return new Set(id ? [id] : []);
  });

  useEffect(() => {
    const fromHash = () => {
      const id = hashId();
      if (id) setOpen((prev) => (prev.has(id) ? prev : new Set(prev).add(id)));
    };
    window.addEventListener('hashchange', fromHash);
    return () => window.removeEventListener('hashchange', fromHash);
  }, []);

  return {
    isOpen: (id) => open.has(id),
    toggle: (id) =>
      setOpen((prev) => {
        const next = new Set(prev);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      }),
  };
}

/* -------------------------------------------------------------------------- */
/* Tables                                                                     */
/* -------------------------------------------------------------------------- */

/**
 * The three-column table parameters and properties share: name, type,
 * description. A CSS grid rather than a `<table>` so it can become one
 * column on a phone, where three columns of monospace do not fit 343px.
 */
const GRID_COLUMNS = {
  xs: '1fr',
  sm: 'minmax(150px, 1.1fr) minmax(130px, 0.9fr) minmax(0, 2.6fr)',
};

function FieldTable({
  caption,
  rows,
}: {
  caption: string;
  rows: {
    key: string;
    name: ReactNode;
    type: ReactNode;
    description: ReactNode;
  }[];
}) {
  const head = {
    display: { xs: 'none', sm: 'block' },
    py: 1,
    fontFamily: font.secondary,
    fontSize: '0.75rem',
    fontWeight: 600,
    textTransform: 'uppercase' as const,
    letterSpacing: '0.06em',
    color: color.text.tertiary,
    borderBottom: cardBorder,
  };
  const cell = {
    minWidth: 0,
    py: { xs: 0.5, sm: 1.25 },
    pr: { sm: 2 },
    overflowWrap: 'anywhere' as const,
  };
  return (
    <Box
      role="table"
      aria-label={caption}
      sx={{
        display: 'grid',
        gridTemplateColumns: GRID_COLUMNS,
        // Cells STRETCH to the row (the default): each carries its own
        // bottom rule, and only a cell as tall as its row draws that rule
        // where the row ends. `alignItems: 'start'` put three rules at three
        // heights on every row whose description wrapped.
        columnGap: 1,
      }}
    >
      <Box role="row" sx={{ display: 'contents' }}>
        <Box role="columnheader" sx={head}>
          Name
        </Box>
        <Box role="columnheader" sx={head}>
          Type
        </Box>
        <Box role="columnheader" sx={head}>
          Description
        </Box>
      </Box>
      {rows.map((row, i) => {
        const last = i === rows.length - 1;
        const border = last ? 'none' : { xs: 'none', sm: cardBorder };
        return (
          <Box
            role="row"
            key={row.key}
            sx={{
              display: 'contents',
              // The phone layout stacks the three cells; the rule between
              // rows moves to the row's last cell.
              '& > :last-child': {
                borderBottom: last ? 'none' : cardBorder,
                pb: { xs: 1.5, sm: 1.25 },
                mb: { xs: 1, sm: 0 },
              },
            }}
          >
            <Box role="cell" sx={{ ...cell, borderBottom: border }}>
              {row.name}
            </Box>
            <Box role="cell" sx={{ ...cell, borderBottom: border }}>
              {row.type}
            </Box>
            <Box
              role="cell"
              sx={{
                ...cell,
                borderBottom: border,
                fontFamily: font.secondary,
                fontSize: '0.875rem',
                color: color.text.secondary,
              }}
            >
              {row.description}
            </Box>
          </Box>
        );
      })}
    </Box>
  );
}

/** The name cell: monospace, with the required mark and the location. */
function FieldName({
  name,
  required,
  where,
}: {
  name: string;
  required: boolean;
  where?: string;
}) {
  return (
    <Stack spacing={0.5} alignItems="flex-start">
      <Box
        component="code"
        sx={{
          fontFamily: font.mono,
          fontSize: '0.875rem',
          fontWeight: 700,
          color: color.text.primary,
        }}
      >
        {name}
      </Box>
      <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
        {required && <Chip tone="error">required</Chip>}
        {where && <Chip>{where}</Chip>}
      </Stack>
    </Stack>
  );
}

/** `1 to 200`, `≥ 0`, `≤ 100 items` — or nothing, when neither end is set. */
function range(
  min: number | undefined,
  max: number | undefined,
  unit: string,
): string[] {
  if (min !== undefined && max !== undefined) {
    return [`${min} to ${max}${unit}`];
  }
  if (min !== undefined) return [`≥ ${min}${unit}`];
  if (max !== undefined) return [`≤ ${max}${unit}`];
  return [];
}

/**
 * What a schema says beyond its type: the allowed values, the default, the
 * bounds. Under the description, where Swagger puts them, so the type column
 * stays one label wide.
 */
function Constraints({
  doc,
  schema,
}: {
  doc: OpenApiDocument;
  schema: Schema | undefined;
}) {
  const resolved = deref(doc, schema).schema;
  const values = resolved?.enum;
  const bounds: string[] = [];
  if (resolved?.default !== undefined) {
    bounds.push(`default ${JSON.stringify(resolved.default)}`);
  }
  bounds.push(...range(resolved?.minimum, resolved?.maximum, ''));
  bounds.push(...range(resolved?.minItems, resolved?.maxItems, ' items'));
  if (!values?.length && bounds.length === 0) return null;
  return (
    <Stack
      direction="row"
      spacing={0.5}
      flexWrap="wrap"
      useFlexGap
      sx={{ mt: 1 }}
    >
      {values?.map((v) => (
        <Chip key={String(v)} tone="accent">
          {String(v)}
        </Chip>
      ))}
      {bounds.map((b) => (
        <Chip key={b}>{b}</Chip>
      ))}
    </Stack>
  );
}

function ParameterTable({
  doc,
  parameters,
}: {
  doc: OpenApiDocument;
  parameters: Parameter[];
}) {
  return (
    <FieldTable
      caption="Parameters"
      rows={parameters.map((p) => ({
        key: `${p.in}-${p.name}`,
        name: (
          <FieldName
            name={p.name}
            required={p.required === true}
            where={p.in}
          />
        ),
        type: <TypeRef doc={doc} schema={p.schema} />,
        description: (
          <>
            <Prose text={p.description} />
            <Constraints doc={doc} schema={p.schema} />
          </>
        ),
      }))}
    />
  );
}

function PropertyTable({
  doc,
  schema,
}: {
  doc: OpenApiDocument;
  schema: Schema;
}) {
  const required = new Set(schema.required ?? []);
  const properties = Object.entries(schema.properties ?? {});
  if (properties.length === 0) return null;
  return (
    <FieldTable
      caption="Properties"
      rows={properties.map(([name, prop]) => ({
        key: name,
        name: <FieldName name={name} required={required.has(name)} />,
        type: <TypeRef doc={doc} schema={prop} />,
        description: (
          <>
            <Prose text={describe(doc, prop)} />
            <Constraints doc={doc} schema={prop} />
          </>
        ),
      }))}
    />
  );
}

/* -------------------------------------------------------------------------- */
/* Operations                                                                 */
/* -------------------------------------------------------------------------- */

const subheading = {
  fontFamily: font.secondary,
  fontSize: '0.75rem',
  fontWeight: 600,
  textTransform: 'uppercase' as const,
  letterSpacing: '0.06em',
  color: color.text.tertiary,
};

function Subheading({ children }: { children: ReactNode }) {
  return (
    <Typography component="h4" sx={subheading}>
      {children}
    </Typography>
  );
}

function Responses({
  doc,
  operation,
}: {
  doc: OpenApiDocument;
  operation: Operation;
}) {
  const entries = Object.entries(operation.responses ?? {});
  if (entries.length === 0) return null;
  const success = entries.find(([status]) => statusTone(status) === 'success');
  const successMedia = primaryMedia(success?.[1].content);
  return (
    <Stack spacing={2}>
      <Subheading>Responses</Subheading>
      <Stack spacing={0}>
        {entries.map(([status, response], i) => {
          const media = primaryMedia(response.content);
          return (
            <Stack
              key={status}
              direction={{ xs: 'column', sm: 'row' }}
              spacing={{ xs: 0.5, sm: 2 }}
              alignItems={{ sm: 'baseline' }}
              sx={{
                py: 1.25,
                borderBottom: i === entries.length - 1 ? 'none' : cardBorder,
              }}
            >
              <Box sx={{ width: { sm: 64 }, flexShrink: 0 }}>
                <Chip tone={statusTone(status)}>{status}</Chip>
              </Box>
              <Box
                sx={{
                  flex: 1,
                  minWidth: 0,
                  fontFamily: font.secondary,
                  fontSize: '0.875rem',
                  color: color.text.secondary,
                }}
              >
                <Prose text={response.description} />
              </Box>
              {media?.media.schema && (
                <Box sx={{ flexShrink: 0 }}>
                  <TypeRef doc={doc} schema={media.media.schema} />
                </Box>
              )}
            </Stack>
          );
        })}
      </Stack>
      {success && successMedia?.media.schema && (
        <ExampleCard
          title={`${success[0]} · ${successMedia.mediaType}`}
          value={
            successMedia.media.example ??
            exampleOf(doc, successMedia.media.schema)
          }
          label={`the ${success[0]} example`}
        />
      )}
    </Stack>
  );
}

function RequestBody({
  doc,
  operation,
}: {
  doc: OpenApiDocument;
  operation: Operation;
}) {
  const body = operation.requestBody;
  const media = primaryMedia(body?.content);
  if (!body || !media) return null;
  return (
    <Stack spacing={2}>
      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        flexWrap="wrap"
        useFlexGap
      >
        <Subheading>Request body</Subheading>
        {body.required && <Chip tone="error">required</Chip>}
        <Chip>{media.mediaType}</Chip>
        {media.media.schema && (
          <TypeRef doc={doc} schema={media.media.schema} />
        )}
      </Stack>
      <Prose
        text={body.description}
        sx={{
          fontFamily: font.secondary,
          fontSize: '0.875rem',
          color: color.text.secondary,
        }}
      />
      {media.media.schema && (
        <ExampleCard
          title="Example body"
          value={media.media.example ?? exampleOf(doc, media.media.schema)}
          label="the example request body"
        />
      )}
    </Stack>
  );
}

function OperationRow({
  doc,
  entry,
  open,
  onToggle,
}: {
  doc: OpenApiDocument;
  entry: OperationEntry;
  open: boolean;
  onToggle: () => void;
}) {
  const { id, method, path, operation } = entry;
  const keyed = requiresKey(doc, operation);
  const header = apiKeyHeader(doc);
  return (
    <Row
      id={id}
      open={open}
      onToggle={onToggle}
      header={
        <>
          <MethodBadge method={method} />
          <Box
            component="code"
            sx={{
              minWidth: 0,
              fontFamily: font.mono,
              fontSize: '0.9375rem',
              fontWeight: 700,
              overflowWrap: 'anywhere',
              textDecoration: operation.deprecated ? 'line-through' : 'none',
            }}
          >
            {path}
          </Box>
          <Typography
            variant="body1"
            component="span"
            sx={{
              flex: 1,
              minWidth: 0,
              color: color.text.tertiary,
              display: { xs: 'none', md: 'inline' },
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {inline(displaySummary(operation.summary) ?? '')}
          </Typography>
          {keyed && (
            <LockOutlinedIcon
              titleAccess={
                header ? `Requires the ${header} header` : 'Requires an API key'
              }
              sx={{ fontSize: 16, flexShrink: 0, color: color.text.tertiary }}
            />
          )}
        </>
      }
    >
      <Stack spacing={3}>
        {operation.summary && (
          <Typography
            variant="body2"
            sx={{ color: color.text.secondary, display: { md: 'none' } }}
          >
            {inline(displaySummary(operation.summary) ?? '')}
          </Typography>
        )}
        <Prose
          text={operation.description}
          sx={{
            fontFamily: font.secondary,
            fontSize: '0.9375rem',
            color: color.text.secondary,
          }}
        />
        {keyed && header && (
          <Typography variant="body2" sx={{ color: color.text.tertiary }}>
            Requires the <code>{header}</code> header.
          </Typography>
        )}
        {operation.parameters && operation.parameters.length > 0 && (
          <Stack spacing={1.5}>
            <Subheading>Parameters</Subheading>
            <ParameterTable doc={doc} parameters={operation.parameters} />
          </Stack>
        )}
        <RequestBody doc={doc} operation={operation} />
        <Responses doc={doc} operation={operation} />
      </Stack>
    </Row>
  );
}

function TagSection({
  doc,
  group,
  rows,
}: {
  doc: OpenApiDocument;
  group: TagGroup;
  rows: ReturnType<typeof useOpenRows>;
}) {
  return (
    <DocSection
      id={tagId(group.name)}
      title={group.name.charAt(0).toUpperCase() + group.name.slice(1)}
      lede={group.description ? inline(group.description) : null}
    >
      <Stack spacing={1.5}>
        {group.operations.map((entry) => (
          <OperationRow
            key={entry.id}
            doc={doc}
            entry={entry}
            open={rows.isOpen(entry.id)}
            onToggle={() => rows.toggle(entry.id)}
          />
        ))}
      </Stack>
    </DocSection>
  );
}

/* -------------------------------------------------------------------------- */
/* Schemas                                                                    */
/* -------------------------------------------------------------------------- */

function SchemaRow({
  doc,
  name,
  schema,
  open,
  onToggle,
}: {
  doc: OpenApiDocument;
  name: string;
  schema: Schema;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <Row
      id={schemaId(name)}
      open={open}
      onToggle={onToggle}
      header={
        <>
          <Box
            component="code"
            sx={{
              minWidth: 0,
              fontFamily: font.mono,
              fontSize: '0.9375rem',
              fontWeight: 700,
              overflowWrap: 'anywhere',
            }}
          >
            {name}
          </Box>
          <Box
            component="span"
            sx={{
              flex: 1,
              minWidth: 0,
              fontFamily: font.mono,
              fontSize: '0.8125rem',
              color: color.text.tertiary,
            }}
          >
            {schema.enum ? 'enum' : typeLabel(doc, schema)}
          </Box>
        </>
      }
    >
      <Stack spacing={3}>
        <Prose
          text={schema.description}
          sx={{
            fontFamily: font.secondary,
            fontSize: '0.9375rem',
            color: color.text.secondary,
          }}
        />
        {schema.enum && (
          <Stack spacing={1.5}>
            <Subheading>Values</Subheading>
            <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
              {schema.enum.map((v) => (
                <Chip key={String(v)} tone="accent">
                  {String(v)}
                </Chip>
              ))}
            </Stack>
          </Stack>
        )}
        {schema.properties && (
          <Stack spacing={1.5}>
            <Subheading>Properties</Subheading>
            <PropertyTable doc={doc} schema={schema} />
          </Stack>
        )}
        {!schema.enum && (
          <ExampleCard
            title="Example"
            value={exampleOf(doc, schema)}
            label={`the ${name} example`}
          />
        )}
      </Stack>
    </Row>
  );
}

/* -------------------------------------------------------------------------- */
/* Page                                                                       */
/* -------------------------------------------------------------------------- */

const OVERVIEW_ID = 'overview';
const SCHEMAS_ID = 'schemas';

function tocFor(doc: OpenApiDocument, groups: TagGroup[]): TocEntry[] {
  const entries: TocEntry[] = [{ id: OVERVIEW_ID, label: 'Base URL & auth' }];
  for (const group of groups) {
    entries.push({
      id: tagId(group.name),
      label: group.name.charAt(0).toUpperCase() + group.name.slice(1),
    });
    for (const op of group.operations) {
      entries.push({
        id: op.id,
        label: `${op.method.toUpperCase()} ${op.path}`,
        level: 2,
      });
    }
  }
  if (Object.keys(doc.components?.schemas ?? {}).length > 0) {
    entries.push({ id: SCHEMAS_ID, label: 'Schemas' });
  }
  return entries;
}

function VersionChips({ doc }: { doc: OpenApiDocument }) {
  return (
    <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
      <Chip tone="accent">v{doc.info.version}</Chip>
      <Chip>OpenAPI {doc.openapi}</Chip>
    </Stack>
  );
}

function Reference({ doc }: { doc: OpenApiDocument }) {
  const groups = useMemo(() => groupByTag(doc), [doc]);
  const sections = useMemo(() => tocFor(doc, groups), [doc, groups]);
  const rows = useOpenRows();

  // The browser scrolls to a `#hash` on load only if the element exists
  // then, and every row here arrives with the document — so a pasted link
  // landed at the top of the page. Once, when the document is in.
  useEffect(() => {
    const id = hashId();
    // Optional call: jsdom has no `scrollIntoView`.
    if (id) document.getElementById(id)?.scrollIntoView?.();
  }, [doc]);
  const header = apiKeyHeader(doc);
  const baseUrl = doc.servers?.[0]?.url;
  const schemas = Object.entries(doc.components?.schemas ?? {});

  return (
    <DocPage
      sections={sections}
      eyebrow={<VersionChips doc={doc} />}
      title="API reference"
      lede={
        <>
          Every endpoint, parameter and response, read from the document the API
          serves. The raw{' '}
          <Link href={OPENAPI_JSON} sx={{ color: color.text.accent }}>
            OpenAPI JSON
          </Link>{' '}
          is the same bytes, for generators and other tooling.
        </>
      }
    >
      <DocSection
        id={OVERVIEW_ID}
        title="Base URL & authentication"
        lede={doc.info.description ? inline(doc.info.description) : null}
      >
        <Stack spacing={1.5}>
          {baseUrl && (
            <ValueStrip
              label="Base URL"
              value={baseUrl}
              copyLabel="the base URL"
            />
          )}
          {header && (
            <>
              <ValueStrip
                label="Header"
                value={`${header}: YOUR_API_KEY`}
                copyLabel="the header"
              />
              <Typography variant="body2" sx={{ color: color.text.tertiary }}>
                Routes marked with a lock need it on every request. Nothing on
                this page sends a request; copy one from the quick start.
              </Typography>
            </>
          )}
        </Stack>
      </DocSection>

      {groups.map((group) => (
        <TagSection key={group.name} doc={doc} group={group} rows={rows} />
      ))}

      {schemas.length > 0 && (
        <DocSection
          id={SCHEMAS_ID}
          title="Schemas"
          lede="Every object and enum the responses above are built from."
        >
          <Stack spacing={1.5}>
            {schemas.map(([name, schema]) => (
              <SchemaRow
                key={name}
                doc={doc}
                name={name}
                schema={schema}
                open={rows.isOpen(schemaId(name))}
                onToggle={() => rows.toggle(schemaId(name))}
              />
            ))}
          </Stack>
        </DocSection>
      )}
    </DocPage>
  );
}

/** The page while the document is on its way, or when it did not arrive. */
function Placeholder({
  loaded,
  reload,
}: {
  loaded: Exclude<Loaded, { state: 'ok' }>;
  reload: () => void;
}) {
  return (
    <DocPage
      sections={[]}
      title="API reference"
      lede="Every endpoint, parameter and response, read from the document the API serves."
    >
      {loaded.state === 'loading' ? (
        <Stack
          direction="row"
          spacing={2}
          alignItems="center"
          aria-busy="true"
          sx={{ color: color.text.tertiary }}
        >
          <CircularProgress
            size={20}
            aria-label="Loading the API description"
          />
          <Typography variant="body2">Loading the API description…</Typography>
        </Stack>
      ) : (
        <DocCard title="The API description could not be loaded">
          <Stack spacing={2} sx={{ p: 2 }}>
            <Typography
              variant="body2"
              role="alert"
              sx={{ color: color.text.secondary, overflowWrap: 'anywhere' }}
            >
              {loaded.message}
            </Typography>
            <Stack
              direction="row"
              spacing={2}
              alignItems="center"
              flexWrap="wrap"
              useFlexGap
            >
              <Button variant="contained" onClick={reload}>
                Try again
              </Button>
              <Link href={OPENAPI_JSON} sx={{ color: color.text.accent }}>
                Open the raw document
              </Link>
            </Stack>
          </Stack>
        </DocCard>
      )}
    </DocPage>
  );
}

export function ApiReference() {
  const { loaded, reload } = useOpenApi(OPENAPI_JSON);
  return loaded.state === 'ok' ? (
    <Reference doc={loaded.doc} />
  ) : (
    <Placeholder loaded={loaded} reload={reload} />
  );
}

export default ApiReference;
