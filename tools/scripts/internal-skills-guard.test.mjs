// `npx skills add rumblefishdev/stellar-prices-api` offers every SKILL.md it
// finds on the default branch (task 0322). Only `skills/*` is meant for the
// public; `.claude/skills` (our lore/ops tooling) and `.agents/skills` (Nx's)
// are hidden by `metadata: internal: true`, which the skills CLI honours.
//
// Nothing else keeps that marker: `nx configure-ai-agents`, which Nx suggests
// on every push, regenerates `.agents/skills` from its own templates and drops
// it, and a new internal skill starts without it. Either way the public list
// grows with every check green. Asserted here:
//
//   - every `.claude/skills/*/SKILL.md` and `.agents/skills/*/SKILL.md` is
//     marked internal;
//   - no `skills/*/SKILL.md` is, and there is at least one.
//
// After `nx configure-ai-agents`, re-add `metadata:` / `internal: true` to the
// regenerated files.
//
// Run: npx nx test @rumblefish/stellar-prices-api-aws-cdk

import assert from 'node:assert/strict';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

const skillFiles = (dir) =>
  existsSync(join(root, dir))
    ? readdirSync(join(root, dir))
        .map((name) => join(dir, name, 'SKILL.md'))
        .filter((file) => existsSync(join(root, file)))
    : [];

// ponytail: line-based frontmatter read, not a YAML parser; enough for the
// `metadata:` block with an indented `internal:` key that every file here uses.
const isInternal = (file) => {
  const text = readFileSync(join(root, file), 'utf8');
  const frontmatter = text.match(/^---\n([\s\S]*?)\n---/)?.[1] ?? '';
  return /^metadata:\n(?:[ \t]+.*\n)*?[ \t]+internal:[ \t]*("true"|true)[ \t]*$/m.test(
    frontmatter + '\n',
  );
};

test('internal skills are hidden from the skills CLI', () => {
  const internal = [
    ...skillFiles('.claude/skills'),
    ...skillFiles('.agents/skills'),
  ];
  assert.ok(internal.length > 0, 'no internal skills found');
  const unmarked = internal.filter((file) => !isInternal(file));
  assert.deepEqual(unmarked, [], 'missing metadata.internal: true');
});

test('public skills stay visible', () => {
  const published = skillFiles('skills');
  assert.ok(published.length > 0, 'no public skill under skills/');
  assert.deepEqual(
    published.filter(isInternal),
    [],
    'public skill marked internal',
  );
});
