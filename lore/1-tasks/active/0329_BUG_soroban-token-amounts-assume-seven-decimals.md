---
id: "0329"
title: "Soroban AMM amounts are scaled as if every token had 7 decimals — every contract token's price is off by 10^(7 − decimals)"
type: BUG
status: active
assignee: stkrolikiewicz
related_adr: []
related_tasks: ["0286", "0242", "0210", "0097", "0060", "0048", "0018"]
tags: [layer-backend, priority-high, effort-medium, ingest, soroban, data-correctness, defect]
links:
  - "../../../packages/prices-ingest-core/src/soroban.rs"
  - "../../../packages/asset-discovery/src/symbols.rs"
  - "../../../docs/database-schema/amm-trades-schema.md"
history:
  - date: "2026-10-06"
    status: active
    who: stkrolikiewicz
    note: >
      Renumbered from 0328. This task was committed as 0328 but its push did
      not land, and another 0328 (board page assignee) was pushed and merged
      meanwhile. An unpushed task reserves no ID.
  - date: "2026-10-06"
    status: active
    who: stkrolikiewicz
    note: >
      Activated for implementation with the proposed design accepted:
      SEP-41 decimals() over RPC, resolved by the ingest on a cache miss,
      persisted in prices.asset_decimals, never defaulting to 7.
  - date: "2026-10-06"
    status: backlog
    who: stkrolikiewicz
    note: >
      Reported by Karol (SBE) on 2026-10-05: SolvBTC, XAUM and XRP priced at
      10^(7 − decimals) of market. SBE holds off pricing pools that contain
      these tokens until we fix it. Root cause confirmed in code; decimals of
      all seven contract tokens the API lists were read on mainnet, and none
      is 7. Design for where decimals come from proposed below, not yet
      decided.
---

# Soroban AMM amounts are scaled as if every token had 7 decimals

## Summary

`amm_trade_to_tick` turns both raw i128 legs of every Soroban swap into
`Decimal` with one fixed scale, `AMM_AMOUNT_SCALE = 7`
([soroban.rs:37](../../../packages/prices-ingest-core/src/soroban.rs), used at
:859–860). Seven is right for a SAC (classic assets are 7-decimal by protocol)
and wrong for a pure contract token, which has its own `decimals()`. Against a
7-decimal counter-leg the stored price is `true × 10^(7 − d)`. The constant came
in with 0060's sizing backfill as a "documented sizing-measurement
approximation" and never got replaced, though 0018 and 0048 §7.2 had both
specified reading `decimals()`. `amm-trades-schema.md` §3 still lists decimals
normalisation as "_Still open._"

## Evidence (2026-10-05)

`/v1/assets?type=soroban&min_volume_usd=0` lists 7 assets. Decimals were read
with `stellar contract invoke --network mainnet --send no -- decimals`.

| Token | Contract | decimals | Our price | × 10^(d−7) | Corrected |
|---|---|---|---|---|---|
| SolvBTC | `CBIJBDNZ…M6VN` | 8 | 8 580.10 | ×10 | ~85 800 |
| XAUM | `CC2RBGYN…VAGO` | 9 | 41.92 | ×100 | ~4 192 |
| XRP | `CB7OOP3V…37XP` | 6 | 14.78 | ÷10 | ~1.48 |
| deJTRSY | `CBI7UCH5…IHRV` | 18 | 1.026e-11 | ×10¹¹ | ~1.03 |
| deJAAA | `CC64WBDG…YGSL` | 18 | 1.05e-11 | ×10¹¹ | ~1.05 |
| BnUSD | `CCT4ZYIY…X7KP` | 18 | 9.98e-12 | ×10¹¹ | ~0.998 |
| xSolvBTC | `CAUP7NFA…772J` | 8 | unpriced | — | — |

**Not proven:** that the other leg in every priced pool is 7-decimal. This is
inferred only from the corrected prices matching the market.

## Impact

- OHLC, `close_usd` and therefore `price_usd_series*` (views over the candle
  tables) are wrong for every contract-token asset, over its whole history.
- Base volume in token units is inflated by 10^(d − 7). USD volume is right,
  because the price and quantity errors cancel.
