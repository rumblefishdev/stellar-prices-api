---
id: "0022"
title: "SDEX filter predicates and extraction spec for the dedicated archive-read backfill"
type: RESEARCH
status: completed
related_adr: ["0002"]
related_tasks: ["0012", "0013", "0020", "0023", "0024", "0025"]
tags: [priority-high, effort-medium, research, sdex, backfill, stream-2, xdr, parser]
links:
  - "../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/R-sdex-operation-xdr-shape.md"
  - "../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-13
    status: backlog
    who: okarcz
    note: >
      Spawned by ADR 0002 to consolidate the SDEX filter + decode
      contract for the dedicated, BE-independent Stream 2 archive
      backfill. Builds on archived task 0020's R-note (XDR shape
      walk) and G-note (extractor algorithm + TradeTick shape).
      Output is the consumer-ready specification that task 0012's
      Rust implementation module consumes.
  - date: 2026-05-13
    status: active
    who: okarcz
    note: >
      Promoted to active. Use the XDR sample files in `/.temp` as
      example Stellar ledger data for the filter-profile run and
      worked examples. The `xdr-parser` crate from
      `../soroban-block-explorer` is available as a reference
      implementation for XDR decoding if the stellar-xdr crate's
      surface needs adapting.
  - date: 2026-05-13
    status: completed
    who: okarcz
    note: >
      Spec complete. 2 G-notes (1020 lines), profile harness (827
      lines Rust + 96-line results.md), 3 real-data worked
      examples from 2000-ledger mainnet sample. Profile: 311
      ledgers/s decode, 99.35% trade-bearing density, 80/20 LP/OB
      split. End-to-end backfill estimate: 12-16 days single-task
      (archive transport is the bottleneck, not decode). Three
      follow-up tasks spawned: 0023 (OHLCV row identity ADR), 0024
      (volume_quote_usd enrichment), 0025 (live multi-source
      merge contract).
---

# SDEX filter predicates and extraction spec for the dedicated archive-read backfill

## Summary

Produce the consolidated specification that the Stream 2 SDEX backfill
(ADR 0002 / task 0012) implements. Output is one or two notes in this
task's `notes/` directory: a precise filter contract (which ledgers
contain trade-bearing operations and how to detect them without parsing
every op fully), and a precise decode + bucket contract (how each
matching ledger turns into `TradeTick`s and how those bucket into
`price_ohlcv` rows under ON CONFLICT semantics).

This task takes the **research** outputs from archived task 0020 (R-note
on XDR shape, G-note on extraction design) and produces the
**implementation contract** for task 0012 to code against. No re-research
of the XDR layout — that's settled.

## Context

ADR 0002 commits Stream 2 to archive reads from ledger 1 to tip, with
the BE-authored `stellar-xdr` parser crate as a library dependency. The
trade-shaped op types are the five identified in 0020/R-note (types
2, 3, 4, 12, 13). The price-tick unit is `ClaimAtom` (V0 / ORDER_BOOK /
LIQUIDITY_POOL). The G-note in 0020 sketches the extractor algorithm
and `TradeTick` shape.

What's still open and this task closes:

1. **Filter strategy** — how to cheaply identify trade-bearing ledgers
   without fully parsing every operation in every transaction. Options
   include: parse `TransactionResultMeta` only and short-circuit on
   absence of `ClaimAtom` arrays; pre-filter on `operation.body.type`
   discriminant before touching results; ledger-level early-out via
   counts in `LedgerHeader`. Pick one with measured CPU cost.

2. **Pair canonicalisation** — `ClaimAtom` records `asset_sold` /
   `asset_bought`. Token A / Token B canonical ordering for
   `price_ohlcv` is asset-id-comparison-based (already implied in
   G-note) but the SDEX-specific edge cases (native XLM as one side,
   classic vs SAC for the same asset, liquidity-pool leg attribution)
   need a single source-of-truth table.

3. **`ClaimAtom` variant handling** — V0 (legacy, ≤ protocol 17),
   ORDER_BOOK (modern), LIQUIDITY_POOL (path-payment-through-AMM).
   Each carries the same trade-shaped fields but the parent
   operation context differs; spec the per-variant decode.

4. **`price` computation per `ClaimAtom`** — `amount_bought /
   amount_sold` is the spot price in (bought-units-per-sold-unit).
   Spec the precision strategy (NUMERIC(28,14)), the asset-side
   decimals normalization (stellar classic = 7 stroops; SAC = ?), and
   the failure mode for divide-by-zero or extreme-precision losses.

5. **Bucket + UPSERT contract under ON CONFLICT** — §5.2's UPSERT
   write semantics apply, but the backfill is processing ledgers in
   bulk (potentially out of order if implementation chooses chunked
   scan). Spec the (`asset_id`, `timestamp`, `granularity='1m'`)
   merge rules for backfill writers: preserve `open` (lowest
   ledger-time wins within the minute), overwrite `close` (highest
   ledger-time wins), `GREATEST(high)` / `LEAST(low)`, sum volumes
   and `trade_count`, recompute `vwap`. Match the live-ingestion
   contract exactly so backfill and live converge on the same row
   shape if they touch the same minute.

