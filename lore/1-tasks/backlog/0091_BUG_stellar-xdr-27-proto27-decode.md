---
id: "0091"
title: "Bump stellar-xdr 26→27 for Protocol-27 decode (unfreeze live ingestion)"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0090"]
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

- [ ] `stellar-xdr` is `27.x` in `Cargo.lock`; no `curr` feature pin remains.
- [ ] Workspace compiles and tests pass with the `curr`→crate-root migration.
- [ ] A real proto27 ledger (≥ 63,401,875) decodes without a parse error
      (probe via `packages/prices-ingest-core/examples/decode_probe.rs` or a
      targeted test).
- [ ] Live ingestion advances past ledger 63,401,875 in prices CH
      (`price_ohlcv_1m` max ledger climbs toward the current tip).
- [ ] Live DLQ drained; no residual "XDR parse failed" errors.
- [ ] Frozen gap (63,384,068 → tip) backfilled/replayed for all live sources.

## Future Work

- Verify whether our live processor was actively dead-lettering (wall #2) vs
  merely idle since the supply stall — inspect the live-ingestion Lambda DLQ
  depth + error logs (outside ClickHouse).
- Consider a CI guard / renovate policy so a future protocol bump surfaces the
  `stellar-xdr` version gap before it freezes prod.
