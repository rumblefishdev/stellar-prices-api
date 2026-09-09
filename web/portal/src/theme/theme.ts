import { createTheme, type ThemeOptions } from '@mui/material/styles';

import { bodyTracking, color, font, radius } from './tokens';

/**
 * The portal's MUI theme — the design system's *interpretation* of the tokens
 * in `tokens.ts`, and the second half of the stack task 0185 deliberately
 * shipped without.
 *
 * Three decisions worth stating, because they are the ones a later edit is
 * likely to undo by accident:
 *
 * 1. **`mode: 'dark'` is declared, not simulated.** MUI derives a great deal
 *    from it — `Paper`'s elevation overlays, the default `divider`, the
 *    contrast text it picks for a filled button. Hand-painting a dark palette
 *    onto `mode: 'light'` gets a page that looks right until the first
 *    component you did not override.
 * 2. **No `prefers-color-scheme` branch.** The design is dark, full stop; the
 *    Figma file has no light variant, and inventing one here would be this
 *    slice re-deciding something no designer decided.
 * 3. **Headings use `clamp()` rather than per-breakpoint overrides.** The
 *    acceptance criterion is 375 px, and a 48 px headline at 375 px wraps to
 *    four lines and pushes the sign-in control below the fold. `clamp()` keeps
 *    that in one declaration instead of a media query per heading level.
 *
 * The `letterSpacing` on every body style is Figma's `-2`, which is PERCENT —
 * see the note in `tokens.ts`. Headings are untracked (`letterSpacing: 0`),
 * which is what makes Clash Display's wide counters read as deliberate rather
 * than as a font that failed to load.
 */

/** Clash Display, the heading face, at a size that survives a 375 px screen. */
const heading = (min: number, max: number, weight = 600) => ({
  fontFamily: font.primary,
  fontWeight: weight,
  // `min` at 375 px, `max` from roughly 1100 px up. The middle term is the
  // fluid one; the viewport coefficient is what makes it track the screen
  // rather than step at a breakpoint.
  fontSize: `clamp(${min / 16}rem, ${(min / 16).toFixed(2)}rem + ${(
    ((max - min) / (1100 - 375)) *
    100
  ).toFixed(2)}vw, ${max / 16}rem)`,
  lineHeight: 1.2,
  letterSpacing: 0,
});

/** Satoshi, the body face, tracked in by Figma's 2%. */
const body = (size: number, lineHeight: number, weight = 500) => ({
  fontFamily: font.secondary,
  fontWeight: weight,
  fontSize: `${size / 16}rem`,
  lineHeight,
  letterSpacing: bodyTracking,
});

const options: ThemeOptions = {
  cssVariables: true,
  palette: {
    mode: 'dark',
    primary: {
      main: color.primary[400],
      light: color.primary[300],
      dark: color.primary[600],
      // Black on yellow. MUI would compute white here from its own contrast
      // threshold, which fails WCAG against #fdda24 by a wide margin — the
      // brand colour is bright enough that the accessible pairing is the dark
      // one, and the Figma buttons show black text for the same reason.
      contrastText: color.black,
    },
    secondary: { main: color.accent.emerald[400], contrastText: color.black },
    success: { main: color.green[400], contrastText: color.black },
    warning: { main: color.yellow[400], contrastText: color.black },
    error: { main: color.red[400], contrastText: color.black },
    info: { main: color.accent.violet[400], contrastText: color.black },
    background: {
      default: color.surface.background,
      paper: color.surface.grayAlt,
    },
    text: {
      primary: color.text.primary,
      secondary: color.text.secondary,
      disabled: color.text.tertiary,
    },
    // Figma's `Stroke/Default` at full strength is a visible grey line; at card
    // scale the design reads as a hairline, so the divider is the token with an
    // alpha rather than a different colour invented here.
    divider: 'rgba(83, 83, 83, 0.6)',
  },
  shape: { borderRadius: radius.md },
  typography: {
    fontFamily: font.secondary,
    // Every heading is Clash Display; every body style is Satoshi. There is no
    // third case, so the families are set per-variant rather than by overriding
    // `h1..h6` after the fact.
    h1: heading(40, 64, 700),
    h2: heading(32, 48),
    h3: heading(28, 40),
    h4: heading(24, 32),
    h5: heading(20, 24),
    h6: { ...heading(18, 20, 500), lineHeight: 1.22 },
    subtitle1: body(20, 1.5, 700),
    subtitle2: body(18, 1.5, 500),
    body1: body(16, 1.5),
    body2: body(14, 1.4),
    caption: body(12, 1.4),
    button: { ...body(16, 1.5, 700), textTransform: 'none' },
    overline: {
      fontFamily: font.mono,
      fontWeight: 500,
      fontSize: '0.75rem',
      lineHeight: 1.4,
      letterSpacing: 0,
      textTransform: 'none',
    },
  },
};