- An asset quoted *in* a contract token gets a correct USD price (the two
  errors multiply out), but its price in that token's units is wrong.
- 18-decimal tokens: a price near 1e-11 keeps only 3–4 significant digits in
  `Decimal(38,14)`. A token under about $0.001 falls below 1e-14 and its fills
  are classified non-price-forming (`price_survives_column_scale`). Rescaling
  stored rows can recover neither case, so history needs a re-ingest, not an
  `UPDATE`.
- `price_forming_i128` classifies the raw integer legs before scaling, so it
  is unaffected.

## Design: where decimals come from (accepted 2026-10-06)

**Source: the token's SEP-41 `decimals()`, read once via RPC `simulateTransaction`.**
This is the only universal source. Swap events carry raw i128 only. Instance
storage layout depends on the implementation (soroban-token-sdk `METADATA`,
OpenZeppelin, custom tokens). The live ingest also never sees an old token's
instance entry in ledger meta. A SAC is 7 without a call. A SAC we still hold
as a Contract identity ([[0242]]) answers 7 over RPC anyway, so it needs no
special case. Decimals are fixed at deploy for every real token, which is the
same "absence, not staleness" rule [[0210]] applies to `prices.asset_symbol`:
fetch once, never refresh.

**Store:** a new `prices.asset_decimals (contract_address, decimals, fetched_at)`,
ReplacingMergeTree. Live ingest and backfills read the same value.

**Resolve in the ingest on a cache miss, not in asset-discovery.** The price is
fixed at write time, and asset-discovery runs hourly, so a new token's first
swaps would land before its decimals do. That leaves a guessed price or a hole
to re-price by hand for every new token. Load the table at run start (it holds
tens of rows). A miss costs one RPC call with the existing 5 s timeout and is
persisted for every later run. Reuse `build_simulate_envelope` from
asset-discovery, which is already generic over the function name, by moving it
into a shared crate. Lambdas have egress (no VPC), and `SOROBAN_RPC_URL` is
already the convention.

