---
id: "0264"
title: "soroban_amm.earliest_data_available claims 2024-02-20 while the first AMM candle is 2024-03-08 — 17 days of coverage that exists at no granularity"
type: BUG
status: active
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

- [ ] The writer path that produced `2024-02-20 17:00` is identified and
      recorded — observation or assumption, said plainly.
- [ ] `soroban_amm.earliest_data_available` matches the earliest actual AMM
      candle, verified against `price_ohlcv_1d` / `_1h`. ⚠️ **CORRECTED
      2026-09-08 — this criterion said "against `price_ohlcv_1m` and not only
      against `_1d`", which is backwards and unsatisfiable.** `_1m` holds no
      non-SDEX rows before 2026-07-01: the AMM backfill pre-rolled into the
      coarse tables and never populated it, while SDEX history in the same table
      reaches 201511. A `_1m` query for a 2024 date returns zero whether or not
      the data ever existed, so it cannot support the conclusion it was used for.
      See [[amm-history-is-not-in-price-ohlcv-1m]].
- [ ] The correction survives a subsequent backfill run — `merge_min` does not
      re-widen it to the wrong value.
- [ ] ⏸️ **DEFERRED to [[0272]]** — something detects the general case: a stored
      watermark that precedes the data behind it, on either stream. Deferred
      rather than met, deliberately: the end-of-run check first proposed is
      **vacuous** after this task's writer fix, because the claim a run writes is
      the earliest minute that run landed, so comparing the two compares a value
      to itself. The useful check is stored-claim vs candle tables on a timer,
      which is 0272's whole scope. A one-off manual reconciliation when the data
      correction lands confirms *this* row without waiting on it.
- [ ] [[0127]] and [[0128]] are told the AMM figure is trustworthy, or told
      plainly that it is not and excluded from the package.

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
