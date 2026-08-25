import ArrowForwardRoundedIcon from '@mui/icons-material/ArrowForwardRounded';
import Box from '@mui/material/Box';
import Container from '@mui/material/Container';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import type { ElementType, ReactNode } from 'react';

import { color, font, radius } from '../theme/tokens';

/**
 * The handful of shapes the landing page repeats, in one file.
 *
 * Everything here appears at least three times in the Figma frame. Anything
 * that appears once lives in the section that uses it — a "primitives" module
 * that collects one-offs is just a second place to look for the same code.
 */

/**
 * Sections alternate between the two floor colours down the page, and each
 * section's cards take the OTHER one. Passing the tone down rather than
 * hard-coding a background per section is what keeps that alternation a single
 * rule instead of fourteen independent decisions — insert a section in the
 * middle and only the `tone` props after it change.
 */
export type Tone = 'base' | 'alt';

export const toneBackground = (tone: Tone) =>
  tone === 'base' ? color.surface.background : color.surface.backgroundAlt;

/**
 * The card colour for a section, **stated per section rather than derived**.
 *
 * There is no rule to derive. Measured off the export, the design uses three
 * different card fills on the four `alt` sections alone — #212121 under the
 * feature grid, #1a1a1a under Endpoints and Fair Access, #0f0f0f under the
 * FAQ — and the earlier two-tone helper flattened all of them to one wrong
 * value. Each section names its own; this type is what keeps that a short
 * list of intentions rather than a scatter of hex literals.
 */
export type CardSurface = 'raised' | 'sunken' | 'deep';

export const cardSurface = (surface: CardSurface) =>
  ({
    /** One step up from the floor. The feature grid. */
    raised: color.surface.background,
    /** One step down. Endpoints, Fair Access. */
    sunken: color.surface.grayAlt,
    /** The floor itself, separated by its border alone. Use Cases, Docs, FAQ. */
    deep: color.surface.backgroundAlt,
  })[surface];

/**
 * The card border. Figma's `Stroke/Default` (#535353) at 45% — at full
 * strength it is a visible grey rule, and the design reads as a hairline that
 * only separates the card from the section behind it.
 */
export const cardBorder = `1px solid ${alpha(color.stroke.default, 0.45)}`;

/**
 * The brand glow behind a section — the hero's light, reused down the page.
 *
 * Two stacked radials for the same reason the hero's backdrop stacks them: a
 * single gradient bright enough to be seen against #0f0f0f bands where it
 * ends, and two (a tight core inside a wide falloff) reads as a light source
 * instead. `strength` is the CORE's alpha; the halo is derived from it so a
 * section can be turned up or down with one number.
 *
 * `aria-hidden` and `pointer-events: none`: it is texture, and it covers the
 * whole section including whatever controls sit in it.
 */
export type GlowPlacement = { at: string; size?: string; strength?: number };

export function SectionGlow({
  at,
  size = '55% 65%',
  strength = 0.08,
}: GlowPlacement) {
  return (
    <Box
      aria-hidden
      sx={{
        position: 'absolute',
        inset: 0,
        pointerEvents: 'none',
        backgroundImage: `
          radial-gradient(${size} at ${at}, ${alpha(color.primary[400], strength)} 0%, transparent 68%),
          radial-gradient(90% 100% at ${at}, ${alpha(color.primary[400], strength * 0.5)} 0%, transparent 72%)`,
      }}
    />
  );
}

/** A full-bleed section with the design's vertical rhythm and 1280 px content. */
export function Section({
  tone = 'base',
  id,
  glow,
  children,
  sx,
}: {
  tone?: Tone;
  id?: string;
  /** Paint the brand glow behind this section — see {@link SectionGlow}. */
  glow?: GlowPlacement;
  children: ReactNode;
  sx?: object;
}) {
  return (
    <Box
      component="section"
      id={id}
      sx={{
        backgroundColor: toneBackground(tone),
        // Only when there is a glow to contain: `relative` is what the glow
        // positions against and `hidden` is what stops its falloff bleeding
        // into the section below, where it would read as a second, dimmer
        // light rather than the tail of this one.
        ...(glow && { position: 'relative', overflow: 'hidden' }),
        // The design's sections run 128px of padding above their eyebrow and
        // roughly as much below; measured against the export, every section
        // here was coming out 50–185px short of its Figma height, which reads
        // as a page that has been squeezed. 96px at 1440, 48 at 375 — the
        // desktop rhythm at phone width turns the page into a scroll of gaps.
        py: { xs: 6, md: 12 },
        ...sx,
      }}
    >
      {glow && <SectionGlow {...glow} />}
      {/* `relative` so the content sits above the glow's layer without either
          needing a z-index of its own. */}
      <Container sx={glow ? { position: 'relative' } : undefined}>
        {children}
      </Container>
    </Box>
  );
}

