import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';

import { color, font } from '../theme/tokens';
import { Section, SectionHeading } from './primitives';

/**
 * "Get started in under one minute" — the four steps, numbered.
 *
 * This section is the epic's story rendered as a promise, and the numbers in it
 * are checkable: step 2 says the key is "generated instantly, shown on screen",
 * which is exactly what task 0189's issue round-trip does and what the
 * dashboard shows. If any of these four stops being true, this is the section
 * that has to change — not the one that gets left as marketing.
 */

const STEPS: readonly { title: string; body: string }[] = [
  {
    title: 'Sign in with Discord',
    body: 'One click. No form to fill. Your Discord account is your identity.',
  },
  {
    title: 'Receive your key',
    body: 'Generated instantly, shown on screen. Always accessible from your dashboard.',
  },
  {
    title: 'Make your first call',
    body: 'Copy the key, paste it into the example request and get a live response.',
  },
  {
    title: 'Monitor usage',
    body: 'Track monthly requests against your quota from the dashboard.',
  },
];

export function SelfService() {
  // The glow is centred and high: this section's heading is centred, and the
  // light behind it is what separates the four steps from the band above.
  return (
    <Section
      tone="base"
      id="get-started"
      glow={{ at: '50% 18%', size: '60% 55%' }}
    >
      <Stack spacing={{ xs: 5, md: 9 }} alignItems="center">
        <SectionHeading
          label="Self-Service Portal"
          title={
            <>
              Get started in under{' '}
              <Box component="span" sx={{ color: color.text.accent }}>
                one minute
              </Box>
            </>
          }
          subtitle="No emails. No waiting. No manual approval. Sign in with Discord and your API key is ready immediately."
        />

        {/* An ordered list, because the steps ARE ordered — the numbers in the
            design are not decoration, and a screen reader should get the
            sequence without them. The rendered numerals are `aria-hidden` for
            the same reason: the list already announces its position. */}
        <Box
          component="ol"
          sx={{
            m: 0,
            p: 0,
            listStyle: 'none',
            display: 'grid',
            gap: { xs: 4, md: 3 },
            pt: { md: 2 },
            width: '100%',
            gridTemplateColumns: {
              xs: '1fr',
              sm: 'repeat(2, 1fr)',
              md: 'repeat(4, 1fr)',
            },
          }}
        >
          {STEPS.map(({ title, body }, index) => (
            <Stack
              component="li"
              key={title}
              spacing={1.5}
              alignItems="center"
              sx={{ textAlign: 'center' }}
            >
              <Box
                aria-hidden
                sx={{
                  width: 44,
                  height: 44,
                  borderRadius: '50%',
                  display: 'grid',
                  placeItems: 'center',
                  border: `1px solid ${color.stroke.action}`,
                  color: color.text.accent,
                  fontFamily: font.mono,
                  fontSize: '0.875rem',
                  fontWeight: 500,
                }}
              >
                {String(index + 1).padStart(2, '0')}
              </Box>
              <Typography variant="h5" component="h3" color="text.primary">
                {title}
              </Typography>
              <Typography variant="body2" sx={{ color: color.text.tertiary }}>
                {body}
              </Typography>
            </Stack>
          ))}
        </Box>
      </Stack>
    </Section>
  );
}
