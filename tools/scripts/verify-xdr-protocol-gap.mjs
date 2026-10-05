#!/usr/bin/env node
//
// Surface when our pinned `stellar-xdr` lags the Stellar mainnet protocol,
// BEFORE the lag stalls live ingestion at an XDR decode wall.
//
// WHY THIS EXISTS
// ---------------
// Protocol 27 "Zipper" froze the live candle frontier for SIX DAYS (task 0091,
// deployed by 0094). The deployed ledger-processor was on `stellar-xdr 26`
// while mainnet moved to protocol 27; it hit a decode wall at ledger
// 63,401,875 and stopped writing rows.
//
// Nothing reported it. The SQS queue drained normally and both it and the DLQ
// were empty, so nothing dead-lettered and nothing could be redriven. The
// Lambda logged zero parse, XDR, panic or ClickHouse errors. The doorbell-lag
// alarm watches queue age, and the queue was healthy — the pass drained and
// simply produced no candles. It was found by reading `max(timestamp)` out of
// `price_ohlcv_1m` by hand, six days in.
//
// Protocol 28 "Adapter" then reached us the same way protocol 27 did: somebody
// read a blog post (task 0277). Two for two. This script is the guard that was
// spawned from 0094's future work to end that.
//
// THE MAPPING THIS ENCODES
// ------------------------
// `stellar-xdr`'s MAJOR version tracks the protocol version: 26 decodes
// protocol 26, 27 decodes 27, 28 decodes 28. That is the whole comparison.
// If upstream ever breaks that correspondence this check goes wrong in the
// safe direction — it compares two integers and says which is larger — but the
// assumption is stated here so a future reader can test it rather than infer
// it.
//
// Protocol 29 broke it (task 0325). stellar-core v29 pins the same XDR commit
// as v28.0.1 (`9c9c145`): the vote changed apply rules, not the format, so
// `stellar-xdr 28` decodes P29 ledgers and no `stellar-xdr 29` was published.
// What stopped ingestion was BE's Galexie, whose captive core must match the
// protocol whatever the XDR does — a failure this script cannot see.
//
// TWO TIERS, AND WHY THE WARNING ONE IS THE VALUABLE ONE
// ------------------------------------------------------
// Horizon's root document publishes both numbers:
//
//   current_protocol_version        what mainnet is running RIGHT NOW
//   core_supported_protocol_version what core is ready to run
//
// `core_supported` rises WEEKS BEFORE the vote. That gap is the entire lead
// time this task exists to buy, so it is the tier that does the real work:
//
//   BEHIND   ours < current         mainnet has moved past us. The decode wall
//                                   is live or one empty-tx-set ledger away.
//                                   Fatal in both modes when a crate for it
//                                   is published (WAITING otherwise, below).
//   LAGGING  ours < core_supported  an upgrade is announced and available.
//                                   Fatal only under --watch (see below).
//
// LAGGING IS SPLIT BY WHETHER THERE IS ANYTHING TO BUMP TO (task 0319)
// ---------------------------------------------------------------------
// Core runs a new protocol before the Rust crate for it is published: SDF
// builds core from the XDR definitions in git, and core's own Rust takes
// rs-stellar-xdr by git too. On 2026-09-29 Horizon reported core 29 while
// crates.io's newest `stellar-xdr` was 28.0.1 — the issue asked for a bump
// nobody could make. So a LAGGING reading also asks crates.io:
//
//   WAITING  no published stable major reaches core_supported. The protocol is
//            announced, there is nothing to bump to yet. NOT fatal, even under
//            --watch: a red run and an issue are for something someone can
//            act on, and here nobody can. The report says so and exits 0.
//   LAGGING  a stable major that reaches core_supported is on crates.io. Bump
//            now (after BE's xdr-parser). Fatal under --watch.
//
// BEHIND asks crates.io the same way (task 0325): mainnet on a protocol with no
// published stable crate is WAITING, not BEHIND. There is nothing to bump to,
// and the protocol may not have changed XDR at all (29 did not). A protocol
// that DOES change XDR and is voted before its crate ships stays green here;
// that stall shows as frozen candles, which the rollup-freshness and
// ledger-processor-no-invocations alarms catch within the hour.
// ponytail: compares protocol numbers; comparing the crate's XDR commit with
// core's (`src/protocol-curr/xdr`) is the exact check if this misfires again.
//
// crates.io unreadable → fatal under --watch, but reported as a check that
// could not complete (`cannot read … crates.io`), not as LAGGING: the watch
// failing to look is worth a red run, not an issue asking for a bump. A
// pre-release (`29.0.0-rc.1`) does not count as published; it is named in the
// report so the wait is visible.
//
// Operator guide: docs/runbooks/xdr-protocol-watch.md
//
// WHY --watch EXISTS
// ------------------
// On a pull request this script must never fail the build because the Stellar
// Foundation announced something or because Horizon had a bad minute. A
// developer cannot fix either, and a gate that red-lights unrelated PRs gets
// disabled within a week.
//
// But a PR-only guard would NOT have caught proto27 at all: no code changed on
// our side — mainnet moved and we stood still. So the real guard is the
// SCHEDULED run, and `--watch` is what makes it strict: there, LAGGING is a
// failure (that is the point — it is the early warning) and an unreachable
// Horizon is a failure too, because a watch that silently checks nothing is
// the exact failure mode this whole task is about.
//
// THE THIRD CHECK: TWO MAJORS IN THE LOCKFILE
// -------------------------------------------
// `xdr-parser` is BE's crate, tracked on `branch="develop"` deliberately (it
// is not a rev pin — see the xdr-parser-develop-branch note), and
// `prices-ingest-core/src/decode.rs` takes `LedgerCloseMeta` across that crate
// boundary. If BE's bump lands before ours, or ours before theirs, Cargo
// resolves BOTH majors into the graph and the types stop matching. That is a
// compile error rather than a silent freeze, but it is worth naming precisely
// when it happens instead of leaving someone to read a wall of trait errors.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '../..');
const watch = process.argv.includes('--watch');
const horizonUrl = process.env.HORIZON_URL ?? 'https://horizon.stellar.org/';
const cratesUrl =
  process.env.CRATES_URL ?? 'https://crates.io/api/v1/crates/stellar-xdr';

