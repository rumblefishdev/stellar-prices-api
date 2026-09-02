import Box from '@mui/material/Box';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';

import authenticationIcon from '../assets/icons/docs-authentication.svg';
import exampleRequestsIcon from '../assets/icons/docs-example-requests.svg';
import openApiIcon from '../assets/icons/docs-openapi.svg';
import quickStartIcon from '../assets/icons/docs-quick-start.svg';
import rateLimitsIcon from '../assets/icons/docs-rate-limits.svg';
import sdkExamplesIcon from '../assets/icons/docs-sdk-examples.svg';
import { color, radius } from '../theme/tokens';
import { QUICKSTART, API_REFERENCE } from './links';
import {
  CardRail,
  Section,
  SectionHeading,
  cardBorder,
  cardSurface,
} from './primitives';

/**
 * "Everything developers need" — six doors into the documentation.
 *
 * Every card is a LINK, not a panel of prose. Two destinations exist today —
 * the quick start and the API reference (task 0195), both pages of this app
 * — and the cards split between them by what a reader would expect behind the
 * title. A card whose body promises curl commands, code snippets or throttling
 * advice goes to the quick start SECTION that holds them; a card about the
 * endpoint catalogue itself goes to the reference.
 *
 * This used to say the cards opened the reference "until task 0163's
 * walkthrough gives the quick start sections of its own to point at". The
 * sections have existed since task 0193 — `first-request`, `sdk`,
 * `rate-limits`, `authentication` are all in `QuickStart.tsx`'s `SECTIONS` —
 * so the wait was over before the sentence was written, and four cards spent
 * that time promising content the page they opened does not carry.
 *
 * Cards rather than a list because that is what the design does, and links
 * rather than `<div>`s because a card that describes a document and cannot open
 * it is the kind of thing that makes a reviewer stop trusting the page.
 */

type Doc = {
  icon: string;
  title: string;
  body: string;
  href: string;
};

const DOCS: readonly Doc[] = [
  {
    icon: quickStartIcon,
    title: 'Quick Start',
    body: 'Get your first live response in under 5 minutes. Covers authentication and the most common endpoint.',
    href: QUICKSTART,
  },
  {
    icon: authenticationIcon,
    title: 'Authentication',
    body: 'How API keys work, where to pass them, and what to expect when a key is missing or rate-limited.',
    // Stays on the reference, unlike its three siblings below: the reference
    // opens with a "Base URL & authentication" strip naming the header, and
    // publishes the 401/403/429 shapes this body promises. Both pages answer
    // this card; the one that answers it in more detail wins.
    href: API_REFERENCE,
  },
  {
    icon: exampleRequestsIcon,
    title: 'Example Requests',
    body: 'Copy-ready curl commands for every endpoint. Test against the live API from your terminal.',
    // The reference generates examples from the schemas and deliberately has
    // no "try it" (task 0126 owns that decision). The curl commands this body
    // promises are the quick start's first request.
    href: `${QUICKSTART}#first-request`,
  },
  {
    icon: openApiIcon,
    title: 'OpenAPI Specification',
    body: 'Full API reference. Browse every endpoint, parameter and schema, and generate clients from the spec.',
    // The rendered reference, which links to the raw JSON in its own header;
    // a card promising Swagger UI used to open the bare document — and the
    // reference is ours now, so the copy no longer names someone else's tool
    // or promises a "try it" the page deliberately does not have (task 0126).
    href: API_REFERENCE,
  },
  {
    icon: sdkExamplesIcon,
    title: 'SDK Examples',
    body: 'Working code snippets in four languages to copy into your project.',
    // The four languages are the quick start's; the reference carries none.
    href: `${QUICKSTART}#sdk`,
  },
  {
    icon: rateLimitsIcon,
    title: 'Rate Limits',
    body: 'How throttling works, what headers to watch, and how to handle 429 responses gracefully.',
    // The reference publishes the 429 response; what to DO about it — the
    // headers, the backoff — is only written in the quick start.
    href: `${QUICKSTART}#rate-limits`,
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

        {/* A rail on a phone, like the feature grid — see `CardRail`. */}
        <CardRail columns={{ sm: 'repeat(2, 1fr)', lg: 'repeat(3, 1fr)' }}>
          {DOCS.map(({ icon, title, body, href }) => (
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
              {/* The exported Figma badge (Adam's `Documentations.zip`) —
                  pale yellow disc, brown glyph, one file. */}
              <Box
                component="img"
                src={icon}
                alt=""
                aria-hidden
                sx={{ width: 32, height: 32, display: 'block' }}
              />
              <Typography variant="h5" component="h3" color="text.primary">
                {title}
              </Typography>
              <Typography variant="body2" sx={{ color: color.text.tertiary }}>
                {body}
              </Typography>
            </Stack>
          ))}
        </CardRail>
      </Stack>
    </Section>
  );
}
