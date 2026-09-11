---
id: "0277"
title: "Bump stellar-xdr 27→28 before Protocol 28 (Adapter) activates on 2026-09-16 — BE must bump xdr-parser first"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0091", "0094", "0098", "0064"]
tags: [layer-backend, priority-high, effort-small, phase-live, clickhouse, resilience, ingestion, deployment]
links:
  - "https://stellar.org/blog/developers/adapter-protocol-28-upgrade-guide"
  - "../archive/0091_BUG_stellar-xdr-27-proto27-decode.md"
  - "../archive/0094_FEATURE_proto27-deploy-replay-verify.md"
  - "0098_FEATURE_xdr-protocol-version-gap-ci-guard.md"
history:
  - date: 2026-09-11
    status: backlog
    who: okarcz
    note: >
      Filed after the Protocol 28 "Adapter" upgrade guide was published.
      Mainnet vote 2026-09-16 17:00 UTC. This is the same shape as [[0091]],
      which was only found AFTER proto27 froze live ingestion for six days —
      filed proactively this time. ⚠️ Blocked in practice on BE bumping
      `xdr-parser`; see the Blocker section.
---

# stellar-xdr 27 → 28 for Protocol 28 "Adapter"

## Summary

Stellar mainnet votes to **Protocol 28 "Adapter" on 2026-09-16 at 17:00 UTC**.
Our workspace is exact-pinned to `stellar-xdr = "=27.0.0"`. `stellar-xdr 28.0.0`
is published on crates.io. Protocol 27 froze live ingestion for six days on
exactly this mechanism, so the bump is the whole task — plus the deploy, because
[[0091]] proved that a merged bump with an undeployed binary changes nothing.

## Context

### The precedent this task exists to avoid repeating

Protocol 27 "Zipper" (2026-07-09) stalled the live candle frontier at ledger
`~63,384,067` / 2026-07-08 12:31 and it stayed frozen for **six days**. The
deployed ledger-processor was on `stellar-xdr 26` while mainnet moved to 27, and
it hit a decode wall at ledger `63,401,875`.

The failure was silent in every way that mattered:

- SQS drained normally; the main queue and the DLQ were both **empty**, so
  nothing dead-lettered and nothing could be redriven.
- The Lambda logged **zero** parse / XDR / panic / ClickHouse errors.
- The doorbell-lag alarm was blind to it — it watches queue age, and the queue
  was healthy. The pass drained and simply wrote no rows.

It was caught by reading `max(timestamp)` per source out of `price_ohlcv_1m` by
hand, six days in.

### What Protocol 28 actually changes for us

Three CAPs ship. Only one reaches this codebase:

| CAP | change | reaches us? |
|---|---|---|
| **CAP-83** | new `StellarValue` type `STELLAR_VALUE_EMPTY_TX_SET` | **yes** |
| CAP-85 | new `CONTRACT_EXECUTABLE_EXTERNAL_REF` executable type | no — zero `ContractExecutable` matches in the repo |
| CAP-86 | new `sparse_map_*` host functions | no — we do not run the Soroban host |

The upgrade guide names the affected class directly: *"indexers, analytics
pipelines, custom tooling"* consuming raw ledger data, which must *"treat ledgers
carrying this value as empty ledgers and handle the new type."* We consume raw
`LedgerCloseMeta` from Galexie objects, so we are squarely in it.

`stellar-xdr 28.0.0`'s release notes say it **ungates CAP-83 and CAP-85** and
re-generates the XDR. The concrete Rust API delta is not enumerated there — it is
`cargo check` that will tell us, exactly as it did for 26→27.

### ⚠️ The break may not land on the activation ledger

Proto27's wall was a clean boundary. This one probably is not: CAP-83 adds a new
union discriminant, so `stellar-xdr 27` should decode Protocol 28 ledgers fine
**until the first ledger that actually carries an empty tx set**. That could be
hours or days after the vote, and it will look unrelated to the upgrade when it
arrives.

**Confirm this against the CAP text before choosing a deploy window.** If it
holds, it removes the "nothing broke at 17:00, we're fine" false all-clear — and
it is an argument for deploying *before* the vote rather than watching after it.

## 🔴 Blocker — BE must bump `xdr-parser` first

**We cannot do this unilaterally.** `prices-ingest-core/src/decode.rs` calls
BE's `xdr_parser::decompress_zstd` + `deserialize_batch` and consumes the
`LedgerCloseMeta` values they return. The type crosses the crate boundary.

Current state, read 2026-09-11:

