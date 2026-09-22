/**
 * The privacy policy's text is `privacy-policy.md`, kept as the document that
 * was reviewed rather than retyped into JSX — a diff of the file is a diff of
 * the policy. This reads it back. It is not a Markdown parser: the policy
 * uses `#`/`##`/`###` headings, paragraphs, `*` bullets, `**bold**`, `` `code` ``
 * and backslash escapes (`1\.`, `\+48`), and that is all this understands. A
 * construct it does not know renders as its literal text, which the spec
 * would show.
 */
export type Block =
  | { kind: 'h3'; text: string }
  | { kind: 'p'; text: string }
  | { kind: 'ul'; items: string[] };

export type Section = {
  /** The heading as a URL fragment — the rail's target. */
  id: string;
  /** The heading's own numeral, so the rail and the text agree on it. */
  number: number;
  /** The heading without its numeral or the `**` it is wrapped in. */
  title: string;
  blocks: Block[];
};

export type Policy = {
  title: string;
  /** The paragraphs between the title and the first section. */
  intro: string[];
  sections: Section[];
};

/** Markdown's backslash escapes, as the draft writes `1\.` and `\+48`. */
const clean = (s: string) =>
  s.replace(/\\([\\`*_{}[\]()#+\-.!])/g, '$1').trim();
/** A heading wrapped in `**…**`, as every heading of the draft is. */
const heading = (s: string) => clean(s).replace(/^\*\*(.*)\*\*$/, '$1');
const slug = (s: string) =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');

export function parsePolicy(md: string): Policy {
  const policy: Policy = { title: '', intro: [], sections: [] };
  let para: string[] = [];
  let list: string[] = [];
  const current = () => policy.sections[policy.sections.length - 1];

  const flush = () => {
    if (para.length) {
      const text = para.join('\n');
      if (current()) current().blocks.push({ kind: 'p', text });
      else policy.intro.push(text);
      para = [];
    }
    if (list.length) {
      current()?.blocks.push({ kind: 'ul', items: list });
      list = [];
    }
  };

  for (const raw of md.split('\n')) {
    const line = raw.trimEnd();
    if (!line) {
      flush();
    } else if (line.startsWith('# ')) {
      policy.title = heading(line.slice(2));
    } else if (line.startsWith('## ')) {
      flush();
      const text = heading(line.slice(3));
      const numbered = /^(\d+)\.\s+(.*)$/.exec(text);
      const title = numbered ? numbered[2] : text;
      policy.sections.push({
        id: slug(title),
        number: numbered ? Number(numbered[1]) : policy.sections.length + 1,
        title,
        blocks: [],
      });
    } else if (line.startsWith('### ')) {
      flush();
      current()?.blocks.push({ kind: 'h3', text: heading(line.slice(4)) });
    } else {
      const bullet = /^\s*\*\s+(.*)$/.exec(line);
      if (bullet) {
        if (para.length) flush();
        list.push(clean(bullet[1]));
      } else {
        if (list.length) flush();
        para.push(clean(line));
      }
    }
  }
  flush();
  return policy;
}