/**
 * A row of cards that is a grid on a wide screen and a swipeable rail on a
 * phone — the mobile design's answer to every three-across grid on the page.
 *
 * At `xs` the items stop wrapping and line up in one horizontal, scrollable
 * row, each `peek` wide so the next one shows at the right edge: that sliver
 * is the whole affordance, the only thing that says "there is more". The rail
 * bleeds out to the viewport edge (the `Container`'s 20px gutter, negated)
 * because a card clipped by the screen reads as "keep going" and a card
 * clipped by a padding box reads as broken. Scroll-snap so a flick lands on
 * a card rather than between two.
 *
 * From `sm` up it is the plain grid the caller describes in `columns`, and
 * none of the scrolling exists — `overflow: visible`, so a focus ring on a
 * card link is not clipped.
 *
 * The scrollbar is hidden at `xs` only. On a phone the rail is swiped, and
 * the bar would sit under the cards as a grey line the design does not have;
 * on a desktop the rail never scrolls, so nothing is being hidden from a
 * mouse user.
 */
export function CardRail({
  component = 'div',
  columns,
  peek = '88%',
  children,
  sx,
}: {
  component?: ElementType;
  /** The grid from `sm` up, e.g. `{ sm: 'repeat(2, 1fr)', lg: 'repeat(3, 1fr)' }`. */
  columns: { sm?: string; md?: string; lg?: string };
  /** Each item's width at `xs`. Under 100%, or nothing peeks. */
  peek?: string;
  children: ReactNode;
  sx?: object;
}) {
  return (
    <Box
      component={component}
      sx={{
        display: 'grid',
        gap: 2,
        gridTemplateColumns: { xs: 'none', ...columns },
        gridAutoFlow: { xs: 'column', sm: 'row' },
        gridAutoColumns: { xs: peek, sm: 'auto' },
        overflowX: { xs: 'auto', sm: 'visible' },
        scrollSnapType: { xs: 'x mandatory', sm: 'none' },
        scrollPaddingInline: 20,
        // The bleed. `auto` width + negative margins is what makes a block
        // (or a stretched flex item) wider than its parent by exactly the
        // gutter on each side; `alignSelf: stretch` is for the callers whose
        // Stack centres its children, which would otherwise shrink-wrap this.
        width: { xs: 'auto', sm: '100%' },
        alignSelf: { xs: 'stretch', sm: 'auto' },
        mx: { xs: -2.5, sm: 0 },
        px: { xs: 2.5, sm: 0 },
        scrollbarWidth: { xs: 'none', sm: 'auto' },
        '&::-webkit-scrollbar': { display: { xs: 'none', sm: 'block' } },
        '& > *': { scrollSnapAlign: 'start' },
        ...sx,
      }}
    >
      {children}
    </Box>
  );
}

/**
 * The yellow eyebrow pill — "Why Prices API", "Use Cases", "FAQ".
 *
 * Rendered as a `<p>`, not a heading. It labels the section that follows and
 * carries no outline weight of its own; making it an `<h3>` above the section's
 * real `<h2>` would put the document outline out of order for a screen reader
 * in exchange for nothing visible.
 */
export function SectionLabel({
  children,
  tone = 'brand',
}: {
  children: ReactNode;
  /**
   * `neutral` is the grey chip the design gives "Free Tier Limits" — measured
   * #212121 with white text off the Fair Access frame. It is the one eyebrow
   * on the page that labels a panel rather than a section, and the design
   * marks that difference by taking the yellow away: two brand chips side by
   * side ("Fair Access" and this one) would read as two section headings.
   */
  tone?: 'brand' | 'neutral';
}) {
  const neutral = tone === 'neutral';
  return (
    <Typography
      component="p"
      sx={{
        // `inline-block` alone, and NO `alignSelf`: the pill hugs its text, and
        // whether it sits left or centred is the parent's business. Forcing
        // `flex-start` here left-aligned it inside the three sections the
        // design centres.
        //
        // ⚠️ `inline-block` does NOT survive being a flex item: a column
        // `Stack` with no `alignItems` stretches its children across the
        // cross axis and the chip comes out as wide as whatever sits below
        // it. Either give the Stack an `alignItems`, or wrap the label in a
        // Box that takes the stretching — Fair Access does the latter.
        display: 'inline-block',
        backgroundColor: neutral
          ? color.surface.background
          : color.surface.primary,
        color: neutral ? color.white : color.black,
        // A rounded rectangle, NOT a pill. Measured off the exported frame:
        // the corner arc runs ~12px at 2x, so 6px at 1x on a 32px-tall chip.
        // `radius.pill` made every eyebrow on the page a lozenge.
        borderRadius: `${radius.chip}px`,
        px: 1.5,
        py: 0.5,
        fontFamily: font.secondary,
        fontWeight: 700,
        fontSize: '0.875rem',
        lineHeight: 1.4,
      }}
    >
      {children}
    </Typography>
  );
}