const base = createTheme(options);

/**
 * Component defaults, applied once here rather than as `sx` repeated down the
 * page. Anything that appears more than twice in the design belongs in this
 * block; anything that appears once belongs in the component that uses it.
 */
export const theme = createTheme(base, {
  components: {
    MuiCssBaseline: {
      styleOverrides: {
        // The floor colour has to be on `html` as well as `body`: overscroll on
        // iOS reveals whatever is behind the body, and a white flash under a
        // dark page is the single most obvious "this is a debug harness" tell
        // on a phone — which is the device the reviewer will open.
        'html, body': {
          backgroundColor: color.surface.background,
          scrollBehavior: 'smooth',
        },
        '@media (prefers-reduced-motion: reduce)': {
          'html, body': { scrollBehavior: 'auto' },
        },
        // The page renders an API key and a lot of `curl` snippets; a long
        // opaque string must break rather than push the layout sideways at
        // 375 px.
        code: { fontFamily: font.mono, overflowWrap: 'anywhere' },
        '::selection': {
          backgroundColor: color.primary[400],
          color: color.black,
        },
      },
    },
    MuiButton: {
      defaultProps: { disableElevation: true },
      styleOverrides: {
        root: { borderRadius: radius.pill, paddingInline: 20, minHeight: 44 },
        // 44 px minimum on every size, not just the default: the design's
        // small buttons are 36 px tall, which is below the touch target every
        // mobile guideline agrees on, and "works on a phone" is an acceptance
        // criterion. The visual height is preserved with padding instead.
        sizeSmall: { minHeight: 44, paddingBlock: 8 },
      },
    },
    MuiPaper: {
      // `elevation: 0` plus a real border. MUI's dark-mode elevation is a white
      // overlay, which on a #0f0f0f floor turns every card into a slightly
      // different grey depending on nesting depth — the design uses one card
      // colour and one border, at every depth.
      defaultProps: { elevation: 0 },
      styleOverrides: {
        root: {
          backgroundImage: 'none',
          backgroundColor: color.surface.grayAlt,
          border: `1px solid ${color.surface.gray}`,
          borderRadius: radius.lg,
        },
      },
    },
    MuiLink: {
      defaultProps: { underline: 'hover' },
      styleOverrides: {
        root: {
          color: color.text.accent,
          textUnderlineOffset: '0.2em',
          '&:focus-visible': {
            outline: `2px solid ${color.stroke.action}`,
            outlineOffset: 2,
            borderRadius: 4,
          },
        },
      },
    },
    MuiChip: {
      styleOverrides: {
        root: { borderRadius: radius.pill, fontFamily: font.secondary },
      },
    },
    MuiContainer: {
      // 1440 − 2×80 = 1280, the design's content width, and the 80 px gutter
      // collapses to 20 at 375 px so the cards do not become slivers.
      defaultProps: { maxWidth: false },
      styleOverrides: {
        root: ({ theme: t }: { theme: typeof base }) => ({
          maxWidth: 1280 + 160,
          paddingInline: 20,
          [t.breakpoints.up('md')]: { paddingInline: 80 },
        }),
      },
    },
  },
});

export default theme;
