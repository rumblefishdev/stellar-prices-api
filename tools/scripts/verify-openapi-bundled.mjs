#!/usr/bin/env node
//
// Fail when the OpenAPI document committed into the portal bundle has drifted
// from the one the API serves.
//
// WHY THIS EXISTS
// ---------------
// Task 0270 ships `web/portal/public/openapi.json` so the "OpenAPI JSON" link
// on the reference page can carry `download`: browsers ignore that attribute
// on a cross-origin href, and on the shared host (task 0194) the API is on
// another hostname, so the only downloadable copy is one inside the bundle.
//
// A second copy of a document is a copy that can go stale, and a stale spec is
// worse than no spec: a generator built from it produces a client for
// endpoints that no longer exist. Nothing else notices — the rendered
// reference still fetches the live document (D-02), so the page looks right
// while the file behind the link is months old.
//
// The comparison is on PARSED JSON, not bytes: what matters is that the
// document says the same thing, and holding the committed file to the exact
// serialization of whichever `serde_json` build produced it would fail this
// gate on a formatting change that no reader can see.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '../..');
const extracted = join(repoRoot, 'target/openapi.json');
const bundled = join(repoRoot, 'web/portal/public/openapi.json');

/** Key order is not part of the contract; the content is. */
const sorted = (value) =>
  Array.isArray(value)
    ? value.map(sorted)
    : value && typeof value === 'object'
      ? Object.fromEntries(
          Object.keys(value)
            .sort()
            .map((k) => [k, sorted(value[k])]),
        )
      : value;

const read = (path) => {
  try {
    return sorted(JSON.parse(readFileSync(path, 'utf8')));
  } catch (error) {
    console.error(`error: cannot read ${path}: ${error.message}`);
    process.exit(1);
  }
};

if (JSON.stringify(read(extracted)) === JSON.stringify(read(bundled))) {
  console.log('web/portal/public/openapi.json matches the extracted document');
  process.exit(0);
}

console.error(
  [
    'error: the OpenAPI document in the portal bundle has drifted.',
    '',
    `  served:  ${extracted} (npm run openapi:extract)`,
    `  bundled: ${bundled}`,
    '',
    'This file is what the "OpenAPI JSON" link on /api/docs hands a reader.',
    'Refresh it and commit the result:',
    '',
    '  npm run openapi:extract && cp target/openapi.json web/portal/public/openapi.json',
  ].join('\n'),
);
process.exit(1);