| | pin |
|---|---|
| ours, `Cargo.toml:28` | `stellar-xdr = { version = "=27.0.0" }` |
| ours, `Cargo.toml:31` | `xdr-parser = { git = "…soroban-block-explorer.git", branch = "develop" }` |
| locked `xdr-parser` rev | `d61b359f` — the BE #325 merge from the proto27 bump; unmoved since |
| BE, `Cargo.toml:40` | `stellar-xdr = { version = "27" }` — i.e. `^27`, will **not** resolve 28 |

If we bump to 28 while `xdr-parser` still compiles against 27, Cargo pulls
**both** major versions into the graph and `decode_object` stops type-checking —
a hard compile error, not a warning. So the order is fixed:

1. **BE bumps `xdr-parser`'s `stellar-xdr` to 28** and merges to `develop`.
2. We re-pin `=28.0.0` and re-pin `xdr-parser` to the resulting merge rev.

⚠️ `xdr-parser` is deliberately tracked on `branch="develop"`, not a rev — see
[[xdr-parser-develop-branch-intentional]]. That means BE's bump lands in our
build on the next `cargo update`, whether or not we are ready, so the two halves
want to move together rather than drift.

➡️ **Raise it with BE now.** This is the long pole and the vote is days out.

## Implementation

- Ask BE to bump `xdr-parser` to `stellar-xdr 28`; agree a rev.
- Bump the workspace pin to `=28.0.0`, keeping the exact-pin style 0091 chose.
- Re-pin `xdr-parser` to BE's merge rev; keep the `branch="develop"` tracking.
- `cargo check --workspace` and fix the API delta. Known surface — the four
  crates that name the dependency:
  - `packages/prices-ingest-core` (`decode.rs`, `filter.rs`, `soroban.rs`,
    `canonical.rs`, `error.rs`, `examples/decode_probe.rs`)
  - `packages/asset-discovery` (`lib.rs`, `symbols.rs`)
  - `packages/sdex-backfill` (`ingest.rs`, `error.rs`)
  - `packages/oracle-worker` (`lib.rs`)
- Check the three `LedgerCloseMeta` match sites in particular — `decode.rs:23`,
  `filter.rs:125` / `:146`, `soroban.rs:164` / `:184`. They match `V0/V1/V2`
  exhaustively, so a new variant is a compile error, which is the good case.
- Decide what an empty-tx-set ledger should do on our path. It carries no
  trades, so the honest handling is "decode it, extract nothing, advance the
  cursor past it" — the cursor must still advance or we stall on a ledger that
  was never going to produce a candle.
- Deploy the ledger-processor. ⚠️ [[0141]] / [[deploy-ships-stale-lambda-assets]]
  — `deploy-production-compute` does **not** build; confirm the asset changed.
- Verify the crossing, as [[0094]] did: watch `max(timestamp)` per source in
  `price_ohlcv_1m` advance past the activation ledger rather than freeze at it.

## Acceptance Criteria

- [ ] BE has bumped `xdr-parser` to `stellar-xdr 28` and the rev is recorded here.
- [ ] Workspace pin is `=28.0.0`; `cargo check --workspace` and the full test
      suite are green.
- [ ] Any new `LedgerCloseMeta` / `StellarValue` variant is handled explicitly,
      and an empty-tx-set ledger **advances the cursor** instead of stalling it.
- [ ] The bumped ledger-processor is **deployed** and the deployed asset is
      confirmed changed — not merely merged. 0091 merged on 2026-07-14 and prod
      stayed frozen until 0094 deployed it.
- [ ] The live candle frontier is measured crossing the Protocol 28 activation
      ledger, recorded before/after — the same check that resolved proto27's
      active-vs-latent question.
- [ ] `prices-production-rollup-freshness-1m` stayed OK through the crossing
      (or fired and was cleared), so the alarm is confirmed to cover this failure
      rather than assumed to.

## Notes

### The alarm coverage is better than it was in July — verify, don't assume

The rollup-freshness alarms from [[0137]] measure **data**, not MV exit status,
and `observability-stack.ts` describes them as covering *"the rollup chain
feeding it is stalled, **or upstream ingestion has halted**"*. So a repeat of the
proto27 freeze should now page within the staleness bound rather than run six
days unseen. That is worth confirming during the crossing rather than trusting —
it is the first real test of that alarm against this specific failure.

### This is the second time the detection mechanism was "somebody read a blog"

[[0098]] — *"Version-gap CI guard: surface stellar-xdr protocol lag before it
freezes prod"* — was spawned from 0094 precisely so the next protocol bump would
surface on its own. It is still in `backlog/`, unbuilt, and Protocol 28 reached
us the same way Protocol 27 did: by chance. Worth pulling 0098 forward alongside
this task; it is tagged `effort-small`.
