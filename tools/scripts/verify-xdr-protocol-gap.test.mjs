// Tier matrix for tools/scripts/verify-xdr-protocol-gap.mjs (tasks 0319,
// 0325), run against a local mock of Horizon and crates.io so it needs no
// network. Every case is relative to the real pin in Cargo.toml, so a
// stellar-xdr bump needs no edit here.
//
// Run: node --test tools/scripts/verify-xdr-protocol-gap.test.mjs
// (an explicit FILE argument — `node --test <dir>` fails on Node 22, the CI
// version pinned in .nvmrc). CI runs it through the infra project's `test`
// target.

import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(here, 'verify-xdr-protocol-gap.mjs');
// The pin as the script's pinnedMajor() reads it: [workspace.dependencies]
// only, so a stellar-xdr line elsewhere (a [patch], a per-crate override)
// cannot make the test and the script disagree.
const toml = readFileSync(join(here, '../../Cargo.toml'), 'utf8');
const deps = toml.split(/^\[workspace\.dependencies\]$/m)[1].split(/^\[/m)[0];
const pin = deps.match(/^stellar-xdr\s*=\s*(.+)$/m)[1];
const p = Number(pin.match(/(\d+)\.\d+\.\d+/)[1]);

// What the mock serves for the running case; `null` answers HTTP 500.
let horizon, crate, base;
const server = createServer((req, res) => {
  const body = req.url.startsWith('/horizon') ? horizon : crate;
  res.writeHead(body ? 200 : 500, { 'content-type': 'application/json' });
  res.end(JSON.stringify(body ?? {}));
});
before(async () => {
  await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
  base = `http://127.0.0.1:${server.address().port}`;
});
after(() => server.close());

const net = (current, supported) => ({
  current_protocol_version: current,
  core_supported_protocol_version: supported,
});
const published = (stable, max = stable) => ({
  crate: { max_stable_version: stable, max_version: max },
});

// Async, not spawnSync: the mock lives in this process and must keep serving.
const run = (watch) =>
  new Promise((ok) =>
    execFile(
      process.execPath,
      [SCRIPT, ...(watch ? ['--watch'] : [])],
      {
        env: {
          ...process.env,
          HORIZON_URL: `${base}/horizon`,
          CRATES_URL: `${base}/crates`,
        },
      },
      (error, stdout, stderr) =>
        ok({ code: error ? error.code : 0, out: stdout + stderr }),
    ),
  );

// [name, horizon, crates.io, --watch?, expected exit, expected text]
// prettier-ignore
const cases = [
  ['level', net(p, p), published(`${p}.0.1`), true, 0, 'is current with mainnet'],
  ['WAITING before the vote', net(p, p + 1), published(`${p}.0.1`), true, 0, '— WAITING: no stellar-xdr'],
  ['LAGGING, watch', net(p, p + 1), published(`${p + 1}.0.0`), true, 1, 'lags the protocol core'],
  ['LAGGING, PR', net(p, p + 1), published(`${p + 1}.0.0`), false, 0, 'the bump is possible now'],
  ['voted, no crate (P29), watch', net(p + 1, p + 1), published(`${p}.0.1`), true, 0, `is behind mainnet protocol ${p + 1} — WAITING: no stellar-xdr`],
  ['voted, no crate (P29), PR', net(p + 1, p + 1), published(`${p}.0.1`), false, 0, '— WAITING: no stellar-xdr'],
  ['voted, pre-release only', net(p + 1, p + 1), published(`${p}.0.1`, `${p + 1}.0.0-rc.1`), true, 0, 'pre-release'],
  ['BEHIND, crate published, watch', net(p + 1, p + 1), published(`${p + 1}.0.0`), true, 1, 'is BEHIND mainnet protocol'],
  ['BEHIND, crate published, PR', net(p + 1, p + 1), published(`${p + 1}.0.0`), false, 1, 'is BEHIND mainnet protocol'],
  ['BEHIND, crate for mainnet but not core', net(p + 1, p + 2), published(`${p + 1}.0.0`), true, 1, 'is BEHIND mainnet protocol'],
  ['voted, crates.io down, watch', net(p + 1, p + 1), null, true, 1, 'error: cannot read'],
  ['voted, crates.io down, PR', net(p + 1, p + 1), null, false, 0, 'crate availability not checked'],
  ['Horizon down, watch', null, published(`${p}.0.1`), true, 1, 'error: cannot reach'],
];

for (const [name, h, c, watch, exit, text] of cases)
  test(name, async () => {
    horizon = h;
    crate = c;
    const { code, out } = await run(watch);
    assert.equal(code, exit, out);
    assert.ok(out.includes(text), `expected "${text}" in:\n${out}`);
  });
