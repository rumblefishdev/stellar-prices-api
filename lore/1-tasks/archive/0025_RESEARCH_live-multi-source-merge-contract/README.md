---
id: '0025'
title: "Live multi-source merge contract: source=sdex → 'aggregated' transition rules"
type: RESEARCH
status: completed
related_adr: ['0003', '0004']
related_tasks: ['0022', '0012', '0023', '0024']
tags:
  [
    layer-research,
    priority-medium,
    effort-small,
    ohlcv,
    live-ingestion,
    multi-source,
  ]
links:
  - '../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md'
  - '../archive/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/notes/S-recommendation.md'
  - '../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md'
  - '../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md'
  - '../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md'
  - './notes/G-merge-contract-spec.md'
  - '../../../docs/database-schema/database-schema-overview.md'
history:
  - date: 2026-05-13
    status: backlog
    who: claude
    note: >
      Spawned from 0022 future-work item 3. Decode spec §5.6 sketched
      the multi-source merge pattern but explicitly scoped the live
      writer-side contract out.
  - date: 2026-05-13
    status: active
    who: okarcz
    note: >
      Promoted to active. Scope is design-only — produces a G-note
      specifying the writer-side multi-source merge contract.
      Implementation lives in the Prices Ledger Processor when task
      0012 lands. ADR 0003's `quote_asset_id` PK makes the merge
      collision well-defined (per-native-pair, per-minute).
  - date: 2026-05-13
    status: completed
    who: okarcz
    note: >
      G-note + ADR 0004 (accepted) landed via PR #12. Three open
      questions answered: (1) writer-side detection via shared
      Rust merge library, (2) per-column merge rules including
      timestamp-ordered open/close, (3) per-row source breakdown
      stored in new `sources_seen JSONB` column. Schema additions
      land in task 0012's pre-backfill bootstrap. No spawn task —
      impl distributes across 0012 + each writer's own task.
---

# Live multi-source merge contract

## Summary

The schema doc commits to a `source = 'aggregated'` row when a
1-minute candle for `(asset, minute, '1m')` is touched by multiple
distinct sources (SDEX, Soroswap, Aquarius, …). Task 0022's
decode-and-bucket spec covered the **backfill** contract (write
`source = 'sdex'` for SDEX-only rows; trust downstream to merge);
this task specs the **live writer-side** contract.

## Context

Per `docs/database-schema/database-schema-overview.md` §"Source
attribution":

> When the same `(timestamp, asset, granularity)` is written by
> multiple distinct sources … the writer uses `source =
'aggregated'` and merges across sources. Single-source candles
> keep their original source label.

Open questions:

1. **Who initiates the merge?** Three options:
   - Each live writer detects `source != self.source` on UPSERT
     conflict and rewrites to `'aggregated'`.
   - A separate Current Price Updater–adjacent process scans
     `price_ohlcv` and consolidates multi-touched rows.
   - The Rollup Lambda merges when it re-aggregates from 1m.

