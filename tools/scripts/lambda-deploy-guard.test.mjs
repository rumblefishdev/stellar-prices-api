// The production deploy must never ship a Lambda artifact it did not just
// build and check (task 0141). Two halves, both asserted here:
//
//   - `verify-lambda-bootstraps.sh` refuses anything that is not a distinct
//     aarch64 ELF per asset — the 10-byte `#!/bin/sh` stubs of 2026-08-12
//     included;
//   - `infra/Makefile` runs the build before every deploy that can ship a
//     Lambda, and every per-stack deploy is `--exclusively`, so a deploy that
//     has nothing to do with Compute cannot drag Compute along.
//
// Run: npx nx test @rumblefish/stellar-prices-api-aws-cdk

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative } from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..');
const verifyScript = join(here, 'verify-lambda-bootstraps.sh');
const buildScript = join(here, 'build-lambda-assets.sh');

// e_machine, little-endian, at offset 18 of the ELF header.
const AARCH64 = 0xb7;
const X86_64 = 0x3e;

const elf = (machine, payload) => {
  const header = Buffer.alloc(64);
  header.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1], 0);
  header.writeUInt16LE(2, 16);
  header.writeUInt16LE(machine, 18);
  return Buffer.concat([header, Buffer.from(payload)]);
};

// A repo root holding just what the scripts read: CDK source naming the asset
// dirs, and whatever is (or is not) sitting under target/lambda/.
const fixtureRoots = [];
after(() => {
  for (const root of fixtureRoots) rmSync(root, { recursive: true });
});

// The CDK's own shape: `process.env['PRICES_API_ASSET_DIR'] ?? '../target/…'`.
const overrideVar = (name) =>
  `${name.toUpperCase().replaceAll('-', '_')}_ASSET_DIR`;

const fixtureRoot = (bootstraps) => {
  const root = mkdtempSync(join(tmpdir(), 'lambda-guard-'));
  fixtureRoots.push(root);
  mkdirSync(join(root, 'infra', 'src'), { recursive: true });
  writeFileSync(
    join(root, 'infra', 'src', 'stack.ts'),
    Object.keys(bootstraps)
      .map(
        (name) =>
          `const a = process.env['${overrideVar(name)}'] ?? '../target/lambda/${name}';`,
      )
      .join('\n'),
  );
  for (const [name, bytes] of Object.entries(bootstraps)) {
    if (bytes === null) continue;
    const dir = join(root, 'target', 'lambda', name);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'bootstrap'), bytes);
    chmodSync(join(dir, 'bootstrap'), 0o755);
  }
  return root;
};

// The operator's shell may carry an override; these tests decide their own.
const cleanEnv = Object.fromEntries(
  Object.entries(process.env).filter(
    ([key]) => !key.endsWith('_ASSET_DIR') && key !== 'CARGO_TARGET_DIR',
  ),
);

const verify = (root, env = {}) =>
  spawnSync('bash', [verifyScript, root], {
    encoding: 'utf8',
    env: { ...cleanEnv, ...env },
  });

test('a distinct aarch64 bootstrap per asset passes', () => {
  const root = fixtureRoot({
    'prices-api': elf(AARCH64, 'api'),
    'oracle-worker': elf(AARCH64, 'oracle'),
  });

  const result = verify(root);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /verified 2 Lambda bootstrap/);
});

test('a shell stub in place of a bootstrap is refused by name', () => {
  const root = fixtureRoot({
    'prices-api': Buffer.from('#!/bin/sh\n'),
    'oracle-worker': elf(AARCH64, 'oracle'),
  });

  const result = verify(root);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /prices-api.*not an ELF/);
});

test('a bootstrap built for the host instead of arm64 is refused', () => {
  const root = fixtureRoot({ 'prices-api': elf(X86_64, 'api') });

  const result = verify(root);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /prices-api.*aarch64/);
});

test('a missing bootstrap is refused', () => {
  const root = fixtureRoot({ 'prices-api': null });

  const result = verify(root);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /prices-api.*missing/);
});

test('two assets sharing one binary are refused', () => {
  const root = fixtureRoot({
    'prices-api': elf(AARCH64, 'same'),
    'oracle-worker': elf(AARCH64, 'same'),
  });

  const result = verify(root);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /identical/);
});

test('an asset dir overridden through the environment is refused', () => {
  const root = fixtureRoot({ 'prices-api': elf(AARCH64, 'api') });

  // CDK would zip /elsewhere; what sits under target/lambda proves nothing.
  const result = verify(root, { PRICES_API_ASSET_DIR: '/elsewhere' });

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /PRICES_API_ASSET_DIR/);
});

