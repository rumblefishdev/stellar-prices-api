// Tests for tools/scripts/ignored-tests.sh — the derived inventory of
// `#[ignore]`d integration tests that CI must run (task 0275).
//
// Every case builds its own throwaway fixture tree under the OS temp dir and
// points the script at it through the `root` argument, so these tests never
// read the live repo and cannot rot with its inventory. The live inventory is
// asserted by CI itself (`ignored-tests.sh check` / `run` in the rust job).
//
// Run: npm run ignored-tests:verify-guard
// (an explicit FILE argument — `node --test <dir>` fails on Node 22, the CI
// version pinned in .nvmrc).

import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  'ignored-tests.sh',
);

const CH =
  '#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]';
const NET =
  '#[ignore = "requires public network — third-party uptime; never gates a PR"]';
const PROD =
  '#[ignore = "requires production — operator after-check; never gates a PR"]';

const created = [];
after(() => {
  for (const dir of created) rmSync(dir, { recursive: true, force: true });
});

// files: { 'relative/path': 'content' }
function tree(files) {
  const root = mkdtempSync(join(tmpdir(), 'ignored-tests-'));
  created.push(root);
  for (const [rel, content] of Object.entries(files)) {
    const path = join(root, rel);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, content);
  }
  return root;
}

function crate(name, dir = name) {
  return {
    [`packages/${dir}/Cargo.toml`]: `[package]\nname = "${name}"\nversion = "0.1.0"\n\n[dependencies]\nserde = { version = "1", features = ["derive"] }\n`,
  };
}

// An `_it.rs` body with one test per attribute given.
function itFile(...attrs) {
  const body = attrs
    .map((attr, i) => `#[tokio::test]\n${attr}\nasync fn t${i}() {}\n`)
    .join('\n');
  return `//! Fixture. Prose mentioning #[ignore] in a doc comment must not count.\n\n${body}`;
}

function run(...args) {
  const r = spawnSync('bash', [SCRIPT, ...args], { encoding: 'utf8' });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

function cleanTree() {
  return tree({
    ...crate('alpha'),
    'packages/alpha/tests/store_it.rs': itFile(CH, CH),
    ...crate('beta'),
    'packages/beta/tests/rpc_it.rs': itFile(NET),
    'packages/beta/tests/after_it.rs': itFile(PROD),
  });
}

test('1: a clean tree passes check, and expect/targets derive from it', () => {
  const root = cleanTree();
  const check = run('check', root);
  assert.equal(check.code, 0, check.err);

  const expect = run('expect', root);
  assert.equal(expect.code, 0, expect.err);
  assert.deepEqual(expect.out.trim().split('\n'), [
    'CH_TESTS=2',
    'CH_TARGETS=1',
    'NET_TESTS=1',
    'PROD_TESTS=1',
  ]);

  const targets = run('targets', root);
  assert.equal(targets.code, 0, targets.err);
  assert.equal(targets.out, '-p alpha --test store_it\n');
});

test('2: an unknown reason fails, naming file and line', () => {
  const root = tree({
    ...crate('alpha'),
    'packages/alpha/tests/store_it.rs': itFile(
      CH,
      '#[ignore = "because reasons"]',
    ),
  });
  const r = run('check', root);
  assert.notEqual(r.code, 0);
  assert.match(
    r.err,
    /packages\/alpha\/tests\/store_it\.rs:8: .*because reasons/,
  );
});

test('3: a bare #[ignore] fails, naming file and line', () => {
  const root = tree({
    ...crate('alpha'),
    'packages/alpha/tests/store_it.rs': itFile('#[ignore]', CH),
  });
  const r = run('check', root);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /packages\/alpha\/tests\/store_it\.rs:4: .*bare/);
});

test('4: a target holding two classes fails, naming the file and both classes', () => {
  const root = tree({
    ...crate('alpha'),
    'packages/alpha/tests/store_it.rs': itFile(CH, NET),
  });
  const r = run('check', root);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /packages\/alpha\/tests\/store_it\.rs/);
  assert.match(r.err, /mixes classes/);
  assert.match(r.err, /\bCH\b/);
  assert.match(r.err, /\bNET\b/);
});

test('5: an _it.rs file with no #[ignore] is in no list and is not an error', () => {
  const root = tree({
    ...crate('alpha'),
    'packages/alpha/tests/store_it.rs': itFile(CH),
    'packages/alpha/tests/plain_it.rs': '#[test]\nfn runs_everywhere() {}\n',
  });
  assert.equal(run('check', root).code, 0);
  const targets = run('targets', root);
  assert.equal(targets.code, 0, targets.err);
  assert.doesNotMatch(targets.out, /plain_it/);
  assert.match(run('expect', root).out, /^CH_TARGETS=1$/m);
});

test('6: the package name comes from Cargo.toml [package], not the directory', () => {
  const root = tree({
    // A `name =` in another table (here `[lib]`, listed first) is not the package.
    'packages/dir-name/Cargo.toml':
      '[lib]\nname = "lib_alias"\n\n[package]\nname = "real-crate"\nversion = "0.1.0"\n',
    'packages/dir-name/tests/store_it.rs': itFile(CH),
  });
  const r = run('targets', root);
  assert.equal(r.code, 0, r.err);
  assert.equal(r.out, '-p real-crate --test store_it\n');
});

