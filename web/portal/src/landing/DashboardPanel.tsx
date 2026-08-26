import Box from '@mui/material/Box';
import LinearProgress from '@mui/material/LinearProgress';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import type { SxProps, Theme } from '@mui/material/styles';
import type { ReactNode } from 'react';

import { theme } from '../theme/theme';
import { color, font, radius } from '../theme/tokens';
import { cardBorder } from './primitives';

/**
 * The dashboard card's hairline — Figma's `Stroke/Default` at full strength.
 *
 * Distinct from the landing page's `cardBorder` (the same colour at 45%) and
 * measured, not chosen: the dashboard's floor is #212121 where the landing's
 * sections are #0f0f0f, and a 45% rule that separates a card on the darker
 * floor is invisible on the lighter one.
 */
export const panelBorder = `1px solid ${color.stroke.default}`;

/**
 * The dashboard's building blocks, shared by the landing page's preview and by
 * the real `/dashboard` route.
 *
 * **Shared on purpose.** The landing page shows a picture of the dashboard and
 * then the visitor signs in and lands on the real one; if those two are built
 * from different code they drift, and the promise the marketing section makes
 * stops matching the thing it was promising. One set of components means the
 * preview cannot go stale without the dashboard going stale with it.
 */

/** An inset panel inside the dashboard card — the darkest surface on the page. */
export function InsetPanel({
  children,
  sx,
}: {
  children: ReactNode;
  sx?: object;
}) {
  return (
    <Stack
      spacing={1}
      sx={{
        p: 2,
        borderRadius: `${radius.md}px`,
        border: cardBorder,
        backgroundColor: color.surface.backgroundAlt,
        ...sx,
      }}
    >
      {children}
    </Stack>
  );
}

/**
 * The card's header line: a title and a status pill.
 *
 * The pill is a claim about the KEY, not about the service — "Active" means
 * this key works. It is passed in rather than assumed, because the real
 * dashboard has states the preview does not (no key yet, revoked) and a pill
 * hard-coded to "Active" would be the card lying about a key that is not.
 */
export function PanelHeader({
  title,
  status,
}: {
  title: string;
  status?: { label: string; tone: 'ok' | 'muted' };
}) {
  return (
    <Stack
      direction="row"
      alignItems="center"
      justifyContent="space-between"
      spacing={2}
    >
      <Typography variant="h5" component="h3" color="text.primary">
        {title}
      </Typography>
      {status && (
        <Stack
          direction="row"
          spacing={0.75}
          alignItems="center"
          sx={{
            px: 1.25,
            py: 0.5,
            borderRadius: `${radius.pill}px`,
            backgroundColor:
              status.tone === 'ok'
                ? alpha(color.surface.success, 0.55)
                : color.surface.gray,
            border: `1px solid ${
              status.tone === 'ok'
                ? alpha(color.green[400], 0.3)
                : alpha(color.stroke.default, 0.45)
            }`,
          }}
        >
          <Box
            aria-hidden
            sx={{
              width: 8,
              height: 8,
              borderRadius: '50%',
              backgroundColor:
                status.tone === 'ok' ? color.text.success : color.text.tertiary,
            }}
          />
          <Typography
            variant="body2"
            sx={{
              fontWeight: 700,
              color:
                status.tone === 'ok' ? color.text.success : color.text.tertiary,
            }}
          >
            {status.label}
          </Typography>
        </Stack>
      )}
    </Stack>
  );
}

/**
 * The key, in the design's monospace yellow.
 *
 * `value` is whatever the caller decided to show — the masked run of dots or
 * the credential itself. This component never decides that: masking is task
 * 0187's rule and the reveal toggle lives with the state that owns it.
 */
