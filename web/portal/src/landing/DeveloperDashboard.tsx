import AddRoundedIcon from '@mui/icons-material/AddRounded';
import BarChartRoundedIcon from '@mui/icons-material/BarChartRounded';
import LockOutlinedIcon from '@mui/icons-material/LockOutlined';
import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import type { SvgIconComponent } from '@mui/icons-material';

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

const CLAIMS: readonly {
  icon: SvgIconComponent;
  title: string;
  body: string;
}[] = [
  {
    icon: LockOutlinedIcon,
    title: 'API Key Management',
    body: 'View and copy your key at any time. Rotate once per month if needed.',
  },
  {
    icon: BarChartRoundedIcon,
    title: 'Usage Dashboard',
    body: 'Live request count against your monthly quota with reset date shown clearly.',
  },
  {
    icon: AddRoundedIcon,
    title: 'Quick Start Guide',
    body: 'Copy-ready curl examples and SDK snippets shown right next to your key.',
  },
];

export function DeveloperDashboard() {
  return (
    <GradientSection id="dashboard-preview" from="base" to="alt">
      <Stack
        direction={{ xs: 'column', md: 'row' }}
        spacing={{ xs: 5, md: 10 }}
        alignItems="center"
      >
        <Box sx={{ flex: '1 1 50%', minWidth: 0, width: '100%' }}>
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

        <Stack spacing={4} sx={{ flex: '1 1 50%', minWidth: 0 }}>
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
            {CLAIMS.map(({ icon: Icon, title, body }) => (
              <Stack
                component="li"
                key={title}
                direction="row"
                spacing={2}
                alignItems="flex-start"
              >
                <Box
                  aria-hidden
                  sx={{
                    flexShrink: 0,
                    width: 32,
                    height: 32,
                    borderRadius: '8px',
                    display: 'grid',
                    placeItems: 'center',
                    backgroundColor: color.accent.blue[100],
                    color: color.accent.blue[900],
                  }}
                >
                  <Icon sx={{ fontSize: 18 }} />
                </Box>
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
