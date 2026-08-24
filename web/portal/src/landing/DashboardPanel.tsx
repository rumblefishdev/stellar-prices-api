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
    <InsetPanel>
      <Typography variant="body2" sx={{ color: color.text.tertiary }}>
        {label}
      </Typography>
      <Stack
        direction={{ xs: 'column', sm: 'row' }}
        spacing={1.5}
        alignItems={{ xs: 'stretch', sm: 'center' }}
        justifyContent="space-between"
      >
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
        {actions && (
          <Stack direction="row" spacing={1} sx={{ flexShrink: 0 }}>
            {actions}
          </Stack>
        )}
      </Stack>
    </InsetPanel>
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
  resetLabel,
}: {
  used: number | null;
  limit: number | null;
  resetLabel?: ReactNode;
}) {
  const known = used !== null && limit !== null && limit > 0;
  const percent = known ? Math.min(100, Math.round((used / limit) * 100)) : 0;

  return (
    <Stack spacing={1.5}>
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

      {known && (
        <>
          <LinearProgress
            variant="determinate"
            value={percent}
            // The figures beside it are the accessible version; the bar is a
            // second rendering of the same fact and would otherwise be read
            // out twice.
            aria-hidden
            sx={{
              height: 8,
              borderRadius: `${radius.pill}px`,
              backgroundColor: color.gray[50],
              '& .MuiLinearProgress-bar': {
                borderRadius: `${radius.pill}px`,
                backgroundColor: color.primary[400],
              },
            }}
          />
          <Stack
            direction="row"
            justifyContent="space-between"
            spacing={2}
            sx={{ color: color.text.secondary }}
          >
            <Typography variant="body2" color="inherit">
              {percent}% used
            </Typography>
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
