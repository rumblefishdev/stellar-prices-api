---
id: "0091"
title: "Bump stellar-xdr 26→27 for Protocol-27 decode (unfreeze live ingestion)"
type: BUG
status: completed
related_adr: []
related_tasks: ["0090", "0094"]
tags: ["milestone-M1", "priority-high", "effort-small", "phase-live"]
links:
  - "https://stellar.org/blog/foundation-news/stellar-zipper-protocol-27-upgrade-guide"
  - "BE PR #325 (lore-0368) fix/0368_stellar-xdr-27-proto27-decode"
  - "BE PR #323 (lore-0367) galexie proto27 hardening"
history:
  - date: 2026-07-14
    status: backlog
    who: okarcz
    note: >
      Created. prices CH live frontier stale at ledger ~63,384,067 /
      2026-07-08 12:31 in the Protocol-27 window. We are pinned to
      stellar-xdr 26.0.0 (BE PR #325 parse-fails from ledger 63,401,875).
      Distinct from the Phase-1 backfill stop at 62,642,957 (task 0090).
  - date: 2026-07-14
    status: backlog
    who: okarcz
    note: >
      UPDATE: processor is NOT frozen — ledger-processor Lambda is healthy
      and actively reconciling/catching up (16 ledgers/inv, ~500 rows, zero
      parse errors) at ledger ~63,362,7xx as of 10:20 UTC, below the stale
      frontier. proto27 decode impact on OUR path is UNCONFIRMED: the
      reconcile had not yet crossed 63,401,875. VERIFY at the crossing
      (~14:00 UTC): stall/parse-fail there => 0091 is an active blocker;
      sails through => 0091 is latent hardening. DLQ empty + 14d retention
      => no redrive/replay needed; recovery is the processor re-ingesting.
  - date: 2026-07-14
    status: completed
    who: okarcz
    note: >
      CODE COMPLETE + MERGED. stellar-xdr 26.0.0→27 (exact-pin =27.0.0),
      curr→crate-root migration across 7 crates + dump-swap-events; xdr-parser
      re-pinned to BE #325 merge d61b359f (kept on branch=develop by choice);
      decode_probe example relocated here. 0 SorobanCredentials usages → no new
      match arms needed. 73 unit tests pass; CI green. Squash-merged to develop
      as PR #104 (e17ed03). Operational remainder (DEPLOY the xdr-27
      ledger-processor before the 63,401,875 crossing, drain DLQ, replay frozen
      gap 63,384,068→tip, verify live advances, add a version-gap CI guard) is
      NOT done — spawned as task 0094. Archived as code-complete.
---

# Bump stellar-xdr 26→27 for Protocol-27 decode (unfreeze live ingestion)

## Summary

Stellar **Protocol 27 "Zipper"** activated on pubnet in the 2026-07-08/09
window. Our workspace is pinned to **`stellar-xdr 26.0.0`**, which cannot decode
proto27 ledger XDR, so **live prices ingestion is frozen**. Bump `stellar-xdr`
26→27 across the workspace, mirroring the already-merged BE fix (PR #325 /
lore-0368), then replay live over the frozen gap.

## Context

Measured in prices ClickHouse `price_ohlcv_1m`: all three live sources stop at
ledger **~63,384,067 / 2026-07-08 12:31** (sdex 63384067, aquarius 63384063,
phoenix 63384050). Two proto27 walls are in play, both BE-confirmed, and we
share both:

1. **Ledger-production supply stall** — BE PR #323 (lore-0367): Galexie's pre-27
   core couldn't produce proto27 ledgers (~16h, "wrote nothing to S3"). Our live
   consumes BE's shared feed, so it starved here — the **proximate** halt at
   63,384,067. BE already hotfixed Galexie to 27.0.0, so supply is restored.
2. **XDR decode incompatibility** — BE PR #325 (lore-0368): `stellar-xdr 26`
   cannot decode proto27; ledger **63,401,875** onward → `Parse` "XDR parse
   failed" → dead-letter. This is the wall directly ahead of our live frontier
   and the remaining blocker on **our** side.

