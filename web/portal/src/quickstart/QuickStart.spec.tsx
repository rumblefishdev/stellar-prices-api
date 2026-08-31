import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { SNIPPET_TABLES } from './QuickStart';

/**
 * Every snippet on the quick start is authored twice — as the coloured JSX a
 * reader sees, and as the plain string the Copy button writes. Nothing derived
 * one from the other, so nothing noticed when task 0194's `getPrices` →
 * `getPrice` rename reached `text` and only part of `view`: the JS snippet on
 * screen declared `prices` and logged `price` (`ReferenceError` for anyone who
 * retyped it), and the Rust one did not compile. Both Copy buttons wrote
 * correct code, which is exactly why the page could ship wrong.
 *
 * This is the tie. It renders each `view` and compares its text to `text`,
 * ignoring how whitespace is chunked between JSX children — that chunking is a
 * detail of how the tokens were split, not of the code being shown.
 */
const collapse = (value: string) => value.replace(/\s+/g, ' ').trim();

describe('quick-start snippets', () => {
  const cases = Object.entries(SNIPPET_TABLES).flatMap(([table, snippets]) =>
    Object.entries(snippets).map(
      ([lang, snippet]) => [`${table}.${lang}`, snippet] as const,
    ),
  );

  it('has a snippet to check', () => {
    expect(cases.length).toBeGreaterThan(0);
  });

  it.each(cases)(
    '%s renders the code its Copy button writes',
    (_name, snippet) => {
      // A wrapper element rather than a fragment: `view` is a `ReactNode` (a
    // list of tokens, not one element), and `render` needs something to mount.
    const { container } = render(<pre>{snippet.view}</pre>);
      expect(collapse(container.textContent ?? '')).toBe(
        collapse(snippet.text),
      );
    },
  );
});
