---
id: "0350"
title: "Phoenix: withdraw_liquidity events are counted as failed swap dispatches, and 4 of 1,469 swaps in 2026-10-01 → 11-05 have no candle"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0242", "0099", "0101"]
tags: [layer-indexing, phoenix, amm, priority-low, effort-small, ingest, observability]
links:
  - "../../../packages/events-backfill/src/main.rs"
history:
  - date: 2026-11-18
    status: backlog
    who: okarcz
    note: >
      Spawned from 0242's reprice (2026-10-01 → 2026-11-05). Two Phoenix
      observations whose cause is not established. Neither is caused by the
      reprice, and neither touches a 0242 asset.
---

# Phoenix: withdraw_liquidity counted as failed swaps, and 4 swaps without a candle

## Summary

During [[0242]]'s reprice, `events-backfill` logged `amm dispatch error … unexpected
topic shape in row at event_index 4` on two Phoenix pools and reported
`phoenix … (swaps failed dispatch: 8)`. Separately, BE holds 1,469 Phoenix swaps in
that range, and only 1,465 became candles. Find what each is. Fix the counter if it
miscounts, and price the 4 swaps if they are real trades.

## Context

Measured 2026-11-18, read-only (`dev_read`), ledgers 64,705,404 → 65,318,468
(2026-10-01 00:00:02 → 2026-11-05 11:32:59):

- **The dispatch errors are on two Phoenix pools**, both in `pool_registry` as
  `phoenix`: `CBCZGGNOEUZG4CAAE7TGTQQHETZMKUT4OIPFHHPKEUX46U4KXBBZ3GLH` (most) and
  `CCPPPTDWJIWXQUQ2CN64S5JYQ7GYWVZIT7YWUUTH75HKIZX53Z2CE3XI` (one).
- **The failing rows look like liquidity withdrawals, not swaps.** On `CBCZGGNO…`
  the pool emitted 704 `swap` events (88 eight-event groups, every group with its
  `sender`) and 48 `withdraw_liquidity` events (12 groups of `sender`,
  `shares_amount`, `return_amount_a`, `return_amount_b`). The withdraw groups'
  `sender` sits at `event_index 4`, the index in the error. If these are the 8
  "failed dispatches", the summary counts a non-swap event as a failed swap: a
  misleading counter, not lost trades.
- **4 swaps have no candle.** Phoenix `swap`/`sender` events: 1,469. Dry-run ticks:
  1,465. `price_ohlcv_1m` trades: 1,465. Live and the reprice agree, so the gap
  predates the reprice. These 4 are not necessarily the 8 rows above. Possible
  causes, none checked: dust, zero amounts, an unresolved leg, or a swap group
  split across operations.
- [[0101]]'s July check found Phoenix exact (raw = candles on every day 07-06 →
  07-17), and the whole-era sweep found 1–2 trade differences on 2026-09-08,
  09-30, 10-01 and 10-05. 10-01 and 10-05 fall inside this range.

## Implementation Plan

1. Find the 4 swaps: per pool and day, compare the `sender` count in
   `soroban_events` with candle `trade_count`, then read the swap groups of the
   days that differ.
2. Run `events-backfill --dry-run --verbose` on one ledger holding a
   `withdraw_liquidity` group of `CBCZGGNO…` (for example 64,787,341). Confirm it
   is what logs the error and raises `swaps failed dispatch`.
3. If so, have the Phoenix extractor skip `withdraw_liquidity` (and
   `provide_liquidity`) without an error, and count only real swap groups as
   failed. Check that live does the same.
4. If the 4 swaps are real trades, fix their cause and reprice their minutes
   (`events-backfill`, then the pre-roll, as in 0242).

## Acceptance Criteria

- [ ] The 8 dispatch errors are explained. A non-swap event no longer counts as
      a failed swap dispatch.
- [ ] The 4 unpriced swaps are identified, with the cause recorded, and either
      priced or recorded as correctly excluded (e.g. dust).
- [ ] Phoenix raw swaps equal candle trades for 2026-10-01 → 11-05, or the
      difference is explained.
