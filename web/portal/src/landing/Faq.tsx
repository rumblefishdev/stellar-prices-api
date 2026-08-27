import ExpandMoreRoundedIcon from '@mui/icons-material/ExpandMoreRounded';
import Accordion from '@mui/material/Accordion';
import AccordionDetails from '@mui/material/AccordionDetails';
import AccordionSummary from '@mui/material/AccordionSummary';
import Box from '@mui/material/Box';
import Link from '@mui/material/Link';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import type { ReactNode } from 'react';

import { color, radius } from '../theme/tokens';
import { STELLAR_DISCORD_INVITE, API_REFERENCE } from './links';
import { Section, SectionHeading, cardBorder, cardSurface } from './primitives';

/**
 * "Common questions" — eight accordions.
 *
 * ⚠️ **The QUESTIONS are the design's; the ANSWERS are written here and need a
 * product read.** The Figma frame shows every row collapsed, so it supplies no
 * answer text at all — and shipping eight rows that open onto nothing would be
 * worse than not shipping the section.
 *
 * So each answer below restates something this repo has already decided
 * somewhere else, and nothing more: the eligibility rule and the invite are
 * task 0189's, the 1 req/s and 100,000/month figures are task 0157's and
 * 0188's, "no manual approval" is the epic's premise, and the replacement cap
 * is the rule task 0191 settled. **No answer invents a policy.** Two are worth
 * a second pair of eyes before this goes in front of anyone — "Can I increase
 * my quota?", where no mechanism exists today, and "Can I rotate my API key?",
 * whose wording belongs to task 0191 and should be taken from there once that
 * slice lands rather than paraphrased here.
 */

type Faq = { question: string; answer: ReactNode };

const FAQS: readonly Faq[] = [
  {
    question: 'How do I get an API key?',
    answer: (
      <>
        Sign in with Discord and the key is issued immediately. Two things are
        checked when you ask for one: membership of the{' '}
        <Link href={STELLAR_DISCORD_INVITE}>Stellar Developers Discord</Link>,
        and a Discord account that is not brand new.
      </>
    ),
  },
  {
    question: 'Do I need approval?',
    answer:
      'No. There is no form, no queue and no manual review — eligibility is checked automatically through Discord when you ask for a key.',
  },
  {
    question: 'What are the rate limits?',
    answer:
      'One request per second per key, and 100,000 requests per month. The monthly quota resets on the 1st of each month at 00:00 UTC.',
  },
  {
    question: 'Can I increase my quota?',
    answer:
      'Not today — the free tier is the only plan, and everyone is on the same limits. If your project needs more, get in touch and tell us what you are building.',
  },
  {
    question: 'Is the API free?',
    answer:
      'Yes. There is no paid tier and no credit card is required. The free tier is the whole product.',
  },
  {
    question: 'How often are prices updated?',
    answer:
      'Prices come straight from Soroswap liquidity pools and are updated on every block. Usage figures on your dashboard are reported by AWS with a short delay, so requests from the last few minutes may not be counted yet.',
  },
  {
    question: 'Where is the documentation?',
    answer: (
      <>
        The full <Link href={API_REFERENCE}>OpenAPI specification</Link> covers
        every endpoint and every response shape. It is linked from your
        dashboard too, next to your key.
      </>
    ),
  },
  {
    question: 'Can I rotate my API key?',
    answer:
      'Yes — you can replace your key once per quota period. The replacement is issued straight away and the old key stops working.',
  },
];

export function Faq() {
  return (
    <Section tone="alt" id="faq">
      <Stack spacing={{ xs: 4, md: 6 }} alignItems="center">
        <SectionHeading label="FAQ" title="Common questions" />

        {/* Two columns that become one, filled COLUMN-first so the reading
            order matches the design's. A CSS grid filled row-first would put
            question 2 top-right, which is what the mock shows — so row-first
            it is, and the DOM order is the reading order at every width. */}
        <Box
          sx={{
            display: 'grid',
            gap: 2,
            width: '100%',
            gridTemplateColumns: { xs: '1fr', md: 'repeat(2, 1fr)' },
          }}
        >
          {FAQS.map(({ question, answer }) => (
            <Accordion
              key={question}
              disableGutters
              square={false}
              sx={{
                alignSelf: 'flex-start',
                borderRadius: `${radius.md}px`,
                border: cardBorder,
                backgroundColor: cardSurface('deep'),
                backgroundImage: 'none',
                '&::before': { display: 'none' },
                '&.Mui-expanded': { margin: 0 },
              }}
            >
              <AccordionSummary
                expandIcon={
                  <ExpandMoreRoundedIcon sx={{ color: color.text.tertiary }} />
                }
                sx={{ px: 2.5, py: 1 }}
              >
                {/* `h3`, so the eight questions are a navigable list under the
                    section's `h2` rather than eight anonymous buttons. */}
                <Typography variant="h6" component="h3" color="text.primary">
                  {question}
                </Typography>
              </AccordionSummary>
              <AccordionDetails sx={{ px: 2.5, pb: 2.5, pt: 0 }}>
                <Typography variant="body2" sx={{ color: color.text.tertiary }}>
                  {answer}
                </Typography>
              </AccordionDetails>
            </Accordion>
          ))}
        </Box>
      </Stack>
    </Section>
  );
}