6. **Resumability checkpoint contract** — what gets written to
   `backfill_progress.current_ledger` and when. Per-ledger?
   Per-chunk? Spec atomicity guarantee (a crash mid-ledger must not
   double-count on resume).

7. **Asset discovery side-effects** — when a `TradeTick`'s asset is
   not in the `assets` table yet, what does the backfill do? Insert
   on the fly? Skip and let the Asset Discovery Lambda backfill the
   gap? Spec the decision.

## Implementation

Run as a pure spec task — no code lands. Produce:

- `notes/G-sdex-filter-strategy.md` — answers 1, 6, 7. Includes the
  measured CPU cost of the chosen filter (parse one sample ledger,
  profile).
- `notes/G-sdex-decode-and-bucket-spec.md` — answers 2, 3, 4, 5.
  Includes one worked example per `ClaimAtom` variant (V0 +
  ORDER_BOOK + LIQUIDITY_POOL), with input XDR bytes → output
  `TradeTick` → resulting `price_ohlcv` row delta.

Both notes are written so that task 0012's Rust implementation can be
graded against them clause-by-clause.

## Acceptance Criteria

- [x] `notes/G-sdex-filter-strategy.md` covers (1) filter strategy,
      (6) checkpoint contract, (7) asset-discovery side-effects.
      Filter strategy is recommended with a CPU-cost note (rough
      profile, not full benchmark).
- [x] `notes/G-sdex-decode-and-bucket-spec.md` covers (2) pair
      canonicalisation, (3) per-`ClaimAtom`-variant decode, (4) price
      computation + precision, (5) UPSERT bucket merge rules.
- [x] One worked example per `ClaimAtom` variant in the decode spec.
      ORDER_BOOK + LIQUIDITY_POOL extracted from real mainnet ledgers
      (62 435 141, 62 442 947) via the profile harness; V0
      synthesized from the protocol XDR spec because V0 is
      pre-protocol-18 and absent from the modern sample.
- [x] Cross-links to 0020's R-note (XDR shape) and G-note (algorithm)
      — no duplication, just extension.
- [x] Task 0012's implementation acceptance criterion ("spec from
      task 0022 is folded into the Rust implementation module")
      is satisfiable: the spec is sectioned so it maps 1:1 onto
      `filter` / `decode` / `bucket` Rust modules. Mapping table
      lives in decode-spec §7 + filter-spec §5.

## Implementation Notes

Landed across 4 commits on `research/0022_sdex-filter-and-extraction-spec`
(PR [#9](https://github.com/rumblefishdev/stellar-prices-api/pull/9)):

| Commit    | Scope                                                        |
| --------- | ------------------------------------------------------------ |
| `579b12f` | Convert task file → directory (`notes/` for spec output)     |
| `0fd6d87` | Add `notes/profile/` Rust harness + 2000-ledger profile data |
| `fc83e02` | Draft both G-notes (1020 lines combined)                     |
| `6cee920` | Add §4 end-to-end backfill runtime estimate to filter spec   |

Notes produced:

- `notes/G-sdex-filter-strategy.md` (~450 lines) — filter, checkpoint,
  asset discovery, end-to-end estimate.
- `notes/G-sdex-decode-and-bucket-spec.md` (~660 lines) — canonicalisation,
  per-variant decode, price math, 1m bucket UPSERT.

Profile harness (`notes/profile/`):

- Standalone Cargo crate (3 binaries: `profile`, `dump-examples`,
  `dump-canonical`); not a workspace member of the main project.
- Uses `stellar-xdr v26` directly (mirrors BE `xdr-parser`'s decode
  setup); does not depend on the BE crate.
- Inputs: Galexie-format `.xdr.zst` files in `.temp/` (gitignored
  developer-local).
- Outputs: `results.md` (timing + density), `examples/*.json` (3
  real ClaimAtoms — ORDER_BOOK / LIQUIDITY_POOL / multi-claim
  canonical ManageSellOffer).
- `target/` is gitignored.

Key numbers from the 2 000-ledger profile (recent mainnet, protocol 22):

- Decode: 3.22 ms/ledger mean, **311 ledgers/s single-thread**.
- ClaimAtom walk: 9 µs/ledger (3 orders of magnitude cheaper than decode).
- Trade-bearing ledger density: **99.35 %**.
- Variant share: V0 0 % / ORDER_BOOK 20.36 % / LIQUIDITY_POOL 79.64 %.
- Tx success rate: 34.69 % (65.31 % of all txs failed —
  MEV/bot retries).

## Design Decisions

### From Plan

1. **Profile harness lives under the task directory, not main
   workspace.** The main `stellar-prices-api` is a TS/Nx skeleton
   today; Rust impl lands in task 0012. Profile is a research
   artifact, not production code — `notes/profile/` keeps it
   alongside its motivating spec.

2. **Three real-data worked examples** (ORDER_BOOK + LIQUIDITY_POOL +
   multi-claim ManageSellOffer) sourced from `.temp/` samples via
   the `dump-examples` + `dump-canonical` binaries. The README
   asked for one example per variant; we delivered that plus a
   multi-claim canonical SDEX trade for completeness.

3. **Backfill UPSERT semantics: whole-row replacement.** Schema
   doc `database-schema-overview.md` L362–365 already commits to
   this for SDEX backfill; spec agrees and contradicts the
   README's point (5) text (which described live-ingestion
   incremental merge). Documented why in decode spec §5.4.

### Emerged

4. **Filter framing collapsed by `stellar-xdr` atomic decode.** The
   README's three filter options (parse `TransactionResultMeta`
   only, pre-filter on `OperationBody.type`, header-only early-out)
   all assume partial XDR decoding is cheap. With the `stellar-xdr`
   crate's `from_xdr` it isn't — decode is atomic. Spec
   re-frames "filter" as "post-decode `OperationResultTr` variant
   walk" (filter spec §1.1–1.3). The user-facing answer to the
   README question changed in spirit but not in conclusion.

