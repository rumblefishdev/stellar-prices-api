---
id: "0264"
title: "soroban_amm.earliest_data_available claims 2024-02-20 while the first AMM candle is 2024-03-08 — 17 days of coverage that exists at no granularity"
type: BUG
status: completed
related_adr: []
related_tasks: ["0127", "0263", "0128", "0106"]
tags: [layer-backend, layer-api, priority-medium, effort-small, milestone-M2, backfill, api, data-correctness]
milestone: 2
links:
  - "../../../packages/sdex-backfill/src/sink.rs"
  - "../../../packages/prices-api/src/backfill/queries_ch.rs"
history:
  - date: 2026-09-04
    status: backlog
    who: okarcz
    note: >
      Found by [[0127]]'s AC 2 pass. The SDEX stream's stored watermark
      reconciled against real rows on the minute; the AMM stream's does not.
      Measured, not inferred — `price_ohlcv_1m` was queried directly for
      non-SDEX rows before 2024-03-08 and returned **nothing**. ⚠️ That evidence
      was later found unsound (2026-09-08): `_1m` holds no AMM rows before
      2026-07-01 at all, so the query could not have returned otherwise. The
      **conclusion survives** — re-measured against `_1d` and `_1h`, which hold
      the durable AMM history: zero AMM rows before 2024-03-08 at every
      granularity, and the overclaim is exactly 17 days.
  - date: 2026-09-08
    status: active
    who: okarcz
    note: >
      Promoted to active for the Milestone 2 pre-submission pass ([[0128]]).
      Blocks nothing in Tranche 2 AC 5 — that criterion is graded on the SDEX
      watermark, which is correct and independently corroborated — but this
      value sits in the same reviewer-facing payload. ⚠️ The correction must
      bypass `merge_min`, which only ever moves the value *older*, so a normal
      run cannot write it and a careless fix gets silently undone. Ships in
      one `sdex-backfill` release with [[0263]].
  - date: 2026-09-08
    status: completed
    who: okarcz
    note: >
      **CLOSED — writer merged (PR #295), data corrected and verified on
      production 2026-09-08.** Four of five criteria met; AC 4 deferred to
      [[0272]] deliberately, not silently. 🔑 The cause was a **true observation
      of the wrong population**, not the ledger-range assumption this task
      hypothesised: a shared minute window was stamped on both rows, so the AMM
      stream inherited the earliest SDEX minute at the activation boundary —
      2024-02-20 17:00, where production holds 141 SDEX candles and zero AMM
      ones. Window now split per stream at the write seam. 30 tests pass.
      ⚠️ This task's own `_1m` evidence was unsound and its AC unsatisfiable;
      both corrected in place, conclusion re-measured against `_1d`/`_1h` and
      upheld at exactly 17 days. Both streams now reconcile to 0 days
      overclaimed. Milestone 2 evidence §5 AC 5 updated from "deferred" to
      corrected.
---

# The AMM stream's `earliest_data_available` is 17 days early

## Summary

`GET /v1/backfill/status` publishes:

```json
"soroban_amm": { "earliest_data_available": "2024-02-20T17:00:00Z" }
```

The earliest AMM candle that actually exists is **2024-03-08** (Soroswap; then
Phoenix 2024-03-21, Aquarius 2024-04-18). **The endpoint claims 17 days of
coverage that is not there.**

This is not a rollup defect. `price_ohlcv_1m` — the finest granularity, the one
everything else is derived from — was queried directly:

```sql
SELECT source, min(timestamp), count()
FROM prices.price_ohlcv_1m
WHERE source != 'sdex' AND timestamp < '2024-03-08'
GROUP BY source
```

**Zero rows.** The data does not exist at any granularity, so the stored value
is simply wrong.

## Context

[[0127]]'s AC 1 ran the same reconciliation on the **SDEX** stream and it
passed cleanly: stored `2015-11-18 03:47:00`, actual `min(timestamp)`
`2015-11-18`, oldest active partition `201511`. The trap that AC checked for did
not fire there. It fires here, on the other stream, and nothing was watching.

⚠️ **It cannot self-correct.** `sink.rs` merges the column with `merge_min`
(*"Monotonic window: never narrow what a prior run already recorded"*), so the
value only ever moves **older**. A wrong-too-early value is permanent until
something writes over it deliberately.

⚠️ **Severity is bounded but real.** The AMM stream is not what Tranche 2 AC 5
is graded on — that criterion names `sdex.earliest_data_available`, which is
correct. But both values sit in the same reviewer-facing payload, and one of
them overstates. A reviewer who checks the one that is not being graded finds a
claim the data does not support.

## How it probably happened

Not established — this is where to start, not a conclusion.

- `2024-02-20 17:00` is suspiciously close to Soroban mainnet activation. A seed
  or a constant derived from the activation boundary, rather than from an
  observed candle, would produce exactly this shape.
- The AMM stream's `start_ledger` is `50457424` — the activation ledger. If any
  writer derived the window from the *ledger range* it intended to cover rather
  than from `PartitionStats::earliest_minute` (the observed candle), the value
  would be an intent, not an observation.
- ⚠️ The stream has been `status: "running"` with `last_push_at` of
  **2026-07-14** — seven weeks stale as of 2026-09-04. Whatever wrote it last
  has not run since.

## Implementation

- Establish the writer path that produced `2024-02-20 17:00`. Confirm whether
  it came from an observed minute or from a ledger-range assumption.
- Correct the stored value to the observed first AMM candle. ⚠️ `merge_min`
  will **not** accept a later value through the normal path — this needs a
  deliberate write, and it should be done in a way that a future run cannot
  silently undo.
- Add the reconciliation as a check rather than a one-off fix: a stored
  `earliest_data_available` that precedes `min(timestamp)` for that stream's
  sources is a defect on either stream, and nothing currently detects it.
  [[0243]] is the nearest existing freshness watcher; decide whether this
  belongs with it or stands alone.
- Re-read `/backfill/status` afterwards and confirm both streams reconcile.

## Acceptance Criteria

- [x] The writer path that produced `2024-02-20 17:00` is identified and
      recorded — observation or assumption, said plainly.
- [x] `soroban_amm.earliest_data_available` matches the earliest actual AMM
      candle, verified against `price_ohlcv_1d` / `_1h`. ⚠️ **CORRECTED
      2026-09-08 — this criterion said "against `price_ohlcv_1m` and not only
      against `_1d`", which is backwards and unsatisfiable.** `_1m` holds no
      non-SDEX rows before 2026-07-01: the AMM backfill pre-rolled into the
      coarse tables and never populated it, while SDEX history in the same table
      reaches 201511. A `_1m` query for a 2024 date returns zero whether or not
      the data ever existed, so it cannot support the conclusion it was used for.
      See [[amm-history-is-not-in-price-ohlcv-1m]].
- [x] The correction survives a subsequent backfill run — `merge_min` does not
      re-widen it to the wrong value. ⚠️ **Met by construction, not yet
      exercised.** No backfill has run since. A future `Combined` run over
      `[activation, live floor]` observes `amm_earliest = 2024-03-08`, and
      `merge_min` of that against the corrected stored value is a no-op. The
      pre-fix path — a shared window carrying the earliest SDEX minute — no
      longer exists, so there is nothing left to re-widen it.
- [ ] ⏸️ **DEFERRED to [[0272]]** — something detects the general case: a stored
      watermark that precedes the data behind it, on either stream. Deferred
      rather than met, deliberately: the end-of-run check first proposed is
      **vacuous** after this task's writer fix, because the claim a run writes is
      the earliest minute that run landed, so comparing the two compares a value
      to itself. The useful check is stored-claim vs candle tables on a timer,
      which is 0272's whole scope. A one-off manual reconciliation when the data
      correction lands confirms *this* row without waiting on it.
- [x] [[0127]] and [[0128]] are told the AMM figure is trustworthy, or told
      plainly that it is not and excluded from the package.


## Implementation Notes

Shipped as PR #295 (stacked on #294), merged 2026-09-08. The **writer** fix
deployed with the `sdex-backfill` sources; the **data** correction was a separate
operator-run statement, applied and verified the same day.

- `ingest.rs` — `PartitionStats` carries `sdex_earliest/latest` and
  `amm_earliest/latest` instead of one mixed pair. `note_candles` takes the same
  `source` string handed to `Sink::write_candles`, so the classification cannot
  drift from what lands in the `source` column.
- `run.rs` — run-level totals merge all four.
- `progress.rs` — each `backfill_progress` row is stamped with its own stream's
  window. A run landing no AMM candles leaves that window `None` rather than
  borrowing SDEX's; `merge_min` reads `None` as "no claim".

Also carried [[0176]]'s `paused` fix, since it is the same function and release.

30 tests pass. Six integration-test `Observed` constructions updated to supply
both windows, preserving their semantics; three regression tests added.

**Production correction**, run by the operator after the release:

```sql
INSERT INTO prices.backfill_progress
SELECT task_name, start_ledger, target_ledger, current_ledger,
       'paused' AS status, last_push_at,
       toDateTime('2024-03-08 19:00:00') AS earliest_data_available,
       newest_data_available, started_at, completed_at, now() AS updated_at
FROM prices.backfill_progress FINAL WHERE task_name = 'soroban_amm';
```

Verified: both streams reconcile to **0 days overclaimed** against
`min(timestamp)` in `price_ohlcv_1d`.

## Issues Encountered

- 🔴 **This task's own evidence was unsound, and the AC was unsatisfiable.** It
  argued from a `price_ohlcv_1m` query returning zero rows before 2024-03-08.
  `_1m` holds no non-SDEX rows before 2026-07-01 at all — the AMM backfill
  pre-rolled into the coarse tables and never populated it — so that query
  returns zero regardless. The **conclusion survived** re-measurement against
  `_1d` and `_1h`, but the reasoning did not. Both the AC and the founding
  history note were corrected in place. See
  [[amm-history-is-not-in-price-ohlcv-1m]].
- **A retention hypothesis was floated and disproved.** `_1m` carries a nominal
  7-day retention, which looked like the explanation until SDEX rows from 201511
  turned up in the same table. Nothing had pruned it.

## Design Decisions

### From Plan

1. **Correct the writer, then the data, in that order.** `merge_min` never moves
   a stored watermark later, so a correction applied before the writer fix would
   be re-stamped by the next run.

### Emerged

2. **Per-stream windows rather than an AMM-only addition.** Adding
   `amm_earliest` alone and leaving the mixed field for the SDEX row would have
   worked — SDEX predates AMM in every Soroban-era window, so the SDEX row was
   incidentally correct. Split both instead: the ambiguous field *was* the
   defect, and leaving one in place invites the same misuse.
3. **Classification taken from the write call site, not the candle.**
   `OhlcvCandle` carries no `source`; only the call site knows. Threading it
   through `note_candles` keeps the window and the `source` column derived from
   one value.
4. **AC 4 deferred to [[0272]] rather than met.** The end-of-run check first
   proposed is vacuous after this fix — the claim a run writes *is* the earliest
   minute it landed, so comparing them compares a value to itself. The useful
   check is stored-claim vs candle tables on a timer.

## Future Work

- [[0272]] — reconcile `backfill_progress` claims against the rows behind them,
  on a schedule. Carries this task's AC 4.

## Notes

- 🔑 **The general lesson, and the reason this is a task rather than a one-line
  fix**: `backfill_progress` holds *claims*, and until [[0127]] nobody had
  compared them to the rows. Two columns are now known to assert more than the
  data supports — this one, and `current_ledger`, which asserts a floor rather
  than proven contiguous coverage ([[0263]]). Same class, different columns,
  same fix shape: reconcile the claim against the rows, on a schedule.
- The SDEX side of the same payload **is** correct and was independently
  corroborated in [[0127]] against `min(timestamp)` and the partition census.
  Do not let this finding cast doubt on it.
