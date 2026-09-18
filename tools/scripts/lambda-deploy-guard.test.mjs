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
// Run: node --test tools/scripts/

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..');
const verifyScript = join(here, 'verify-lambda-bootstraps.sh');

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
const fixtureRoot = (bootstraps) => {
  const root = mkdtempSync(join(tmpdir(), 'lambda-guard-'));
  mkdirSync(join(root, 'infra', 'src'), { recursive: true });
  writeFileSync(
    join(root, 'infra', 'src', 'stack.ts'),
    Object.keys(bootstraps)
      .map((name) => `const a = env ?? '../target/lambda/${name}';`)
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

const verify = (root) =>
  spawnSync('bash', [verifyScript, root], { encoding: 'utf8' });

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

// --- infra/Makefile wiring -------------------------------------------------

const makefile = readFileSync(join(repoRoot, 'infra', 'Makefile'), 'utf8');

const dryRun = (target) => {
  const result = spawnSync(
    'make',
    ['-n', '-C', join(repoRoot, 'infra'), target],
    {
      encoding: 'utf8',
    },
  );
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.split('\n');
};

// Every production deploy target that runs `cdk deploy`, read from the
// Makefile rather than listed here, so a new one is covered the day it lands.
const cdkDeploys = [...makefile.matchAll(/^(deploy-production[a-z-]*):/gm)]
  .map((m) => m[1])
  .map((target) => {
    const lines = dryRun(target);
    const at = lines.findIndex((line) => /\bcdk\b.* deploy /.test(line));
    return { target, lines, at };
  })
  .filter(({ at }) => at >= 0);

// Stacks that package a Lambda, derived from the CDK source: `compute-stack.ts`
// naming `target/lambda` means `Prices-production-Compute` ships bootstraps.
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

test('the Makefile exposes cdk deploy targets and Lambda stacks to check', () => {
  assert.ok(cdkDeploys.length >= 2, 'found no cdk deploy targets');
  assert.ok(lambdaStacks.length >= 1, 'found no stack packaging a Lambda');
  assert.ok(
    cdkDeploys.some(({ lines, at }) => shipsLambdas(lines[at])),
    'no deploy target matched a Lambda-packaging stack',
  );
});

test('every deploy that can ship a Lambda builds the Lambdas first', () => {
  for (const { target, lines, at } of cdkDeploys) {
    if (!shipsLambdas(lines[at])) continue;
    const built = lines.findIndex((line) =>
      line.includes('build-lambda-assets.sh'),
    );
    assert.ok(built >= 0, `${target} never builds the Lambda assets`);
    assert.ok(built < at, `${target} builds the Lambda assets after deploying`);
  }
});

test('every per-stack deploy is exclusive to its stack', () => {
  for (const { target, lines, at } of cdkDeploys) {
    if (lines[at].includes('--all')) continue;
    assert.match(
      lines[at],
      /--exclusively/,
      `${target} can pull in dependency stacks`,
    );
  }
});
