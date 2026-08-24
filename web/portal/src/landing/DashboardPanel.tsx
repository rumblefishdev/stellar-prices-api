import Box from '@mui/material/Box';
import LinearProgress from '@mui/material/LinearProgress';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import type { ReactNode } from 'react';

import { color, font, radius } from '../theme/tokens';
import { cardBorder } from './primitives';

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
  actions,
}: {
  label: string;
  value: ReactNode;
  testId?: string;
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
          border: `1px solid ${color.stroke.action}`,
          backgroundColor: color.surface.backgroundAlt,
        }}
      >
        <Typography variant="body2" sx={{ color: color.text.tertiary }}>
          {label}
        </Typography>
        <Box
          component="code"
          data-testid={testId}
          sx={{
            fontFamily: font.mono,
            fontSize: '0.9375rem',
            color: color.text.accent,
            // A 40-character opaque string must wrap rather than widen the
            // card — this renders at 375 px too.
            overflowWrap: 'anywhere',
            minWidth: 0,
          }}
        >
          {value}
        </Box>
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
              color: full ? color.text.error : color.text.accent,
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
              sx={{ color: color.text.secondary }}
            >
              <Typography
                variant="body2"
                sx={{ color: full ? color.text.error : 'inherit' }}
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
              height: 8,
              borderRadius: `${radius.pill}px`,
              backgroundColor: color.gray[50],
              '& .MuiLinearProgress-bar': {
                borderRadius: `${radius.pill}px`,
                // Red at the ceiling. The bar is the fastest read on the card,
                // and "you have run out" should not arrive in the same colour
                // as "you have plenty left".
                backgroundColor: full ? color.red[400] : color.primary[400],
              },
            }}
          />

          <Stack
            direction="row"
            justifyContent="space-between"
            spacing={2}
            sx={{ color: color.text.secondary }}
          >
            {!headline && (
              <Typography variant="body2" color="inherit">
                {percent}% used
              </Typography>
            )}
            {resetLabel && (
              <Typography variant="body2" color="inherit">
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
  children: ReactNode;
  sx?: object;
}) {
  return (
    <Box
      component="section"
      sx={{
        borderRadius: `${radius.lg}px`,
        border: cardBorder,
        backgroundColor: color.surface.grayAlt,
        overflow: 'hidden',
        ...sx,
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        spacing={1.5}
        sx={{ px: 3, py: 2, borderBottom: cardBorder }}
      >
        <Typography variant="h5" component="h2" color="text.primary">
          {title}
        </Typography>
        {status && <StatusPill {...status} />}
      </Stack>
      <Stack spacing={2.5} sx={{ p: 3 }}>
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
  const skin = {
    ok: { fg: color.text.success, bg: alpha(color.surface.success, 0.55) },
    bad: { fg: color.text.error, bg: alpha('#82181a', 0.55) },
    muted: { fg: color.text.tertiary, bg: color.surface.gray },
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
      <Typography variant="body2" sx={{ color: color.text.tertiary }}>
        {label}
      </Typography>
      <Box
        sx={{
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