export function KeyField({
  label,
  value,
  testId,
  inlineAction,
  actions,
}: {
  /**
   * Omitted on the first-login card, where the sentence above the box already
   * says what it is ("Copy it below and make your first request") and the
   * design draws the box bare.
   */
  label?: string;
  value: ReactNode;
  testId?: string;
  /**
   * A control that sits INSIDE the ring, on the value's row — the first-login
   * frame's "Show key" (Adam, 2026-08-26).
   *
   * Distinct from `actions`, which sit below the box: this one belongs to the
   * value it acts on, and the frame draws the two rows differently. The value
   * keeps `minWidth: 0` so a 40-character key wraps rather than pushing the
   * control off the card at 375 px.
   */
  inlineAction?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <Stack spacing={2}>
      <Stack
        spacing={1}
        sx={{
          p: 2,
          borderRadius: `${radius.md}px`,
          // The one yellow-outlined box on the page. The design rings the
          // credential itself rather than the panel around it, which is what
          // makes the key the thing your eye lands on when the dashboard opens
          // — and this screen exists so that a developer can copy it.
          //
          // `primary[400]`, measured off the frame's ring — not the darker
          // `stroke.action` (#edbe05) this used to take, which is the strip's
          // border below and reads as a duller yellow beside the key itself.
          border: `1px solid ${color.primary[400]}`,
          backgroundColor: color.surface.backgroundAlt,
        }}
      >
        {label && (
          <Typography variant="body2" sx={{ color: color.text.tertiary }}>
            {label}
          </Typography>
        )}
        {/* `flex-start`, not `space-between`: the frame puts the control
            immediately BESIDE the value, not against the far edge of a ring
            that runs the width of the card. The value keeps `minWidth: 0` and
            is the half that shrinks, so revealing a 40-character key wraps it
            rather than pushing the control out of the box. */}
        <Stack
          direction="row"
          spacing={1.5}
          sx={{ alignItems: 'center', justifyContent: 'flex-start' }}
        >
          <Box
            component="code"
            data-testid={testId}
            sx={{
              fontFamily: font.mono,
              fontSize: '0.9375rem',
              color: color.text.accent,
              // No chip behind the key: it already sits in its own ringed box,
              // and a second, differently-grey rectangle inside that is the
              // shade Adam kept seeing. The chrome's rule is narrowed to `p
              // code` so this is now what actually applies rather than what
              // loses to it on specificity.
              backgroundColor: 'transparent',
              padding: 0,
              // A 40-character opaque string must wrap rather than widen the
              // card — this renders at 375 px too.
              overflowWrap: 'anywhere',
              minWidth: 0,
            }}
          >
            {value}
          </Box>
          {inlineAction && (
            // `flexShrink: 0` so the control keeps its width and the key wraps
            // instead — the chrome's bare-`<button>` rule sets
            // `alignSelf: flex-start`, which this wrapper absorbs so the
            // button still sits on the value's centre line.
            <Box sx={{ flexShrink: 0, display: 'flex', alignItems: 'center' }}>
              {inlineAction}
            </Box>
          )}
        </Stack>
      </Stack>
      {actions && (
        <Stack direction="row" spacing={1.5} sx={{ flexWrap: 'wrap' }}>
          {actions}
        </Stack>
      )}
    </Stack>
  );
}

/**
 * Used-of-quota as a bar, a fraction and a reset date.
 *
 * **The bar is task 0193's addition; the numbers are task 0188's.** That split
 * matters: 0188 decided that this panel shows used, remaining and the limit as
 * figures rather than prose, and decided the reset rule's wording. This adds a
 * way to see the ratio at a glance and re-decides none of it — every number it
 * renders is passed in, and the caller keeps the labels and the `data-testid`s
 * those tests pin.
 *
 * `used` or `limit` being null is the honest "AWS has recorded nothing yet"
 * state (0188 again): the bar is omitted rather than drawn at zero, because a
 * zero-length bar reads as "you have used none of your quota" when the truth is
 * "we do not know yet".
 */