5. **V0 example synthesized, not real.** V0 `ClaimAtom`s are
   pre-protocol-18 (≤ early 2022); the modern `.temp/` sample is
   protocol 22 and contains none. Decode spec §3.3 reconstructs V0
   from the protocol XDR spec and clearly marks it synthesized.
   Acceptable because the V0 variant only differs in counterparty
   shape (raw ed25519 vs `AccountID`); trade-shaped fields are
   identical to ORDER_BOOK.

6. **Three follow-up backlog tasks spawned (0023, 0024, 0025).**
   The decode spec surfaced three load-bearing gaps that this
   research task did not have the scope to resolve unilaterally:
   - OHLCV row identity (`asset_id`-only PK vs `(asset_id, quote_asset_id)`)
     is a schema-change ADR;
   - `volume_quote_usd` enrichment needs an oracle-join pass;
   - multi-source live-merge contract is outside backfill scope.

7. **End-to-end runtime estimate added after first PR push.**
   User asked for the explicit decode→extract→write total. Added
   §4 to filter-strategy spec confirming §5.6's "~16 days
   single-task" figure with profile-data backing. Parallelisation
   table to ~4 tasks for ~3–4 day completion if needed.

8. **Branch + PR workflow correction.** Initial commits landed on
   `develop` directly; user asked for branch + PR. Created
   `research/0022_sdex-filter-and-extraction-spec`, force-moved
   develop ref back to `origin/develop`, pushed branch, opened
   PR #9. No work lost — commits preserved on the branch.

## Issues Encountered

- **`LedgerCloseMeta::V2` has a different `tx_processing` element
  type** (`TransactionResultMetaV1`) than V0/V1
  (`TransactionResultMeta`). First harness draft tried to share a
  closure across all three variants; cargo flagged the mismatch.
  Fix: introduced a `TxView<'a>` wrapper and per-variant `match`
  populating it (`src/lib.rs:tx_views`). Idiomatic and lets the
  three variants stay structurally similar.

- **`Hash`/`PoolId` aren't `Copy`.** Initial JSON-dump code did
  `hex::encode(a.liquidity_pool_id.0)` which is a move; fix:
  `hex::encode(a.liquidity_pool_id.0.as_slice())`.

- **Working-directory drift between Bash invocations.** After a
  `cd lore/.../profile`, subsequent shell invocations started
  fresh (new shell per `Bash` call). Worked around by using
  `--manifest-path` for cargo and absolute paths for everything
  else.

## Future Work

Three follow-up backlog tasks spawned. Brief context here;
detail in each task's README:

1. **[0023](../../backlog/0023_RESEARCH_ohlcv-row-identity-base-vs-pair.md)** —
   Decide OHLCV row identity (base-only vs (base, quote))
   before task 0012 implementation can start. Schema-change
   ADR needed.

2. **[0024](../../backlog/0024_FEATURE_volume-quote-usd-enrichment.md)** —
   Implement the `volume_quote_usd` enrichment pass that joins
   `oracle_prices` against the backfill's `price_ohlcv` rows
   for non-USD-quoted pairs.

3. **[0025](../../backlog/0025_RESEARCH_live-multi-source-merge-contract.md)** —
   Spec the live writer-side multi-source merge contract
   (`source = 'sdex' → 'aggregated'` transition rules) for the
   Prices Ledger Processor. Out of scope for backfill spec.

## Notes

- This task did **not** re-litigate Option A vs Option B from task
  0020. ADR 0002 already settled that. The filter strategy explored
  here is in-binary filtering on XDR-decoded ops, not BE-CH-sourced
  pre-filtering.
- This task did **not** explore parallelisation / sharding strategies
  for the backfill in depth — that's a task 0012 implementation
  concern. Filter spec §4.3 sketches the disjoint-range
  parallelisation pattern but stops short of operational detail.
- Estimated effort was 2-3 days; actual delivery: 1 session.
