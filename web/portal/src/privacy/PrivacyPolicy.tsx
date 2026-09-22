import Box from '@mui/material/Box';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import type { ReactNode } from 'react';

import { DocPage, DocSection } from '../landing/DocPrimitives';
import { color, font } from '../theme/tokens';
import { parsePolicy, type Block } from './policy';
import policyMd from './privacy-policy.md?raw';

/**
 * The privacy policy (task 0303), rendered from `privacy-policy.md` — the
 * document that was reviewed, not a retyping of it. Same page shape as the
 * API reference and the quick start: a rail of the sixteen sections on the
 * left, the text on the right. The draft's two opening paragraphs are the
 * page's lede; each section's first paragraph is its lede.
 */
export const POLICY = parsePolicy(policyMd);

/**
 * The date of the text in `privacy-policy.md`. The document carries none of
 * its own, and a policy without one cannot say which version a visitor read
 * — bump it with every edit to the file.
 */
export const POLICY_DATED = '22 September 2026';

/**
 * `**bold**`, `` `code` `` and the hard line breaks the draft writes with two
 * trailing spaces (the postal address) — the only inline marks the policy
 * uses. Anything else is text.
 */
export function inline(text: string): ReactNode[] {
  return text.split('\n').flatMap((line, i) => [
    ...(i ? [<br key={`br-${i}`} />] : []),
    ...line
      .split(/(\*\*[^*]+\*\*|`[^`]+`)/)
      .filter(Boolean)
      .map((part, j) =>
        part.startsWith('**') ? (
          <strong key={`${i}-${j}`}>{part.slice(2, -2)}</strong>
        ) : part.startsWith('`') ? (
          <code key={`${i}-${j}`}>{part.slice(1, -1)}</code>
        ) : (
          part
        ),
      ),
  ]);
}

function BlockView({ block }: { block: Block }) {
  switch (block.kind) {
    case 'h3':
      return (
        <Typography variant="h5" component="h3" color="text.primary">
          {block.text}
        </Typography>
      );
    case 'p':
      return (
        <Typography variant="body1" sx={{ color: color.text.secondary }}>
          {inline(block.text)}
        </Typography>
      );
    case 'ul':
      return (
        <Stack
          component="ul"
          spacing={1}
          sx={{ m: 0, pl: 3, color: color.text.secondary }}
        >
          {block.items.map((item, i) => (
            <Typography key={i} component="li" variant="body1">
              {inline(item)}
            </Typography>
          ))}
        </Stack>
      );
  }
}

export default function PrivacyPolicy() {
  return (
    <DocPage
      sections={POLICY.sections.map((s) => ({ id: s.id, label: s.title }))}
      eyebrow={
        <Typography
          variant="overline"
          component="p"
          sx={{ color: color.text.tertiary }}
        >
          Version of {POLICY_DATED}
        </Typography>
      }
      title={POLICY.title}
      lede={POLICY.intro.map((p, i) => (
        <Box component="span" key={i} sx={{ display: 'block', mt: i ? 1 : 0 }}>
          {inline(p)}
        </Box>
      ))}
    >
      {POLICY.sections.map(({ id, number, title, blocks }) => {
        // The section's first paragraph is its lede, under the heading, as
        // the other doc pages draw theirs; the rest is the body.
        const [first, ...rest] = blocks;
        const opensWithText = first?.kind === 'p';
        return (
          <DocSection
            key={id}
            id={id}
            title={`${number}. ${title}`}
            lede={opensWithText ? inline(first.text) : ''}
          >
            <Stack
              spacing={2}
              sx={{
                '& code': { fontFamily: font.mono, fontSize: '0.875em' },
                '& strong': { color: color.text.primary, fontWeight: 600 },
              }}
            >
              {(opensWithText ? rest : blocks).map((block, i) => (
                <BlockView key={i} block={block} />
              ))}
            </Stack>
          </DocSection>
        );
      })}
    </DocPage>
  );
}