2. **What numeric merge happens?** Sum volumes? Volume-weighted
   re-aggregation of OHLC from the underlying ticks (requires
   ticks to be persisted, which 0022's backfill does not do)?
   Argmin/argmax for open/close across sources by their internal
   timestamps?

3. **Is the source attribution losslessly recoverable?** Once a
   row is `'aggregated'`, can downstream consumers see which
   constituent sources contributed? (The `sources` JSONB on
   `current_prices` carries per-source breakdown but
   `price_ohlcv` doesn't.)

## Implementation

- Survey: identify the consumer paths that read `source` from
  `price_ohlcv`.
- Decide who initiates the merge.
- Spec the numeric merge formula (likely: each source contributes
  its own complete candle; the aggregated row stores
  volume-weighted everything across sources, with open/close
  determined by source-internal timestamp).
- Decide whether to persist constituent source list per row
  (extra `sources_seen JSONB` column?) or rely on `current_prices`
  for that information.

## Acceptance Criteria

- [x] Specification note in this task's `notes/` directory.
      → [`notes/G-merge-contract-spec.md`](./notes/G-merge-contract-spec.md).
- [x] Schema implication (new column? new table?) documented and
      either implemented or spawned as a separate schema task.
      → Three new columns spec'd; ADR 0004 (accepted) records the
      decision; implementation lands in task 0012's pre-backfill
      schema bootstrap.
- [x] Live writer implementation guidance for the Prices Ledger
      Processor (task 0012's eventual sibling).
      → Merge formula table in G-note §3.1; shared Rust library
      sketch in §1.3; SQL `ON CONFLICT DO UPDATE` clause in
      ADR 0004.

## Implementation Notes

Landed across 3 commits on `research/0025_live-multi-source-merge-contract`
(PR [#12](https://github.com/rumblefishdev/stellar-prices-api/pull/12),
squash-merged into develop as `6cd5e26`):

| Commit    | Scope                                           |
| --------- | ----------------------------------------------- |
| `d23f05c` | Convert task to directory                       |
| `81900b9` | Merge-contract design G-note (~450 lines)       |
| `d550180` | Draft ADR 0004 (proposed) + README link refresh |

Followed by completion + archive on develop (this commit).

Artifacts produced:

- [`notes/G-merge-contract-spec.md`](./notes/G-merge-contract-spec.md)
  (~450 lines) — eight-section design spec covering writer-side
  detection, schema additions, per-column merge rules, source
  attribution, backfill interaction, concurrency, acceptance gate,
  and open items.
- [`lore/2-adrs/0004_price-ohlcv-multi-source-merge-columns.md`](../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md)
  (~269 lines) — accepted ADR for the three schema additions.

## Design Decisions

### From Plan

1. **Writer-side merge detection** (option (a) from README). The
   reasoning lives in G-note §1.2: latency-zero, atomicity
   trivial under PG row-level locks, formula in one shared place.

### Emerged

2. **Schema-addition need for deterministic open/close.** The
   README didn't anticipate that `open`/`close` couldn't be
   merged deterministically across sources without timestamp
   columns. Surfaced in §2: added `first_trade_at` /
   `last_trade_at` (TIMESTAMPTZ) to enable chronologically-
   correct merge.
3. **`sources_seen JSONB` instead of relying on
   `current_prices.sources`.** Initially leaned toward "rely on
   `current_prices.sources`" (G-note §4.2 option b). On
   second pass realised the Current Price Updater needs
   per-1m-row breakdown to reconstruct the 24h `sources` JSONB.
   So per-row storage IS required after all — adopted §4.3
   option (a).
4. **ADR 0004 over an addendum to ADR 0003.** Considered
   amending the accepted ADR 0003 with a §"Open/close
   determinism" section. Chose a new ADR (0004) instead to
   preserve 0003 as a closed record and to give the multi-
   source merge schema work its own discoverable home.
5. **No spawn task for implementation.** Implementation of the
   merge formula distributes across task 0012 (backfill +
   shared library + Prices Ledger Processor) and any future
   per-source live writers (Soroswap consumer, Aquarius
   consumer). No single "implement 0025" task to spawn —
   the spec + ADR are the deliverables, each writer's own task
   carries forward the contract.

## Issues Encountered

None substantive. Followed the same activation/archive pattern as
0023/0024. The "edit before rename" git issue from 0024 was
avoided this time by `git mv` first then `Edit` at the new path.

## Future Work

Three open items in G-note §8, none blocking:

- **Current Price Updater spec update** to read `sources_seen`
  (lives in design doc §5.5). Lands when the live-writer
  implementation tasks materialise post-0012.
- **Per-source rolling state cache** — not adopted (G-note
  §4.3 option b-ii). Revisit only if `sources_seen` JSONB
  becomes too heavy at scale.
- **Backfill spec edit for `first_trade_at`/`last_trade_at`** —
  task 0022's archived decode-and-bucket spec §5 needs a
  one-line correction. Edit lands in task 0012's worklog when
  implementation begins, not retroactively in the archived spec.