**On failure, or decimals > 28 (rust_decimal's maximum scale): never fall back
to 7.** Drop the tick, record it next to `prices.unresolved_pools`, and repair
with `events-backfill` once the token resolves. A wrong price is worse than a
missing one.

Rejected:
- **Hard-coded map of the 7 tokens.** Every new token is mispriced or unpriced
  until a PR lands. Since the RPC code exists, the map is barely smaller.
- **Asset-discovery only.** Leaves an hourly gap and a manual re-price per new
  token.
- **Instance storage from ledger meta.** Layout varies by implementation, and
  old tokens never appear in the meta.

## Plan

1. Shared simulate helper and a `decimals()` resolver with the three-way
   outcome from `symbols.rs`.
2. `prices.asset_decimals` DDL.
3. `amm_trade_to_tick` scales each leg by its own decimals: SAC = 7, unresolved
   → drop and record. This is the only production call site of the scale.
4. Backfills (`events-backfill` and the soroban path of `sdex-backfill`) share
   the same resolver.
5. **History rides [[0286]] phase 3 if timing allows.** Stages B/C re-ingest the
   AMM era through `events-backfill`. If this fix is on `fishuser-hero` before
   stage C reaches the months these tokens traded, nothing extra runs.
   Otherwise, a targeted `events-backfill` plus pre-roll over those pools'
   range. Coordinate with the phase-3 operator.
6. Tell Karol (SBE) when prices are correct.

## Implementation (branch `fix/0329_soroban-token-amounts-assume-seven-decimals`)

- `prices_ingest_core::soroban_rpc`: the `simulateTransaction` envelope and the
  Absent/Transient boundary, moved from asset-discovery's `symbols.rs`, which now
  delegates to it and re-exports the old names.
- `AssetRegistry::decimals_of`: classic identities and SACs → 7, a `Contract`
  → its resolved decimals or `None`. `amm_trade_to_tick` scales each leg by its
  own decimals. An unknown leg yields no tick, interns nothing, and lands in
  `LedgerSoroban::missing_decimals`.
- `prices.asset_decimals` + `OhlcvWriter::{load,write}_decimals` +
  `DecimalsResolver` (`decimals()` must be a `U32` ≤ 28).
- Callers decode, resolve what was missing, persist it, record it, and decode
  that ledger again: the live reconcile loop, `events-backfill`, and
  `sdex-backfill` in `combined` mode.

### Design decisions

#### Emerged

1. **Dropped trades are counted, not recorded in a table.** `unresolved_pools`
   is keyed by pool, so there is no row for a token. Instead the live processor
   counts them on `RunStats`, WARNs with the contracts, and publishes
   `TradesMissingDecimals`, alarmed like `UnregisteredPoolEvents`. Both
   backfills print `trades dropped (decimals)`, and `reingest_0286.py` marks a
   month with a non-zero count DEFECT. Added after review (PR #395).
2. **Absent is terminal per process, Transient retries after 10 minutes.** A
   contract that answered with no usable scale is not asked again until a cold
   start, and is never persisted as a sentinel. A contract that gave no answer
   is retried after 10 minutes. Calls run concurrently, so one ledger costs one
   RPC timeout at worst.
3. **Decode again rather than a two-phase tick.** Decoding is deterministic and
   the registry inserts are idempotent map writes, so the first result is
   discarded whole.
4. **`CandleSink::write_decimals` defaults to a no-op.** Only `ClickHouseSink`
   persists. The test sinks need no change.
5. **`sdex-backfill` reads the table only in `combined` mode.** The 0286 phase-3
   `sdex-only` runs therefore do not need the table to exist.
6. **An `events-backfill` dry-run resolves but does not persist.**
7. **Phase-3 volume reconcile leaves out contract-token candles.** Their volume
   moves by design (an 18-decimal token's base volume falls by 10^11), so
   `reingest_0286.py` compares volumes on the other candles only and trades on
   all of them. A "before" read without that scope compares trades only, as a
   FINDING. Added after review (PR #395).

### Deploy order

1. Apply `init.sql` on the box: it creates `prices.asset_decimals`. The live
   processor's init fails without it.
2. Optional pre-seed, so the first cold start needs no RPC call (the Lambda has
   a 60 s budget):
   `INSERT INTO prices.asset_decimals (contract_address, decimals) VALUES ('CBIJBDNZNF4X35BJ4FFZWCDBSCKOP5NB4PLG4SNENRMLAPYG4P5FM6VN', 8), ('CBI7UCH5KGSVQRO5H4SUCZUTZABCITZLRHQQZTWL2TK4RZ72TAR6IHRV', 18), ('CB7OOP3VSAWBZOOTOG2YEFANVU45GVWYUUM5HI32DKLHVKUDOFVQ37XP', 6), ('CC2RBGYNCFBCVENIDL5BFBWPH4OUZM2UA3OD2K2N54GLMWCC4KWPVAGO', 9), ('CC64WBDGS6QQP22QTTIACYIXT3WF7BBQEYOQPLTP7GTKYY7PZ74QYGSL', 18), ('CCT4ZYIYZ3TUO2AWQFEOFGBZ6HQP3GW5TA37CK7CRZVFRDXYTHTYX7KP', 18), ('CAUP7NFABXE5TJRL3FKTPMWRLC7IAXYDCTHQRFSCLR5TMGKHOOQO772J', 8)`
3. Deploy the ledger processor.
4. Repair history right away. Until it is repaired, `vwap_24h` blends old and
   new candles for 24 h, and a `carried` price keeps the old candle. Run
   `events-backfill` + pre-roll at least over a recent window, and the full
   range via 0286 phase 3 if stage C has not passed these months yet.

## Acceptance Criteria

- [x] Unit test: an 8-, 18- and 6-decimal leg against a 7-decimal leg gives the
      true price and the true base volume
      (`each_leg_is_scaled_by_its_own_tokens_decimals`).
- [x] A token whose decimals are unresolved never produces a price
      (`a_leg_with_unknown_decimals_prices_nothing_and_is_reported`).
- [ ] After deploy, all six priced tokens are within a few percent of an
      external reference.
- [ ] History of the affected assets re-ingested; `price_usd_series*` right over
      the full range, with significant digits restored for the 18-decimal
      tokens.
- [x] `amm-trades-schema.md` §3 closed; the `AMM_AMOUNT_SCALE` constant removed.