/**
 * The circled arrow inside both hero buttons.
 *
 * `aria-hidden`, always: it is decoration on a control whose label already
 * says where it goes, and announcing "arrow forward" after "Get API Key" is
 * noise. The circle is a real element rather than an icon with a border so the
 * three variants differ by colour alone:
 *
 * - `onPrimary` — filled black, yellow arrow. On the yellow button.
 * - `onDark` — outlined yellow, yellow arrow. On the dark outlined button.
 * - `onLight` — filled yellow, black arrow. On the white button in the closing
 *   call to action, where a black disc would be the heaviest thing on the
 *   section and the design puts the brand colour instead.
 */
export function ArrowBadge({
  variant,
}: {
  variant: 'onPrimary' | 'onDark' | 'onLight';
}) {
  const onPrimary = variant === 'onPrimary';
  const onLight = variant === 'onLight';
  return (
    <Box
      aria-hidden
      sx={{
        width: 24,
        height: 24,
        flexShrink: 0,
        borderRadius: '50%',
        display: 'grid',
        placeItems: 'center',
        backgroundColor: onLight
          ? color.primary[400]
          : onPrimary
            ? color.black
            : 'transparent',
        border:
          onPrimary || onLight ? 'none' : `1.5px solid ${color.primary[400]}`,
        color: onLight ? color.black : color.primary[400],
      }}
    >
      <ArrowForwardRoundedIcon sx={{ fontSize: 14 }} />
    </Box>
  );
}

/**
 * A section's centred heading block: eyebrow, title, one line of subcopy.
 *
 * `component` is a required-by-default `h2`. The page has exactly one `h1` (the
 * hero), and every section below it is a sibling at level 2 — MUI's `variant`
 * controls size only, so without this the visual scale and the document
 * outline drift apart the first time a section wants a smaller title.
 */
export function SectionHeading({
  label,
  title,
  subtitle,
  align = 'center',
  id,
}: {
  label: string;
  title: ReactNode;
  subtitle?: ReactNode;
  align?: 'center' | 'left';
  id?: string;
}) {
  return (
    <Stack
      spacing={2}
      alignItems={align === 'center' ? 'center' : 'flex-start'}
      sx={{ textAlign: align, maxWidth: align === 'center' ? 780 : 520 }}
    >
      <SectionLabel>{label}</SectionLabel>
      <Typography variant="h2" component="h2" id={id} color="text.primary">
        {title}
      </Typography>
      {subtitle && (
        <Typography variant="body1" sx={{ color: color.text.tertiary }}>
          {subtitle}
        </Typography>
      )}
    </Stack>
  );
}

/**
 * The window-chrome card: three dots, a monospace title, and a body.
 *
 * The design uses this shape three times — the hero's terminal, the Endpoints
 * example response, and the dashboard preview — and the real `/dashboard`
 * route now uses it too, which is the point of extracting it. A visitor who
 * saw the preview on the landing page should recognise the thing they land on.
 */
export function WindowCard({
  title,
  children,
  surface = 'sunken',
  sx,
}: {
  title: string;
  children: ReactNode;
  surface?: CardSurface;
  sx?: object;
}) {
  return (
    <Box
      sx={{
        borderRadius: `${radius.lg}px`,
        border: cardBorder,
        backgroundColor: cardSurface(surface),
        overflow: 'hidden',
        width: '100%',
        ...sx,
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        spacing={2}
        sx={{ px: 2, height: 40, borderBottom: cardBorder }}
      >
        <Stack direction="row" spacing={0.5} aria-hidden>
          {[color.red[400], color.yellow[400], color.green[400]].map((dot) => (
            <Box
              key={dot}
              sx={{
                width: 12,
                height: 12,
                borderRadius: '50%',
                backgroundColor: dot,
              }}
            />
          ))}
        </Stack>
        <Typography
          component="span"
          sx={{
            fontFamily: font.mono,
            fontSize: '0.875rem',
            fontWeight: 500,
            color: color.text.tertiary,
          }}
        >
          {title}
        </Typography>
      </Stack>
      {children}
    </Box>
  );
}

/**
 * A section painted with the design's vertical gradient rather than a flat
 * floor — the Developer Dashboard band fades `background` → `background-alt`
 * and the closing call to action fades back the other way, which is what stops
 * the lower half of the page from reading as one long slab.
 */
export function GradientSection({
  id,
  from,
  to,
  children,
  sx,
}: {
  id?: string;
  from: Tone;
  to: Tone;
  children: ReactNode;
  sx?: object;
}) {
  return (
    <Box
      component="section"
      id={id}
      sx={{
        backgroundImage: `linear-gradient(${toneBackground(from)}, ${toneBackground(to)})`,
        py: { xs: 6, md: 12 },
        ...sx,
      }}
    >
      <Container>{children}</Container>
    </Box>
  );
}
