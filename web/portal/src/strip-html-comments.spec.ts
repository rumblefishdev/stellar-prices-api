import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { stripHtmlComments } from '../vite.config.mts';

/**
 * `index.html` is a public document on a page that renders an API key from task
 * 0187 on, and it is mostly commentary — which task adds sub-routes, how S3
 * answers a missing key, why there is no `<base href>`. Vite ships HTML comments
 * verbatim, so all of that was being served to anyone opening view-source.
 *
 * Nothing here is a secret. It is the roadmap and the bucket's miss behaviour,
 * which is free reconnaissance and buys nobody anything, so it is stripped at
 * build. The source keeps every word.
 */
const transform = (html: string): string => {
  const plugin = stripHtmlComments();
  const hook = plugin.transformIndexHtml as {
    handler: (html: string) => string;
  };
  return hook.handler(html);
};

describe('stripHtmlComments', () => {
  it('runs on build only, so `nx dev` still shows the commentary', () => {
    // `vite preview` serves the built output, so it sees the stripped document
    // exactly as CloudFront does.
    expect(stripHtmlComments().apply).toBe('build');
  });

  it('removes comments and leaves the markup that matters', () => {
    const out = transform(
      [
        '<!doctype html>',
        '<html lang="en">',
        '  <head>',
        '    <!-- a note about why -->',
        '    <title>Stellar Prices API — API keys</title>',
        '  </head>',
        '  <body>',
        '    <!-- multi',
        '         line -->',
        '    <div id="root"></div>',
        '  </body>',
        '</html>',
      ].join('\n'),
    );

    expect(out).not.toMatch(/<!--/);
    expect(out).not.toMatch(/a note about why/);
    expect(out).not.toMatch(/multi/);
    // The doctype opens with `<!` and must survive.
    expect(out).toMatch(/^<!doctype html>/);
    expect(out).toMatch(/<title>Stellar Prices API — API keys<\/title>/);
    expect(out).toMatch(/<div id="root"><\/div>/);
    // The comment takes its own indented line with it rather than leaving a
    // blank one behind.
    expect(out).not.toMatch(/\n[ \t]+\n/);
  });

  it('clears the real entry document, which is where the disclosure was', () => {
    const source = readFileSync(
      join(import.meta.dirname, '..', 'index.html'),
      'utf8',
    );

    // Guards the premise: if someone strips the comments from the source
    // instead, this test should stop claiming to prove anything.
    expect(source).toMatch(/<!--/);

    const out = transform(source);
    expect(out).not.toMatch(/<!--/);
    expect(out).not.toMatch(/task 0187/);
    expect(out).toMatch(/<div id="root"><\/div>/);
    expect(out).toMatch(/<link rel="icon"/);
  });
});