export function UsageMeter({
  used,
  limit,
  remaining,
  resetLabel,
  headline = false,
}: {
  used: number | null;
  limit: number | null;
  remaining?: number | null;
  resetLabel?: ReactNode;
  /**
   * The dashboard's treatment: the count set large in the brand colour, with
   * the percentage and what is left on the row under the bar. The landing
   * page's preview uses the compact form, where the card's own header has
   * already said "Monthly Usage".
   */
  headline?: boolean;
}) {
  const known = used !== null && limit !== null && limit > 0;
  const percent = known ? Math.min(100, Math.round((used / limit) * 100)) : 0;
  const full = percent >= 100;

  return (
    <Stack spacing={1.5}>
      {!headline && (
        <Stack
          direction="row"
          justifyContent="space-between"
          alignItems="baseline"
          spacing={2}
        >
          <Typography variant="body1" color="text.primary">
            Monthly Usage
          </Typography>
          {known && (
            <Typography variant="body1" color="text.primary">
              {used.toLocaleString('en-US')} / {limit.toLocaleString('en-US')}
            </Typography>
          )}
        </Stack>
      )}

      {known && headline && (
        <Stack
          direction="row"
          spacing={1.5}
          alignItems="baseline"
          sx={{ flexWrap: 'wrap' }}
        >
          <Typography
            component="span"
            sx={{
              fontFamily: font.primary,
              fontWeight: 700,
              fontSize: { xs: '2.25rem', sm: '3rem' },
              lineHeight: 1,
              // Yellow at every level, the ceiling included (Adam,
              // 2026-08-26). The figure is the card's identity, not its alarm:
              // the bar, the caption and the notice below already carry "you
              // have run out" in red, and turning the one big brand-coloured
              // number red as well made the card read as an error page rather
              // than a dashboard reporting a bad number.
              color: color.text.accent,
            }}
          >
            <RawFigure testId="usage-used" value={used} />
          </Typography>
          <Typography variant="subtitle1" sx={{ color: color.text.secondary }}>
            / <RawFigure testId="usage-limit" value={limit} /> requests
          </Typography>
        </Stack>
      )}

      {known && (
        <>
          {headline && (
            <Stack
              direction="row"
              justifyContent="space-between"
              spacing={2}
              // Measured: the percentage is tertiary grey and what is left is
              // a step brighter. The two are not the same claim — one is a
              // ratio, the other a number of requests you still have.
              sx={{ color: color.text.secondary }}
            >
              <Typography
                variant="body2"
                sx={{
                  // #ffa2a2 at the ceiling — the gradient's PALE end (Adam,
                  // 2026-08-26). The saturated #e7000b is reserved for the
                  // notice's picked-out words, and these two captions are
                  // labels on a bar rather than emphasis: the same red at full
                  // strength in four places on one card left nothing louder
                  // than anything else.
                  color: full ? color.red[300] : color.text.tertiary,
                }}
              >
                {full ? '100% - limit reached' : `${percent}% used`}
              </Typography>
              {remaining !== null && remaining !== undefined && (
                <Typography variant="body2" color="inherit">
                  <RawFigure testId="usage-remaining" value={remaining} />{' '}
                  remaining
                </Typography>
              )}
            </Stack>
          )}

          <LinearProgress
            variant="determinate"
            value={percent}
            // The figures beside it are the accessible version; the bar is a
            // second rendering of the same fact and would otherwise be read out
            // twice.
            aria-hidden
            sx={{
              // Measured off the frame: a 10px white pill with the fill
              // running #ffe945 → #cc9302 left to right. A flat brand yellow
              // on a white track reads as a solid block; the gradient is what
              // gives the bar a direction.
              height: 10,
              borderRadius: `${radius.pill}px`,
              backgroundColor: color.white,
              '& .MuiLinearProgress-bar': {
                borderRadius: `${radius.pill}px`,
                // Red at the ceiling. The bar is the fastest read on the card,
                // and "you have run out" should not arrive in the same colour
                // as "you have plenty left".
                backgroundColor: 'transparent',
                // Red at the ceiling, and a GRADIENT like the yellow one —
                // measured off the `Dashboard - limits` frame. The flat red
                // fill this replaces was the one bar on the page built
                // differently from the other, which read as a different
                // component rather than the same component in a worse state.
                backgroundImage: full
                  ? `linear-gradient(90deg, ${color.red[300]}, ${color.red[600]})`
                  : `linear-gradient(90deg, ${color.primary[300]}, ${color.primary[600]})`,
              },
            }}
          />

          {/* Under the bar, and LEFT — the frame's "Resets May 1" sits at the
              start of the row, not opposite the percentage. On the dashboard
              the percentage has already been said above the bar, so this row
              carries the one caption; on the landing preview, which passes no
              `resetLabel`, it carries the percentage instead. */}
          <Stack
            direction="row"
            justifyContent="space-between"
            spacing={2}
            // The fallback for this row's children. Both captions below set
            // their own colour because an inherited one loses to the chrome's
            // `& p` rule (see the reset label) — this stays as the value they
            // agree with, so the row is never the odd one out if a third
            // caption is added and forgets.
            //
            // Red at the ceiling is the frame's own treatment and it earns it:
            // once the quota is spent, the reset date stops being a footnote
            // and becomes the single most useful fact on the card, because it
            // is when the API starts answering again.
            sx={{ color: full ? color.red[300] : color.text.tertiary }}
          >
            {!headline && (
              <Typography variant="body2" color="inherit">
                {percent}% used
              </Typography>
            )}
            {resetLabel && (
              // ⚠️ The colour is set HERE and not inherited from the row.
              // `body2` renders a `<p>`, and the dashboard chrome paints every
              // `<p>` in its subtree secondary grey — a rule that beats an
              // inherited value on specificity, so `color="inherit"` silently
              // came out #d3d3d3 while the percentage beside it (which carries
              // its own `sx`) went red. Measured, not reasoned about: the two
              // captions bracket the same bar and were rendering in two
              // different colours.
              <Typography
                variant="body2"
                sx={{ color: full ? color.red[300] : color.text.tertiary }}
              >
                {resetLabel}
              </Typography>
            )}
          </Stack>
        </>
      )}
    </Stack>
  );
}

