import CheckRoundedIcon from '@mui/icons-material/CheckRounded';
import ContentCopyRoundedIcon from '@mui/icons-material/ContentCopyRounded';
import Box from '@mui/material/Box';
import Container from '@mui/material/Container';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import { useEffect, useState, type ReactNode } from 'react';

import { color, font, radius } from '../theme/tokens';
import { panelBorder } from './DashboardPanel';
import { cardBorder } from './primitives';

/**
 * The documentation pages' building blocks — the quick start's, lifted out.
 *
 * Task 0193 built these in `quickstart/QuickStart.tsx` off the Figma `Quick
 * start` frame (`918:644`): the rule grid, the headline glow, the sticky rail,
 * the title band cards, the copy pill. Task 0195's API reference is the same
 * page shape with different content, so the pieces moved here rather than
 * being interpreted a second time from the frame — a second interpretation is
 * how two pages of one portal end up two pixels apart everywhere.
 *
 * Nothing changed on the way out except the section id, which was the quick
 * start's own union and is any string now, and `Toc`/`DocPage`, which take
 * their sections as a prop instead of reading the quick start's list.
 */

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
export function CopyButton({ text, label }: { text: string; label: string }) {
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
export function SectionTitle({
  id,
  title,
  lede,
}: {
  id: string;
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
      {lede && (
        <Typography variant="body1" sx={{ color: color.text.secondary }}>
          {lede}
        </Typography>
      )}
    </Stack>
  );
}

/** The wrapper every section takes: title, then a 24px gap, then the body. */
export function DocSection({
  id,
  title,
  lede,
  children,
}: {
  id: string;
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
export function DocCard({
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
export function Code({ children, sx }: { children: ReactNode; sx?: object }) {
  return (
    <Box
      component="pre"
      sx={{
        m: 0,
        p: 2,
        fontFamily: font.mono,
        fontSize: '0.8125rem',
        lineHeight: 1.7,
        color: color.text.secondary,
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
export function ValueStrip({
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

/* -------------------------------------------------------------------------- */
/* Table of contents                                                          */
/* -------------------------------------------------------------------------- */

/**
 * One rail entry. `level: 2` is an entry inside the one above it — the API
 * reference lists every operation under its tag — drawn indented, in the
 * monospace, and allowed to wrap where the rail is a column.
 */
export type TocEntry = {
  id: string;
  label: string;
  level?: 1 | 2;
};

/**
 * The left rail: every section, the current one underlined, as the frame
 * draws it. A row of pills on a phone, where a column of ten would push the
 * guide below the fold.
 *
 * The current section is read off the page on every scroll rather than from
 * an `IntersectionObserver`. The observer reports only the headings whose
 * visibility just changed, so between two of its callbacks — scrolling
 * through a long section, or landing mid-page from a `#hash` — it has nothing
 * to say and the rail stays on whatever it last knew. This reads the positions
 * each time and always has an answer.
 *
 * `getClientRects()` is the "is this laid out at all" guard. In jsdom every
 * rect is zero, which would otherwise make every heading look as though it had
 * passed the line and light up the last entry on a page nobody has scrolled.
 */
export function Toc({ sections }: { sections: readonly TocEntry[] }) {
  const [current, setCurrent] = useState<string | undefined>(sections[0]?.id);

  useEffect(() => {
    let frame = 0;

    const update = () => {
      frame = 0;
      // 120px: the sticky navbar is 52 and a heading that has slid just under
      // it still reads as the section you are in, not the one before.
      const line = 120;
      let active = sections[0]?.id;
      let laidOut = false;
      for (const { id } of sections) {
        const el = document.getElementById(id);
        if (!el || el.getClientRects().length === 0) continue;
        laidOut = true;
        if (el.getBoundingClientRect().top <= line) active = id;
      }
      // The foot of the page. The last section is usually shorter than the
      // viewport, so its heading never reaches the line and "What's next"
      // would be unreachable however far you scrolled.
      if (
        laidOut &&
        sections.length > 0 &&
        window.innerHeight + window.scrollY >=
          document.documentElement.scrollHeight - 2
      ) {
        active = sections[sections.length - 1].id;
      }
      setCurrent(active);
    };

    // rAF-throttled: `update` reads layout, and doing that on every scroll
    // event is how a page of ten sections starts to feel heavy on a phone.
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(update);
    };

    update();
    window.addEventListener('scroll', onScroll, { passive: true });
    window.addEventListener('resize', onScroll);
    return () => {
      window.removeEventListener('scroll', onScroll);
      window.removeEventListener('resize', onScroll);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [sections]);

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
        // Ten entries are shorter than any laptop viewport, but a stuck rail
        // that runs off the bottom of a short window is unreachable — so it
        // scrolls inside itself rather than growing past the screen.
        maxHeight: { md: 'calc(100dvh - 120px)' },
        overflowY: { md: 'auto' },
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
        {sections.map(({ id, label, level = 1 }) => {
          const active = id === current;
          const nested = level === 2;
          return (
            <li key={id}>
              <Link
                href={`#${id}`}
                aria-current={active ? 'location' : undefined}
                sx={{
                  display: 'block',
                  // A nested entry is an operation's method and path, which
                  // does not fit 220px in one line — it wraps in the column
                  // and stays one pill in the phone rail.
                  whiteSpace: {
                    xs: 'nowrap',
                    md: nested ? 'normal' : 'nowrap',
                  },
                  overflowWrap: 'anywhere',
                  py: { xs: 0.75, md: nested ? 0.5 : 1 },
                  px: { xs: 1.5, md: 0 },
                  pl: { md: nested ? 1.5 : 0 },
                  borderRadius: { xs: `${radius.pill}px`, md: 0 },
                  border: { xs: cardBorder, md: 'none' },
                  borderBottom: {
                    md: `1px solid ${active ? color.text.primary : 'transparent'}`,
                  },
                  fontFamily: nested ? font.mono : font.secondary,
                  fontSize: nested ? '0.75rem' : '0.875rem',
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

/**
 * The documentation page: the rule grid, the rail, the headline with its glow
 * and lede, then the sections. Everything the quick start and the API
 * reference share above their first section.
 */
export function DocPage({
  sections,
  eyebrow,
  title,
  lede,
  children,
}: {
  sections: readonly TocEntry[];
  /** Chips or a label above the headline — the reference's version stamps. */
  eyebrow?: ReactNode;
  title: string;
  lede: ReactNode;
  children: ReactNode;
}) {
  return (
    <Box
      component="main"
      sx={{
        position: 'relative',
        // `clip`, NOT `hidden`. `hidden` makes this element a scroll container,
        // and a scroll container is what `position: sticky` sticks to — so the
        // rail was pinned to a box that never scrolls and rode up the page with
        // the content. `clip` still cuts off the glow's left edge without
        // creating one. It is also x-only, so the guide is not trapped in a
        // container of its own height.
        overflowX: 'clip',
        backgroundColor: color.surface.background,
        minHeight: 'calc(100dvh - 52px)',
        py: { xs: 5, md: 10 },
      }}
    >
      {/* The rule grid, MEASURED off the frame rather than carried over from
          the dashboard, and in PIXELS — which is the whole point. A percentage
          here resolves against this `main`, and this `main` is the entire
          guide, so a mask in percent runs thousands of pixels past its end.

          It is 80px, and it does not run the length of the page. On the
          frame the last rule is at 714px and it is gone by 870 — an ellipse
          centred a little left of the headline, 1150 × 470, which is also
          what makes it fade out to the right rather than stopping in a line.
          At its strongest a rule measures #393939 on the #212121 floor, i.e.
          `Stroke/Default` at half — 24 levels of contrast, near the limit of
          what is a texture rather than a table.

          The glow is NOT here: it hangs off the headline instead, so that it
          lands on the same word whatever the window is doing. See the note
          beside it. */}
      <Box
        aria-hidden
        sx={{
          position: 'absolute',
          inset: 0,
          pointerEvents: 'none',
          backgroundImage: `
            linear-gradient(${alpha(color.stroke.default, 0.5)} 1px, transparent 1px),
            linear-gradient(90deg, ${alpha(color.stroke.default, 0.5)} 1px, transparent 1px)`,
          backgroundSize: '80px 80px',
          // The frame's rules land on 41px / 18px within this element, not on
          // its corner.
          backgroundPosition: '41px 18px',
          maskImage: {
            xs: 'radial-gradient(560px 380px at 30% 300px, #000 0%, rgba(0,0,0,0.85) 25%, transparent 100%)',
            md: 'radial-gradient(1150px 470px at 380px 378px, #000 0%, rgba(0,0,0,0.85) 25%, transparent 100%)',
          },
        }}
      />
      <Container sx={{ position: 'relative' }}>
        <Stack
          direction={{ xs: 'column', md: 'row' }}
          spacing={{ xs: 3, md: 10 }}
          alignItems="flex-start"
        >
          <Toc sections={sections} />
          <Stack spacing={{ xs: 8, md: 12 }} sx={{ minWidth: 0, flex: 1 }}>
            <Stack
              spacing={2}
              // The glow's frame of reference. `isolate` so its `zIndex: -1`
              // means "behind the headline" and not "behind the page" — an
              // element sent below zero in the root stacking context
              // disappears under the floor colour painted on `main`.
              sx={{ maxWidth: 760, position: 'relative', isolation: 'isolate' }}
            >
              {/* The brand glow, on the first word of the headline.

                  Anchored to the HEADLINE, not to the page: measured off the
                  frame it sits at (390, 190), which on a 1440 frame is the
                  first line of the title — but 390px from the left edge of a
                  1920 window is out in the left margin, because the content
                  column is centred and the frame's is not. Hung off the title
                  block instead, it lands on the same word at every width, and
                  the phone layout needs no rule of its own.

                  Peaks at 12% of the brand yellow (measured #3b3721 at its
                  brightest) and is dead within ~180px. The mid stop is the
                  measured falloff — half strength at 80px, where a two-stop
                  gradient would put it at 105. */}
              <Box
                aria-hidden
                sx={{
                  position: 'absolute',
                  zIndex: -1,
                  pointerEvents: 'none',
                  // 210 × 220 radii, so the box is twice each and the centre
                  // of the gradient is the centre of the box. The offsets put
                  // that centre 10px right of the title's left edge and 58px
                  // down — the middle of the first word.
                  left: -200,
                  top: -162,
                  width: 420,
                  height: 440,
                  backgroundImage: `radial-gradient(210px 220px at 50% 50%, ${alpha(
                    color.primary[400],
                    0.12,
                  )} 0%, ${alpha(color.primary[400], 0.06)} 38%, transparent 85%)`,
                }}
              />
              {eyebrow}
              <Typography variant="h2" component="h1" color="text.primary">
                {title}
              </Typography>
              <Typography
                variant="subtitle2"
                sx={{ color: color.text.secondary }}
              >
                {lede}
              </Typography>
            </Stack>
            {children}
          </Stack>
        </Stack>
      </Container>
    </Box>
  );
}
