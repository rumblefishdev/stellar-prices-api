/**
 * The Figma design system's variables, transcribed once.
 *
 * Source: the `Designs` file's `Landing page` frame
 * (`n1p6WCMVd4iinbuvOA2WjP`, node `778:5567`), read through the Figma MCP
 * server's `get_variable_defs`. **The names here are Figma's names**, not
 * invented ones — `surface.background`, `text.tertiary`, `stroke.default` —
 * so that a designer's "the tertiary text is too dim" maps to exactly one
 * line of this file and a variable rename in Figma is a one-line diff here.
 *
 * Kept SEPARATE from `theme.ts` on purpose. This module is the transcription
 * and holds no opinions; `theme.ts` is the interpretation — which token becomes
 * `palette.primary.main`, which becomes a border. Mixing the two is how a
 * design system drifts: somebody adjusts a shade to fix one component and the
 * file no longer says what Figma says.
 *
 * Values that look odd but are not:
 *
 * - `letterSpacing: -2` in Figma is **percent**, not pixels — the body styles
 *   are tracked in by 2%. Rendered as `-0.02em` so it scales with the size.
 * - The spacing scale is keyed by MULTIPLES of 8 (`0.25 → 2px`, `1 → 8px`,
 *   `10 → 80px`), which is why MUI's default 8px `spacing` unit is left alone:
 *   `theme.spacing(3)` and the design's `3` are the same 24px.
 */

/** Raw palette, straight from the Figma variables. */
export const color = {
  /** Brand yellow. One hue does the work of a whole accent scale. */
  primary: {
    100: '#fffcc2',
    300: '#ffe945',
    400: '#fdda24',
    600: '#cc9302',
    900: '#724311',
    950: '#432205',
  },
  text: {
    primary: '#f5f5f5',
    secondary: '#d3d3d3',
    tertiary: '#a3a3a3',
    accent: '#fdda24',
    success: '#05df72',
    error: '#ffa2a2',
  },
  surface: {
    /**
     * The page floor, and the odd-numbered sections.
     *
     * The design ALTERNATES these two down the page — hero and Use Cases on
     * `background`, the feature grid on `backgroundAlt` — and each section's
     * cards take the *other* value. That is what separates a card from its
     * section without a shadow, and it is why neither of these is "the
     * background": which one a section gets is a property of the section.
     * Measured off the Figma renders: #212121 and #0f0f0f, cards #272727.
     */
    background: '#212121',
    /** The alternating section, one step darker. */
    backgroundAlt: '#0f0f0f',
    /** Cards on a `background` section. */
    gray: '#272727',
    /** The darkest surface — code blocks and terminal chrome. */
    grayAlt: '#1a1a1a',
    primary: '#fdda24',
    success: '#0d542b',
  },
  stroke: {
    default: '#535353',
    action: '#edbe05',
  },
  /** The six feature-icon hues. 400 is the glyph, 900 the disc behind it. */
  accent: {
    emerald: { 100: '#d0fae5', 400: '#00d492', 600: '#009966', 900: '#004f3b' },
    violet: { 100: '#ede9fe', 400: '#a684ff', 900: '#4d179a' },
    blue: { 100: '#dbeafe', 900: '#1c398e' },
  },
  gray: { 50: '#fafafa', 900: '#1a1a1a' },
  red: { 400: '#ff6467' },
  yellow: { 400: '#ffb900' },
  green: { 400: '#05df72' },
  black: '#000000',
  white: '#ffffff',
} as const;

/**
 * The three families, in the roles Figma gives them.
 *
 * Each carries a real fallback stack. The faces are self-hosted (see
 * `fonts.css`) and `font-display: swap` means the fallback is what a visitor
 * reads for the first few hundred milliseconds — so the fallbacks are chosen
 * to be close in metrics, not left as a bare `sans-serif`.
 */
export const font = {
  /** Headings. */
  primary: "'Clash Display', 'Trebuchet MS', system-ui, sans-serif",
  /** Body copy and UI. */
  secondary: "'Satoshi', system-ui, -apple-system, 'Segoe UI', sans-serif",
  /** Code, keys, figures — anything that must not be proportional. */
  mono: "'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, monospace",
} as const;

/** Corner radii. `pill` is Figma's 9999. */
/**
 * Corner radii.
 *
 * `chip` is not a Figma variable — it is measured. The eyebrow labels
 * ("Endpoints", "Fair Access", …) are rounded rectangles whose corner arc runs
 * about 12px on the 2x export, i.e. 6px at design scale, and neither `md` (12)
 * nor `pill` matches that. Buttons ARE pills; the labels never were.
 */
export const radius = { chip: 6, md: 12, lg: 16, pill: 9999 } as const;

/** The tracked-in body letter-spacing — Figma's `-2` percent. */
export const bodyTracking = '-0.02em';