/**
 * A figure shown grouped and asserted raw.
 *
 * The dashboard reads `42,180`; task 0188's tests read `42180` off a
 * `data-testid`. Rather than pick one, the raw value goes in a clipped node
 * carrying the id and the grouped one beside it is `aria-hidden` — so the
 * tests, the accessibility tree and the eye each get the form they want, and
 * `toLocaleString` can never change what an assertion reads.
 */
function RawFigure({ testId, value }: { testId: string; value: number }) {
  return (
    <>
      <Box
        component="span"
        data-testid={testId}
        // `'1px'`, not `1`: in `sx` a unitless width at or below 1 is a
        // FRACTION, so `width: 1` would make this clipped span 100% wide and
        // give the page a horizontal scrollbar.
        sx={{
          position: 'absolute',
          width: '1px',
          height: '1px',
          overflow: 'hidden',
          clip: 'rect(0 0 0 0)',
          whiteSpace: 'nowrap',
        }}
      >
        {value}
      </Box>
      <span aria-hidden>{value.toLocaleString('en-US')}</span>
    </>
  );
}

/**
 * The dashboard's card shell — a titled header band over a body.
 *
 * The three cards in the Figma dashboard share it: "API Key", "Monthly Usage"
 * and "Rate Limit". The status pill lives in the header beside the title, which
 * is the only place the design ever puts one.
 */
