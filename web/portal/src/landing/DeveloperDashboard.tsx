import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';

import apiKeyIcon from '../assets/icons/dashboard-api-key.svg';
import quickStartIcon from '../assets/icons/dashboard-quick-start.svg';
import usageIcon from '../assets/icons/dashboard-usage.svg';
import { color, radius } from '../theme/tokens';
import { KeyField, PanelHeader, UsageMeter } from './DashboardPanel';
import { GradientSection, SectionHeading, WindowCard } from './primitives';

/**
 * "Everything in one place" — a picture of the dashboard, beside three claims.
 *
 * The card is built from the SAME components the real `/dashboard` renders
 * (`DashboardPanel.tsx`), with sample values instead of a session. That is the
 * point: the promise this section makes is checkable by signing in, and a
 * preview drawn separately would be free to drift from the thing it previews.
 */

const SAMPLE_KEY = 'sf_live_k8mN2pQxRvLzW9aTbYcUoJeHdFgIsSw4...';
const SAMPLE_USED = 42_180;
const SAMPLE_LIMIT = 100_000;

/**
 * **The exported Figma badges** (Adam's `developerdashboard.zip`), disc and
 * glyph baked into each file at 32×32.
 *
 * These three are rounded SQUARES, where the feature grid's are circles —
 * that is what the design draws, and it survives a round of review that made
 * them discs against Material stand-ins. The shape is the design's, not a
 * leftover.
 */
const CLAIMS: readonly { icon: string; title: string; body: string }[] = [
  {
    icon: apiKeyIcon,
    title: 'API Key Management',
    body: 'View and copy your key at any time. Rotate once per month if needed.',
  },
  {
    icon: usageIcon,
    title: 'Usage Dashboard',
    body: 'Live request count against your monthly quota with reset date shown clearly.',
  },
  {
    icon: quickStartIcon,
    title: 'Quick Start Guide',
    body: 'Copy-ready curl examples and SDK snippets shown right next to your key.',
  },
];

export function DeveloperDashboard() {
  return (
    <GradientSection id="dashboard-preview" from="base" to="alt">
      <Stack
        direction={{ xs: 'column', md: 'row' }}
        // 80px between the claims and the preview at every width. Stacked, the
        // two halves are a block of text and a picture of a screen with its
        // own border; at 40px the picture read as belonging to the last claim
        // above it rather than to the section.
        spacing={10}
        // ⚠️ REQUIRED here, not a style preference. A `Stack` spaces with
        // `margin` on every child but the first IN DOM ORDER, and this one
        // swaps its two halves with `order` at `xs` — so the margin landed on
        // the text block, which `order` had moved to the top, putting 80px
        // above the heading and nothing at all between the text and the
        // preview. `gap` is laid out in VISUAL order, so it survives the swap.
        useFlexGap
        alignItems="center"
      >
        {/* Second on a phone, first on a desktop. The mobile frame leads with
            the heading and the three claims and puts the preview under them:
            stacked, a picture of a dashboard above the sentence saying what
            it is reads as a screenshot that failed to caption itself. */}
        <Box
          sx={{
            flex: '1 1 50%',
            minWidth: 0,
            width: '100%',
            order: { xs: 2, md: 1 },
          }}
        >
          {/* `aria-hidden`: it is a screenshot of a screen the visitor can go
              and see, and the three claims to its right already say in words
              what it shows. Reading out a sample API key would be noise. */}
          <Box aria-hidden>
            <WindowCard title="API Key">
              <Stack spacing={2} sx={{ p: 3 }}>
                <PanelHeader
                  title="My API Key"
                  status={{ label: 'Active', tone: 'ok' }}
                />
                <KeyField label="API Key" value={SAMPLE_KEY} />
                <Box
                  sx={{
                    p: 2,
                    borderRadius: `${radius.md}px`,
                    border: `1px solid ${color.surface.gray}`,
                    backgroundColor: color.surface.backgroundAlt,
                  }}
                >
                  <UsageMeter
                    used={SAMPLE_USED}
                    limit={SAMPLE_LIMIT}
                    resetLabel="Resets May 1"
                  />
                </Box>
              </Stack>
            </WindowCard>
          </Box>
        </Box>

        <Stack
          spacing={4}
          sx={{ flex: '1 1 50%', minWidth: 0, order: { xs: 1, md: 2 } }}
        >
          <SectionHeading
            align="left"
            label="Developer Dashboard"
            title="Everything in one place"
            subtitle="Your API key, usage stats and documentation — all from one dashboard."
          />

          <Stack
            component="ul"
            spacing={3}
            sx={{ m: 0, p: 0, listStyle: 'none' }}
          >
            {CLAIMS.map(({ icon, title, body }) => (
              <Stack
                component="li"
                key={title}
                direction="row"
                spacing={2}
                alignItems="flex-start"
              >
                <Box
                  component="img"
                  src={icon}
                  alt=""
                  aria-hidden
                  sx={{
                    flexShrink: 0,
                    width: 32,
                    height: 32,
                    display: 'block',
                  }}
                />
                <Stack spacing={0.5}>
                  <Typography variant="h5" component="h3" color="text.primary">
                    {title}
                  </Typography>
                  <Typography
                    variant="body2"
                    sx={{ color: color.text.tertiary }}
                  >
                    {body}
                  </Typography>
                </Stack>
              </Stack>
            ))}
          </Stack>
        </Stack>
      </Stack>
    </GradientSection>
  );
}
