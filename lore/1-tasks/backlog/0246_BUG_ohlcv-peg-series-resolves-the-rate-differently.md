---
id: "0246"
title: "/ohlcv's USDC peg series resolves the rate at the bucket START, unbounded — it disagrees with price_usd_series"
type: BUG
status: backlog
related_adr: ["0011"]
related_tasks: ["0168", "0170", "0167"]
tags: ["priority-medium", "effort-small", "clickhouse", "read-surface", "data-correctness", "api"]
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-31
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0168]] future work. Found while implementing 0168: two of
      our surfaces read prices.usd_rate for the same identity and the same
      bucket and reach different values, by two independent mechanisms. Not
      fixed in 0168 because it changes a shipped endpoint's published values.
---

# `/ohlcv`'s peg series and `price_usd_series` disagree about the same bucket

## Summary

`ohlcv_peg_series` ([[0170]], `queries_ch.rs`) and `price_usd_series*` ([[0168]],
`views.sql`) both synthesise canonical USDC from `prices.usd_rate`. They resolve
the rate by **different rules**, so the same identity in the same bucket reads
differently depending on which surface a consumer asks.

## Context

`/ohlcv` ASOFs at the bucket's **start** with **no staleness bound**:

```sql
ASOF LEFT JOIN ( … ) AS r ON b.k = r.k AND r.rts <= b.bkt
```

`price_usd_series*` takes the **last observation inside the bucket**, and falls
back to `$1`/`peg` when the bucket has none.

Two consequences, independent of each other:

1. **Normal operation.** `/ohlcv`'s daily "close" is the *previous* day's last
   reading; the view's is the day's own. They differ by the intraday drift —
   small (~1e-4) but they are different rows, and a consumer diffing
   `/ohlcv?granularity=1d` against `price_usd_series` sees it.
2. **After an oracle outage.** `/ohlcv` forward-fills the last known rate
   **indefinitely**, still labelled `method = 'oracle'`. A dead oracle's final
   reading would be served as a measurement for as long as the outage lasts.
   The view falls back to `$1`/`peg` and says so.

`init.sql`'s 0167 block names the rule for a bucket-grained consumer: *"T is the
BUCKET'S END — i.e. the bucket's closing rate"*, and gives the reason — it is the
only resolution under which a daily close equals the last hourly close of that
day, i.e. the only one that composes across the six grains.

⚠️ `/ohlcv` is not simply wrong. It sets `o = h = l = c = rate` for a flat
synthetic candle, and bucket-start resolution is defensible for an **open**. What
is not defensible is the same value being the **close**, and the unbounded
forward-fill is a defect on any reading.

## Implementation

- Resolve at the bucket's end for `c` (and `h`/`l`), keeping the bucket's start
  for `o` if a genuine open is wanted — or collapse to the view's rule and
  document that these are flat candles.
- Bound the ASOF by a staleness window so an outage falls back to `peg` rather
  than forward-filling `oracle` forever. One bucket width is what [[0168]] uses
  and it needs no new constant.
- ⚠️ Do NOT add a `SETTINGS` clause. `prices_reader` is read-only and refuses
  one before a row runs (code 164) — that is exactly how this endpoint 500'd on
  2026-08-27.
- The cheapest shape is probably the one [[0168]] landed on: collapse `usd_rate`
  to one row per bucket with `argMax` and join on the bucket, no ASOF at all.

## Acceptance Criteria

- [ ] For a bucket with observations, `/ohlcv` and `price_usd_series` at the same
      grain publish the **same** `close` for canonical USDC.
- [ ] An oracle gap longer than the staleness window renders as `method = 'peg'`
      on both surfaces, never as a forward-filled `'oracle'`.
- [ ] A test pins the agreement across the two surfaces directly, rather than
      each surface pinning its own rule in isolation — that isolation is why the
      divergence went unnoticed.
- [ ] No `SETTINGS` clause; verified as a `readonly = 1` user
      (`ohlcv_peg_series_answers_for_a_readonly_user` is the existing pattern).

## Out of scope

- The enrichment peg tier's flat `$1` in `close_usd` itself — a write-path change,
  tracked in [[0168]]'s "Known adjacent gap".