Proto27 XDR changes (per official Zipper guide + BE PR #325): two new
`SorobanCredentials` variants (`AddressV2`, `AddressWithDelegates`), a new
`ENVELOPE_TYPE_SOROBAN_AUTHORIZATION_WITH_ADDRESS`, new host functions.
**`LedgerCloseMeta` / `TransactionMeta` / `ScAddress` shapes are unchanged** —
only the credential/envelope surface. `stellar-xdr 27` also collapses the
`curr`/`next` feature+module split, so `stellar_xdr::curr::*` must migrate to the
crate root and the `features = ["curr"]` pin is dropped.

**NOT this task:** the Phase-1 historical backfill stop at ledger 62,642,957
(~late May, ~759k ledgers below proto27) — an external mid-loop kill, tracked
under task 0090.

## Implementation

- Bump `stellar-xdr` `26.0.0` → `27` in `Cargo.toml` (workspace) and refresh
  `Cargo.lock`; drop any `features = ["curr"]` pin.
- Migrate all `stellar_xdr::curr::*` references to the crate root across the
  crates that touch XDR: `xdr_parser` (whichever crate provides it),
  `prices-ingest-core`, `sdex-backfill`, `asset-discovery`, `oracle-worker`.
  (`ledger_minute` in `packages/sdex-backfill/src/ingest.rs` uses
  `stellar_xdr::curr::LedgerCloseMeta` — one of the sites.)
- Handle the two new proto27 `SorobanCredentials` variants wherever credentials
  are matched (follow BE `crates/xdr-parser/src/op_source.rs` as the model);
  add covering tests. Our SDEX/AMM/oracle extraction likely doesn't read
  SorobanCredentials, so this may be a no-op match arm — verify.
- `cargo check --workspace --all-targets` + `cargo test` green.
- After deploy: replay/reprocess live ingestion from ~63,384,068 so the frozen
  gap (63,384,068 → tip) is filled; drain any proto27 DLQ.

## Acceptance Criteria

- [x] `stellar-xdr` is `27.x` in `Cargo.lock`; no `curr` feature pin remains.
      (Exact-pinned `=27.0.0` in `Cargo.toml`; lock at 27.0.0.)
- [x] Workspace compiles and tests pass with the `curr`→crate-root migration.
      (73 unit tests pass across decode/extract crates; CI green on PR #104.)
- [~] A real proto27 ledger (≥ 63,401,875) decodes without a parse error.
      Decode path is BE's `xdr-parser` at #325 (verified there against real
      proto27 ledgers, 287+2 tests); `decode_probe` example compiles at xdr 27
      but has NOT yet been run locally against a ≥63,401,875 file. Live-decode
      confirmation → **0094**.
- [ ] Live ingestion advances past ledger 63,401,875 in prices CH — **deferred
      to 0094** (requires deploying the xdr-27 processor).
- [ ] Live DLQ drained; no residual "XDR parse failed" errors — **deferred to 0094**.
- [ ] Frozen gap (63,384,068 → tip) backfilled/replayed — **deferred to 0094**.

## Implementation Notes

Merged to `develop` as **PR #104** (squash `e17ed03`), base `develop`. Files
touched (12): `Cargo.toml` + `Cargo.lock`; `stellar_xdr::curr::*` → `stellar_xdr::*`
in `asset-discovery`, `oracle-worker`, `prices-ingest-core` (`canonical`/`decode`/
`filter`/`soroban`), `sdex-backfill/ingest`, and the `dump-swap-events` tool; new
`prices-ingest-core/examples/decode_probe.rs` (relocated from PR #103). No
production-logic change — a pure version bump + module migration guarded by the
compiler.

## Design Decisions

### From Plan

1. **26→27 workspace-wide `curr`→crate-root migration** exactly as scoped; the
   `features = ["curr"]` pin dropped (v27 collapsed the curr/next split).

### Emerged

2. **No SorobanCredentials match arms added.** The plan expected possible new
   match arms for the two proto27 credential variants. We reference
   `SorobanCredentials` in **0 places**, so — unlike BE #325 — none were needed.
   `deserialize_batch` still requires v27 because it parses whole ledgers eagerly.

3. **`xdr-parser` re-pinned to BE's #325 *merge commit* `d61b359f`, not develop
   HEAD** — the minimal commit carrying stellar-xdr 27, avoiding 28 unrelated
   xdr-parser commits.

4. **`xdr-parser` kept on `branch = "develop"` (float), NOT `rev`-pinned.**
   A code-review (PR #104) flagged the missing rev-pin as fragile; user chose to
   keep it tracking develop so it auto-follows BE's decode stack. `stellar-xdr`
   IS exact-pinned (`=27.0.0`) — deliberate asymmetry.

5. **`stellar-xdr` tightened from `27` (caret) to `=27.0.0` (exact)** post-review,
   so `cargo update` can't float to a 27.x that diverges from xdr-parser's types.

6. **Task archived as code-complete with the operational tail split to 0094**,
   mirroring the 0053→0088 pattern (code merged ≠ run/deployed).

## Issues Encountered

- **Task record stranded on an unmerged branch.** The 0091 file was spawned on
  the still-open PR #103 (`docs/0090`) branch, so on `develop` the session
  `current-task.md` symlink dangled after #104 merged. Reconciled by completing +
  archiving 0091 here on the #103 branch (per user decision), then clearing the
  dangling session task on `develop`.

## Future Work

Spawned as **task 0094** (operational remainder — priority-high, time-sensitive):
deploy the xdr-27 `ledger-processor` before the 63,401,875 crossing, drain the
live DLQ, replay the frozen gap (63,384,068 → tip) for all live sources, confirm
`price_ohlcv_1m` max ledger climbs past 63,401,875, and add a version-gap CI
guard so the next protocol bump surfaces before it freezes prod.