const die = (...lines) => {
  console.error(lines.join('\n'));
  process.exit(1);
};

// ---------------------------------------------------------------- our pin --

/**
 * The `[workspace.dependencies]` pin — our stated intent. Read by regex rather
 * than a TOML parser so this script stays dependency-free; the pattern is
 * anchored to the section so a same-named key elsewhere cannot match.
 */
const pinnedMajor = () => {
  const toml = readFileSync(join(repoRoot, 'Cargo.toml'), 'utf8');
  const section = toml.split(/^\[workspace\.dependencies\]$/m)[1];
  if (!section) die('error: no [workspace.dependencies] section in Cargo.toml');

  const line = section.split(/^\[/m)[0].match(/^stellar-xdr\s*=\s*(.+)$/m);
  if (!line) die('error: no stellar-xdr entry in [workspace.dependencies]');

  const version = line[1].match(/(\d+)\.\d+\.\d+/);
  if (!version)
    die(`error: cannot read a version out of: stellar-xdr = ${line[1]}`);

  return Number(version[1]);
};

/**
 * Every `stellar-xdr` actually resolved into the build. More than one MAJOR
 * here is the xdr-parser skew described in the header.
 */
const resolvedMajors = () => {
  const lock = readFileSync(join(repoRoot, 'Cargo.lock'), 'utf8');
  const majors = new Set();
  for (const block of lock.split('[[package]]')) {
    if (!/^\s*name = "stellar-xdr"\s*$/m.test(block)) continue;
    const version = block.match(/^\s*version = "(\d+)\.\d+\.\d+"\s*$/m);
    if (version) majors.add(Number(version[1]));
  }
  return [...majors].sort((a, b) => a - b);
};

// ------------------------------------------------------------- the network --

const networkProtocol = async () => {
  const response = await fetch(horizonUrl, {
    signal: AbortSignal.timeout(15000),
    headers: { accept: 'application/json' },
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);

  const root = await response.json();
  const current = root.current_protocol_version;
  const supported =
    root.core_supported_protocol_version ?? root.supported_protocol_version;

  if (!Number.isInteger(current) || !Number.isInteger(supported))
    throw new Error(
      `unexpected root document: current_protocol_version=${current}, ` +
        `core_supported_protocol_version=${supported}`,
    );

  return { current, supported };
};

/**
 * What crates.io has published: the newest stable version, and the newest
 * version of any kind when that is a pre-release ahead of it. crates.io
 * rejects requests without a User-Agent.
 */
const publishedCrate = async () => {
  const response = await fetch(cratesUrl, {
    signal: AbortSignal.timeout(15000),
    headers: {
      accept: 'application/json',
      'user-agent':
        'stellar-prices-api xdr-protocol-watch (github.com/rumblefishdev/stellar-prices-api)',
    },
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);

  const { crate } = await response.json();
  const stable = crate?.max_stable_version;
  const major = Number(String(stable).match(/^(\d+)\.\d+\.\d+$/)?.[1]);
  if (!Number.isInteger(major))
    throw new Error(`unexpected crate document: max_stable_version=${stable}`);

  const newest = crate.max_version;
  return {
    stable,
    major,
    preRelease: newest && newest !== stable ? newest : null,
  };
};

// -------------------------------------------------------------------- run --

const pinned = pinnedMajor();
const resolved = resolvedMajors();

if (resolved.length > 1)
  die(
    `error: ${resolved.length} stellar-xdr majors resolved into the build: ${resolved.join(', ')}.`,
    '',
    'decode.rs passes LedgerCloseMeta across the xdr-parser crate boundary, so',
    'two majors in the graph is a type mismatch, not a warning. Our pin and',
    "BE's xdr-parser must move together — see task 0277.",
  );

if (resolved.length === 1 && resolved[0] !== pinned)
  die(
    `error: Cargo.toml pins stellar-xdr ${pinned} but Cargo.lock resolved ${resolved[0]}.`,
    '',
    'Run `cargo update -p stellar-xdr` (or re-pin) so the lockfile and the',
    'stated pin agree before trusting any protocol comparison.',
  );

let network;
try {
  network = await networkProtocol();
} catch (error) {
  const message = `cannot reach ${horizonUrl}: ${error.message}`;
  if (watch)
    die(
      `error: ${message}`,
      '',
      'A scheduled watch that checks nothing is the failure this guard exists',
      'to prevent, so an unreachable Horizon fails here rather than passing',
      'quietly. On a pull request the same condition is only a notice.',
    );
  console.log(`notice: ${message} — protocol comparison skipped`);
  process.exit(0);
}

const summary =
  `pinned stellar-xdr ${pinned} | mainnet current ${network.current} | ` +
  `core supports ${network.supported}`;

const behind = pinned < network.current;

if (behind || pinned < network.supported) {
  // The crate a bump would need: mainnet's protocol once it has voted, the one
  // core supports while the vote is still ahead.
  const needed = behind ? network.current : network.supported;

  let crate;
  try {
    crate = await publishedCrate();
  } catch (error) {
    const message = `cannot read ${cratesUrl}: ${error.message}`;
    if (watch)
      die(
        `error: ${message}`,
        '',
        `  ${summary}`,
        '',
        `Whether a stellar-xdr ${needed} is published could not be checked.`,
        'The watch could not complete, so the run fails; no bump is being',
        'asked for. The next run checks again.',
      );
    console.log(`notice: ${message} — crate availability not checked`);
    process.exit(0);
  }

  // The `— WAITING: no stellar-xdr` wording is matched by master's
  // xdr-protocol-watch.yml to close an open issue as "not actionable yet".
  if (crate.major < needed) {
    console.log(
      [
        behind
          ? `notice: stellar-xdr ${pinned} is behind mainnet protocol ${network.current} — WAITING: no stellar-xdr ${needed} is published yet.`
          : `notice: stellar-xdr ${pinned} lags the protocol core already supports (${network.supported}) — WAITING: no stellar-xdr ${needed} is published yet.`,
        '',
        `  ${summary} | newest on crates.io ${crate.stable}` +
          (crate.preRelease ? ` (pre-release ${crate.preRelease})` : ''),
        '',
        ...(behind
          ? [
              'Mainnet has voted, but there is no crate to bump to: either the',
              'protocol changed no XDR (29 kept the XDR of 28) or the crate is late.',
              'A real decode wall shows as frozen candles — the rollup-freshness and',
              'ledger-processor-no-invocations alarms. The watch fails and opens an',
              `issue the day a stable stellar-xdr ${needed}.x appears on crates.io.`,
            ]
          : [
              'Nothing to do yet: there is no crate to bump to, and mainnet has not',
              'voted. The watch fails and opens an issue the day a stable',
              `stellar-xdr ${needed}.x appears on crates.io.`,
            ]),
      ].join('\n'),
    );
    process.exit(0);
  }

  if (behind)
    die(
      `error: stellar-xdr ${pinned} is BEHIND mainnet protocol ${network.current}.`,
      '',
      `  ${summary} | newest on crates.io ${crate.stable}`,
      '',
      'This is the proto27 condition: the decode wall is live or one ledger',
      'away, and a stalled ledger-processor drains its queue and logs nothing.',
      '',
      'Bump the pin, then DEPLOY it — 0091 merged the proto27 fix and prod',
      'stayed frozen until 0094 actually shipped the binary. Runbook:',
      'docs/runbooks/deploy-ledger-processor.md',
    );

  const lines = [
    `stellar-xdr ${pinned} lags the protocol core already supports (${network.supported}).`,
    '',
    `  ${summary} | newest on crates.io ${crate.stable}`,
    '',
    `stellar-xdr ${crate.stable} is published — the bump is possible now.`,
    'Mainnet has not voted yet, so nothing is broken — this is the lead time.',
    'Open the bump before the vote. BE must bump xdr-parser first, and check',
    "the crate's XDR commit, not only its version number.",
  ];
  if (watch) die(`error: ${lines[0]}`, ...lines.slice(1));
  console.log(`notice: ${lines.join('\n')}`);
  process.exit(0);
}

console.log(`stellar-xdr is current with mainnet — ${summary}`);
