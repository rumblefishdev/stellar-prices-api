import ArticleOutlinedIcon from '@mui/icons-material/ArticleOutlined';
import CodeRoundedIcon from '@mui/icons-material/CodeRounded';
import DescriptionOutlinedIcon from '@mui/icons-material/DescriptionOutlined';
import ErrorOutlineRoundedIcon from '@mui/icons-material/ErrorOutlineRounded';
import KeyOutlinedIcon from '@mui/icons-material/KeyOutlined';
import TaskAltRoundedIcon from '@mui/icons-material/TaskAltRounded';
import Box from '@mui/material/Box';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import type { SvgIconComponent } from '@mui/icons-material';

import { color, radius } from '../theme/tokens';
import { OPENAPI_JSON, QUICKSTART, SWAGGER_UI } from './links';
import { Section, SectionHeading, cardBorder, cardSurface } from './primitives';

/**
 * "Everything developers need" — six doors into the documentation.
 *
 * Every card is a LINK, not a panel of prose, and they all point at the same
 * place today: the OpenAPI document, which is the only documentation artefact
 * actually served (see `links.ts`). Task 0163 writes the quickstart and task
 * 0195 mounts Swagger UI; when they land, the three constants in `links.ts`
 * diverge and these cards start pointing at six real destinations.
 *
 * Cards rather than a list because that is what the design does, and links
 * rather than `<div>`s because a card that describes a document and cannot open
 * it is the kind of thing that makes a reviewer stop trusting the page.
 */

type Doc = {
  icon: SvgIconComponent;
  title: string;
  body: string;
  href: string;
};

const DOCS: readonly Doc[] = [
  {
    icon: ArticleOutlinedIcon,
    title: 'Quick Start',
    body: 'Get your first live response in under 5 minutes. Covers authentication and the most common endpoint.',
    href: QUICKSTART,
  },
  {
    icon: KeyOutlinedIcon,
    title: 'Authentication',
    body: 'How API keys work, where to pass them, and what to expect when a key is missing or rate-limited.',
    href: SWAGGER_UI,
  },
  {
    icon: TaskAltRoundedIcon,
    title: 'Example Requests',
    body: 'Copy-ready curl commands for every endpoint. Test against the live API from your terminal.',
    href: SWAGGER_UI,
  },
  {
    icon: DescriptionOutlinedIcon,
    title: 'OpenAPI Specification',
    body: 'Full Swagger UI included. Explore, test and generate client code directly from the spec.',
    href: OPENAPI_JSON,
  },
  {
    icon: CodeRoundedIcon,
    title: 'SDK Examples',
    body: 'Working code snippets in four languages to copy into your project.',
    href: SWAGGER_UI,
  },
  {
    icon: ErrorOutlineRoundedIcon,
    title: 'Rate Limits',
    body: 'How throttling works, what headers to watch, and how to handle 429 responses gracefully.',
    href: SWAGGER_UI,
  },
];

export function Documentation() {
  return (
    <Section tone="base" id="docs">
      <Stack spacing={{ xs: 4, md: 6 }}>
        <SectionHeading
          align="left"
          label="Documentation"
          title="Everything developers need"
          subtitle="From your first curl to production deployment."
        />

        <Box
          sx={{
            display: 'grid',
            gap: 2,
            gridTemplateColumns: {
              xs: '1fr',
              sm: 'repeat(2, 1fr)',
              lg: 'repeat(3, 1fr)',
            },
          }}
        >
          {DOCS.map(({ icon: Icon, title, body, href }) => (
            <Stack
              key={title}
              component={Link}
              href={href}
              underline="none"
              spacing={1.5}
              sx={{
                p: 3,
                height: '100%',
                borderRadius: `${radius.lg}px`,
                border: cardBorder,
                backgroundColor: cardSurface('deep'),
                color: 'inherit',
                transition: 'border-color 120ms ease',
                '&:hover': { borderColor: color.stroke.default },
                '&:focus-visible': {
                  outline: `2px solid ${color.stroke.action}`,
                  outlineOffset: 2,
                },
              }}
            >
              <Box
                aria-hidden
                sx={{
                  width: 32,
                  height: 32,
                  borderRadius: '50%',
                  display: 'grid',
                  placeItems: 'center',
                  backgroundColor: color.primary[100],
                  color: color.primary[950],
                }}
              >
                <Icon sx={{ fontSize: 18 }} />
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