export function DashboardCard({
  title,
  status,
  children,
  sx,
}: {
  title: string;
  status?: { label: string; tone: 'ok' | 'muted' | 'bad' };
  /**
   * Omitted for the deliberately empty tile — the `Dashboard - no key` frame
   * gives Monthly Usage and Rate Limit a header band over an empty body while
   * the card above carries the page's only action.
   *
   * The card keeps its header and its height in that case rather than
   * collapsing to a title bar: a collapsed panel reads as a rendering failure,
   * and the dashboard would visibly rearrange itself the moment a key arrived.
   */
  children?: ReactNode;
  sx?: object;
}) {
  return (
    <Box
      component="section"
      sx={{
        borderRadius: `${radius.lg}px`,
        // A SOLID #535353 hairline, not the landing page's 45% one — measured
        // off the frame's card edge. The dashboard's cards sit on a lighter
        // floor (#212121) than the landing's sections, and the faint rule that
        // separates a card there disappears here.
        border: panelBorder,
        // Two fills, not one: the title band is the darkest surface and the
        // body a step lighter, which is what makes the header read as a header
        // without a second border. Measured #1a1a1a over #272727.
        backgroundColor: color.surface.gray,
        overflow: 'hidden',
        ...sx,
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        spacing={1.5}
        sx={{
          px: 3,
          py: 2,
          backgroundColor: color.surface.grayAlt,
          borderBottom: panelBorder,
        }}
      >
        {/* 24px, the same size the page's own heading takes — measured, and
            equal on the frame: the card titles and "Dashboard" have the same
            cap height. `h5` (20px) made every card title a step smaller. */}
        <Typography variant="h4" component="h2" color="text.primary">
          {title}
        </Typography>
        {status && <StatusPill {...status} />}
      </Stack>
      <Stack
        spacing={2.5}
        sx={{
          p: 3,
          // The empty tile keeps a body. Proportional to the frame, where the
          // two empty cards stand about as tall as the filled Monthly Usage
          // card they replace — not a bare header band.
          ...(children === undefined && { minHeight: 240 }),
        }}
      >
        {children}
      </Stack>
    </Box>
  );
}

/** "Just issued", "Active", "Not issued". Dot plus label, coloured by tone. */
export function StatusPill({
  label,
  tone,
}: {
  label: string;
  tone: 'ok' | 'muted' | 'bad';
}) {
  // Solid fills, measured: #0d542b behind #05df72. The 55% versions these
  // replace were mixing with the card behind them, which on the frame's
  // lighter body turned the one green thing on the card grey-green.
  const skin = {
    ok: { fg: color.text.success, bg: color.surface.success },
    bad: { fg: color.text.error, bg: '#82181a' },
    // A step DARKER than the card body it sits on, not lighter: the body is
    // `surface.gray` now, so the old muted fill was the card itself.
    muted: { fg: color.text.tertiary, bg: color.surface.grayAlt },
  }[tone];

  return (
    <Stack
      direction="row"
      spacing={0.75}
      alignItems="center"
      sx={{
        px: 1,
        py: 0.25,
        borderRadius: `${radius.chip}px`,
        backgroundColor: skin.bg,
      }}
    >
      <Box
        aria-hidden
        sx={{
          width: 7,
          height: 7,
          borderRadius: '50%',
          backgroundColor: skin.fg,
        }}
      />
      <Typography variant="body2" sx={{ fontWeight: 700, color: skin.fg }}>
        {label}
      </Typography>
    </Stack>
  );
}

/**
 * The bordered note under the key card's metadata — the frame's yellow strip.
 *
 * A `<p>` inside a ringed box, with the warning glyph `aria-hidden`: the
 * sentence carries the meaning and "warning triangle" announced before it is
 * noise. It is `role="note"`, not `role="alert"`: nothing has just gone wrong
 * and nothing needs interrupting — it states a rule that applies before the
 * visitor presses anything.
 *
 * `glyph={false}` drops the disc, for the one place the frame draws this box
 * bare: inside the regenerate dialog (Adam, 2026-08-26), where the heading and
 * its own badge have already said "this is a warning" and a second marker two
 * lines down is the third time in one card. The ring and the yellow
 * `Important:` carry it there. Everywhere else the glyph stays — on the
 * dashboard the strip sits among ordinary prose with nothing else marking it.
 *
 * `sx` merges over the box's own rules, for the callers that need the same
 * strip at a different rhythm — the dialog runs it at a taller `py` than the
 * dashboard does. An escape hatch rather than a second boolean: the next
 * variation would otherwise be a third prop, and the component would end up
 * enumerating its call sites.
 */
export function NoticeStrip({
  children,
  glyph = true,
  tone = 'warning',
  sx,
}: {
  children: ReactNode;
  glyph?: boolean;
  /**
   * `'warning'` is the frame's yellow strip — a rule that applies, stated
   * before you trip over it (key rotation is once a month).
   *
   * `'error'` is task 0193's addition for the quota-reached card: a rule you
   * have ALREADY tripped over, and one that is changing what the API does to
   * your requests right now. The two are the same box because they are the same
   * kind of statement and the frame draws them identically apart from the hue —
   * but they must not be the same colour, because "this will apply" and "this
   * is applying" are not the same news, and the yellow one is on screen at the
   * same time two cards up.
   *
   * It stays `role="note"`, not `role="alert"`: nothing has *just* happened.
   * The state is already true when the dashboard opens, and an alert role
   * interrupts a screen reader mid-sentence to say so on every visit.
   */
  tone?: 'warning' | 'error';
  sx?: SxProps<Theme>;
}) {
  // Border, glyph and the picked-out words move together — the strip reads as
  // one coloured object, and a red rule around yellow emphasis would look like
  // two notices that collided.
  const skin =
    tone === 'error'
      ? // Measured, Adam 2026-08-26: prose #ffa2a2 with the picked-out words a
        // full step down at #e7000b — the same pair the bar's gradient runs
        // between, so the strip reads as belonging to the meter above it. The
        // emphasis is DARKER than its prose here, which inverts the yellow
        // strip's relationship: on red, a lighter accent on a light-red
        // sentence has nowhere to go, and the saturated end is the only value
        // that still reads as emphasis.
        { line: color.red[600], mark: color.red[600], prose: color.red[300] }
      : {
          line: color.stroke.action,
          mark: color.text.accent,
          prose: color.text.primary,
        };

  return (
    <Stack
      role="note"
      direction="row"
      spacing={1.5}
      // MUI's array form, not an object spread: `SxProps` is legally a
      // function or an array as well as an object, and spreading one of those
      // would silently drop it.
      sx={[
        {
          p: 1.5,
          borderRadius: `${radius.md}px`,
          border: `1px solid ${skin.line}`,
          // The card's darkest surface, measured — not a tinted brand wash. On
          // the frame the strip reads as an inset panel that happens to be
          // ringed yellow, and a brown fill under yellow text is the one
          // combination on this page that loses contrast.
          backgroundColor: color.surface.grayAlt,
        },
        ...(Array.isArray(sx) ? sx : [sx]),
      ]}
    >
      {glyph && (
        <Box
          aria-hidden
          sx={{
            flexShrink: 0,
            width: 20,
            height: 20,
            borderRadius: '50%',
            display: 'grid',
            placeItems: 'center',
            border: `1.5px solid ${skin.mark}`,
            color: skin.mark,
            fontFamily: font.secondary,
            fontWeight: 700,
            fontSize: '0.8125rem',
            lineHeight: 1,
          }}
        >
          !
        </Box>
      )}
      {/* The frame's two-colour sentence: white prose with the rule and the
          date picked out in the brand yellow. Styled here rather than at each
          call site so a `<strong>` inside any notice reads the same — and
          because the dashboard's chrome paints every `<p>` secondary grey,
          which this has to override to reach the frame's #f5f5f5.
          ⚠️ `margin: 0` belongs HERE, not to the caller. It used to arrive
          from the dashboard chrome's `& p` rule, which reaches this strip only
          while it renders inside that subtree — the regenerate dialog renders
          through a portal, outside it, and the `<p>` came back with the user
          agent's 1em above and below. The box owns its own rhythm now, so the
          padding a caller sets is the padding it gets wherever it is mounted;
          on the dashboard nothing moves, because the chrome was already
          zeroing it. */}
      <Box
        sx={{
          minWidth: 0,
          '& p': { ...theme.typography.body1, color: skin.prose, m: 0 },
          '& strong': { color: skin.mark, fontWeight: 700 },
        }}
      >
        {children}
      </Box>
    </Stack>
  );
}

/**
 * The metadata row under the key: label-over-value pairs separated by rules.
 *
 * A `<dl>`, because that is what it is — three terms and their definitions,
 * not a table and not a list of sentences. The rules are `Stack` dividers, so
 * they appear BETWEEN the columns and never trail the last one, and they are
 * dropped at `xs` where the columns stack and a vertical rule between rows
 * would be pointing the wrong way.
 *
 * The caller decides how many columns there are. On the first-login card that
 * is two or three: the quota column renders only once the usage endpoint has
 * answered, because the alternative is printing a plan limit this page has
 * not been told.
 */
export function MetaRow({
  children,
  hidden = false,
}: {
  children: ReactNode;
  /** Rendered nowhere at all — see the first-login card. */
  hidden?: boolean;
}) {
  if (hidden) return null;
  return (
    <Stack
      component="dl"
      direction={{ xs: 'column', sm: 'row' }}
      spacing={{ xs: 2, sm: 3 }}
      sx={{ m: 0 }}
      divider={
        <Box
          aria-hidden
          sx={{
            display: { xs: 'none', sm: 'block' },
            width: '1px',
            alignSelf: 'stretch',
            backgroundColor: alpha(color.stroke.default, 0.45),
          }}
        />
      }
    >
      {children}
    </Stack>
  );
}

/**
 * A label-over-value pair from the API Key card's metadata row.
 *
 * The design shows four — Key ID, Issued, Last rotated, Discord account — and
 * this build renders TWO of them. `GET /key` returns an id, a name and the
 * value (see `api/portal.ts`); it returns no timestamps at all, so "Issued" and
 * "Last rotated" would have to be invented. A dashboard that makes up the date
 * a credential was created is worse than one that does not show it.
 */
export function MetaField({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <Stack spacing={0.5} sx={{ minWidth: 0 }}>
      <Typography
        component="dt"
        variant="body2"
        sx={{ color: color.text.tertiary }}
      >
        {label}
      </Typography>
      <Box
        component="dd"
        sx={{
          m: 0,
          ...({ fontFamily: font.secondary } as object),
          fontWeight: 700,
          fontSize: '0.9375rem',
          color: color.text.primary,
          overflowWrap: 'anywhere',
        }}
      >
        {children}
      </Box>
    </Stack>
  );
}