test('a cargo target dir other than the one CDK reads is refused unbuilt', () => {
  const root = fixtureRoot({ 'prices-api': elf(AARCH64, 'api') });
  writeFileSync(
    join(root, 'Cargo.toml'),
    '[package]\nname = "prices-api"\nversion = "0.0.0"\nedition = "2021"\n',
  );
  mkdirSync(join(root, 'src'));
  writeFileSync(join(root, 'src', 'lib.rs'), '');

  const result = spawnSync('bash', [buildScript, root], {
    encoding: 'utf8',
    env: { ...cleanEnv, CARGO_TARGET_DIR: join(root, 'elsewhere') },
  });

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /target dir/);
  assert.doesNotMatch(result.stdout, /cargo lambda build/);
});

// --- infra/Makefile wiring -------------------------------------------------

const makefile = readFileSync(join(repoRoot, 'infra', 'Makefile'), 'utf8');

const dryRun = (target) => {
  const result = spawnSync(
    'make',
    ['-n', '-C', join(repoRoot, 'infra'), target],
    { encoding: 'utf8' },
  );
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.split('\n');
};

const buildsLambdasBefore = (lines, at) => {
  const built = lines.findIndex((line) =>
    line.includes('build-lambda-assets.sh'),
  );
  return built >= 0 && built < at;
};

// Every production deploy target that runs `cdk deploy`, read from the
// Makefile rather than listed here, so a new one is covered the day it lands.
// Computed inside the tests: a `make` that cannot run must fail the tests that
// need it, by name, not the whole file at import.
let cdkDeploysMemo;
const cdkDeploys = () =>
  (cdkDeploysMemo ??= [
    ...makefile.matchAll(/^(deploy-production[A-Za-z0-9_-]*):/gm),
  ]
    .map((m) => m[1])
    .map((target) => {
      const lines = dryRun(target);
      const at = lines.findIndex((line) => /\bcdk\b.* deploy /.test(line));
      return { target, lines, at };
    })
    .filter(({ at }) => at >= 0));

// Stacks that package a Lambda, derived from the CDK source: `compute-stack.ts`
// naming `target/lambda` means `Prices-production-Compute` ships bootstraps.
// That reading is only sound while the asset-dir literals live in the stack
// that uses them — asserted below, so moving them cannot blind this quietly.
const stacksDir = join(repoRoot, 'infra', 'src', 'lib', 'stacks');
const lambdaStacks = readdirSync(stacksDir)
  .filter((file) => file.endsWith('-stack.ts'))
  .filter((file) =>
    readFileSync(join(stacksDir, file), 'utf8').includes('target/lambda'),
  )
  .map((file) => file.replace(/-stack\.ts$/, '').replaceAll('-', ''));

const shipsLambdas = (deployLine) =>
  deployLine.includes('--all') ||
  lambdaStacks.some((stack) =>
    deployLine.toLowerCase().includes(`prices-production-${stack} `),
  );

const sourceFiles = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory()
      ? sourceFiles(join(dir, entry.name))
      : [join(dir, entry.name)],
  );

test('every Lambda asset dir is named by the stack file that packages it', () => {
  const naming = sourceFiles(join(repoRoot, 'infra', 'src')).filter((file) =>
    /'\.\.\/target\/lambda\//.test(readFileSync(file, 'utf8')),
  );

  assert.ok(naming.length >= 1, 'found no asset dir literal at all');
  for (const file of naming) {
    assert.match(
      relative(repoRoot, file),
      /^infra\/src\/lib\/stacks\/[a-z-]+-stack\.ts$/,
      'an asset dir named outside a stack file hides its stack from these tests',
    );
  }
});

test('every Lambda-packaging stack is matched by a deploy target', () => {
  assert.ok(cdkDeploys().length >= 2, 'found no cdk deploy targets');
  for (const stack of lambdaStacks) {
    assert.ok(
      cdkDeploys().some(({ lines, at }) =>
        lines[at].toLowerCase().includes(`prices-production-${stack} `),
      ),
      `no deploy target names the ${stack} stack`,
    );
  }
});

test('every deploy that can ship a Lambda builds the Lambdas first', () => {
  for (const { target, lines, at } of cdkDeploys()) {
    if (!shipsLambdas(lines[at])) continue;
    assert.ok(
      buildsLambdasBefore(lines, at),
      `${target} does not build the Lambda assets before deploying`,
    );
  }
});

test('the production diff is taken against freshly built Lambdas', () => {
  const lines = dryRun('diff-production');
  const at = lines.findIndex((line) => /\bcdk\b.* diff/.test(line));

  assert.ok(at >= 0, 'diff-production runs no cdk diff');
  assert.ok(
    buildsLambdasBefore(lines, at),
    'diff-production describes whatever artifacts were on disk',
  );
});

test('every per-stack deploy is exclusive to its stack', () => {
  for (const { target, lines, at } of cdkDeploys()) {
    if (lines[at].includes('--all')) continue;
    assert.match(
      lines[at],
      /--exclusively/,
      `${target} can pull in dependency stacks`,
    );
  }
});