test('7: the same target name in two crates is two targets', () => {
  const root = tree({
    ...crate('alpha'),
    'packages/alpha/tests/seed_it.rs': itFile(CH),
    ...crate('beta'),
    'packages/beta/tests/seed_it.rs': itFile(CH, CH),
  });
  const targets = run('targets', root);
  assert.equal(targets.code, 0, targets.err);
  assert.equal(
    targets.out,
    '-p alpha --test seed_it\n-p beta --test seed_it\n',
  );
  const expect = run('expect', root).out;
  assert.match(expect, /^CH_TARGETS=2$/m);
  assert.match(expect, /^CH_TESTS=3$/m);
});

test('8: a CH target name shared with a non-CH target fails', () => {
  // `cargo test --workspace --test oracle_it -- --ignored` would run BOTH
  // binaries, arming the network test through the name alone.
  const root = tree({
    ...crate('alpha'),
    'packages/alpha/tests/oracle_it.rs': itFile(CH),
    ...crate('beta'),
    'packages/beta/tests/oracle_it.rs': itFile(NET),
  });
  const r = run('check', root);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /oracle_it/);
  assert.match(r.err, /name collision/);
});

test('9: a tree with no _it.rs files fails instead of reporting an empty inventory', () => {
  const root = tree({ ...crate('alpha'), 'packages/alpha/src/lib.rs': '' });
  const r = run('check', root);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /no ClickHouse-class/);
});

test('10: image-tag reads the pin from docker-compose.yml, and refuses when it is gone', () => {
  const compose = (image) =>
    `services:\n  clickhouse:\n    # a comment\n    image: ${image}\n    ports:\n      - '8123:8123'\n`;
  const good = tree({
    'docker-compose.yml': compose('clickhouse/clickhouse-server:26.3.10.60'),
  });
  const r = run('image-tag', good);
  assert.equal(r.code, 0, r.err);
  assert.equal(r.out, '26.3.10.60\n');

  const renamed = tree({
    'docker-compose.yml': compose('example/other-db:26.3.10.60'),
  });
  const bad = run('image-tag', renamed);
  assert.notEqual(bad.code, 0);
  assert.match(bad.err, /clickhouse-server/);
});

test('10b: an #[ignore] outside packages/*/tests/*_it.rs fails', () => {
  // The classifier only derives targets from `*_it.rs` files; an ignored test
  // anywhere else would be invisible to it — never run, never counted.
  const root = tree({
    ...crate('alpha'),
    'packages/alpha/tests/store_it.rs': itFile(CH),
    'packages/alpha/src/lib.rs': `#[cfg(test)]\nmod t {\n    ${CH}\n    #[test]\n    fn hidden() {}\n}\n`,
  });
  const r = run('check', root);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /packages\/alpha\/src\/lib\.rs:3/);
  assert.match(r.err, /outside/);
});

// ---- `assert <logfile> [root]`: the counted run's verdict (task 0275, D4) ----
//
// The fixture tree derives CH_TESTS=3 over CH_TARGETS=2; each case feeds a
// synthetic cargo log, so no cargo and no ClickHouse are needed.

function countedTree() {
  return tree({
    ...crate('alpha'),
    'packages/alpha/tests/store_it.rs': itFile(CH, CH),
    ...crate('beta'),
    'packages/beta/tests/seed_it.rs': itFile(CH),
    'packages/beta/tests/rpc_it.rs': itFile(NET),
  });
}

const result = (passed, failed = 0) =>
  `test result: ${failed ? 'FAILED' : 'ok'}. ${passed} passed; ${failed} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s`;

function assertLog(lines) {
  const root = countedTree();
  const log = join(root, 'cargo.log');
  writeFileSync(log, lines.join('\n') + '\n');
  return run('assert', log, root);
}

test('11: a log matching both derived counts passes', () => {
  const r = assertLog([
    '     Running tests/store_it.rs (target/debug/deps/store_it-aaaa)',
    result(2),
    '     Running tests/seed_it.rs (target/debug/deps/seed_it-bbbb)',
    result(1),
  ]);
  assert.equal(r.code, 0, r.err);
  assert.match(r.out, /3 passed/);
});

test('12: a binary with no summary at all is reported as such, not as drift', () => {
  const r = assertLog([
    '     Running tests/store_it.rs (target/debug/deps/store_it-aaaa)',
    result(2),
    '     Running tests/seed_it.rs (target/debug/deps/seed_it-bbbb)',
    'error: test failed, to rerun pass `-p beta --test seed_it`',
    "Caused by: process didn't exit successfully (signal: 9, SIGKILL: kill)",
  ]);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /produced no summary/);
  assert.match(r.err, /1 of 2/);
  assert.doesNotMatch(r.err, /count mismatch/);
});

test('13: the right number of summaries with too few passes is a count mismatch', () => {
  const r = assertLog([result(1), result(1)]);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /count mismatch/);
  assert.match(r.err, /expected 3/);
  assert.match(r.err, /got 2/);
});

test('14: any failed test fails the run, whatever the sums', () => {
  const r = assertLog([result(2), result(1, 1)]);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /1 failed/);
});

test('15: ANSI colour around cargo output is parsed the same', () => {
  const esc = String.fromCharCode(27);
  const colored = (passed) =>
    `${esc}[0m${esc}[1mtest result: ${esc}[32mok${esc}[0m. ${passed} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s`;
  const r = assertLog([colored(2), colored(1)]);
  assert.equal(r.code, 0, r.err);
  assert.match(r.out, /3 passed/);
});

test('16: a Doc-tests summary in the log fails rather than being miscounted', () => {
  const r = assertLog([result(2), result(1), '   Doc-tests alpha', result(0)]);
  assert.notEqual(r.code, 0);
  assert.match(r.err, /Doc-tests/);
});
