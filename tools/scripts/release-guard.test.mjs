// Production changes only from the release commit (task 0343, step 13):
//
//   - `check-release-commit.sh` refuses a dirty tree, an untracked file, a
//     HEAD that is not the freshly fetched tip of `origin/deploy/production`,
//     and a remote it cannot reach;
//   - `infra/Makefile` runs it FIRST in every `deploy-production*`,
//     `destroy-production*` and `sync-portal-explorer` target, and not in
//     `diff-production`, which a release PR takes from `develop`.
//
// Run: npx nx test @rumblefish/stellar-prices-api-aws-cdk

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..');
const guardScript = join(here, 'check-release-commit.sh');

// --- the script, against throwaway repositories ----------------------------

const scratch = [];
after(() => {
  for (const dir of scratch) rmSync(dir, { recursive: true, force: true });
});

// No user or system git config: a signing or hook setting on the operator's
// machine must not decide these tests.
const gitEnv = {
  ...process.env,
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_AUTHOR_NAME: 'test',
  GIT_AUTHOR_EMAIL: 'test@example.invalid',
  GIT_COMMITTER_NAME: 'test',
  GIT_COMMITTER_EMAIL: 'test@example.invalid',
};

const git = (cwd, ...args) => {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8', env: gitEnv });
  assert.equal(result.status, 0, `git ${args.join(' ')}: ${result.stderr}`);
  return result.stdout.trim();
};

const commit = (cwd, file, content) => {
  writeFileSync(join(cwd, file), content);
  git(cwd, 'add', file);
  git(cwd, 'commit', '-q', '-m', `write ${file}`);
};

// A bare `origin` with `develop` one commit ahead of `deploy/production`, a
// publisher clone to move the branches, and the operator's clone, checked out
// at the release tip and clean.
const fixture = () => {
  const dir = mkdtempSync(join(tmpdir(), 'release-guard-'));
  scratch.push(dir);
  const origin = join(dir, 'origin.git');
  const publisher = join(dir, 'publisher');
  const operator = join(dir, 'operator');

  git(dir, 'init', '-q', '--bare', '-b', 'develop', origin);
  git(dir, 'clone', '-q', origin, publisher);
  commit(publisher, 'app.txt', 'v1\n');
  git(publisher, 'push', '-q', 'origin', 'HEAD:refs/heads/develop');
  git(publisher, 'push', '-q', 'origin', 'HEAD:refs/heads/deploy/production');
  commit(publisher, 'app.txt', 'v2\n');
  git(publisher, 'push', '-q', 'origin', 'HEAD:refs/heads/develop');

  git(dir, 'clone', '-q', origin, operator);
  git(operator, 'checkout', '-q', '--detach', 'origin/deploy/production');
  return { origin, publisher, operator };
};

const guard = (root) =>
  spawnSync('bash', [guardScript, root], { encoding: 'utf8', env: gitEnv });

test('the release tip, clean, passes', () => {
  const { operator } = fixture();
  const result = guard(operator);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /HEAD is origin\/deploy\/production/);
});

test('a modified file is refused', () => {
  const { operator } = fixture();
  writeFileSync(join(operator, 'app.txt'), 'edited\n');
  const result = guard(operator);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /working tree is not clean/);
  assert.match(result.stderr, /app\.txt/);
});

test('an untracked file is refused', () => {
  const { operator } = fixture();
  writeFileSync(join(operator, 'new-handler.rs'), 'fn main() {}\n');
  const result = guard(operator);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /new-handler\.rs/);
});

test('an ignored file is not', () => {
  const { operator } = fixture();
  writeFileSync(join(operator, '.git', 'info', 'exclude'), 'target/\n');
  mkdirSync(join(operator, 'target'));
  writeFileSync(join(operator, 'target', 'bootstrap'), 'built\n');
  const result = guard(operator);
  assert.equal(result.status, 0, result.stderr);
});

test('develop is refused, with the commands that fix it', () => {
  const { operator } = fixture();
  git(operator, 'checkout', '-q', 'develop');
  const result = guard(operator);
  assert.equal(result.status, 1);
  assert.match(
    result.stderr,
    /HEAD is not the tip of origin\/deploy\/production/,
  );
  assert.match(
    result.stderr,
    /git checkout --detach origin\/deploy\/production/,
  );
});

// The operator's remote-tracking ref still points at the old tip; only a
// fresh fetch shows that the release moved on.
test('a release that moved on since the last fetch is refused', () => {
  const { publisher, operator } = fixture();
  git(publisher, 'push', '-q', 'origin', 'HEAD:refs/heads/deploy/production');
  const result = guard(operator);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /HEAD is not the tip/);
});

test('an unreachable remote is refused, not trusted', () => {
  const { operator } = fixture();
  git(
    operator,
    'remote',
    'set-url',
    'origin',
    join(operator, 'no-such-remote'),
  );
  const result = guard(operator);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /could not fetch/);
});

test('no environment variable moves the guard to another branch', () => {
  const { operator } = fixture();
  git(operator, 'checkout', '-q', 'develop');
  const result = spawnSync('bash', [guardScript, operator], {
    encoding: 'utf8',
    env: {
      ...gitEnv,
      RELEASE_BRANCH: 'develop',
      RELEASE_REMOTE: 'origin',
      RELEASE_GUARD: 'off',
    },
  });
  assert.equal(result.status, 1);
});

// --- infra/Makefile wiring --------------------------------------------------

const makefile = readFileSync(join(repoRoot, 'infra', 'Makefile'), 'utf8');

const dryRun = (target) => {
  const result = spawnSync(
    'make',
    ['-n', '--no-print-directory', '-C', join(repoRoot, 'infra'), target],
    { encoding: 'utf8' },
  );
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.split('\n').filter((line) => line.trim() !== '');
};

const guarded = [
  ...makefile.matchAll(/^((?:deploy|destroy)-production[A-Za-z0-9_-]*):/gm),
]
  .map((m) => m[1])
  .concat('sync-portal-explorer');

test('every production-changing target runs the guard before anything else', () => {
  assert.ok(guarded.length >= 13, `found only ${guarded.length} targets`);
  for (const target of guarded) {
    const lines = dryRun(target);
    assert.match(
      lines[0] ?? '',
      /check-release-commit\.sh/,
      `${target} does something before the release guard: ${lines[0]}`,
    );
  }
});

test('the production diff is not guarded', () => {
  const lines = dryRun('diff-production');
  assert.ok(
    !lines.some((line) => line.includes('check-release-commit.sh')),
    'diff-production runs the release guard, so a release PR cannot be diffed from develop',
  );
});
