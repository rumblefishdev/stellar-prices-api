---
id: "0246"
title: "/ohlcv's USDC peg series resolves the rate at the bucket START, unbounded — it disagrees with price_usd_series"
type: BUG
status: active
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
  - date: 2026-09-01
    status: active
    who: okarcz
    note: >
      Activated. [[0168]] deployed and closed the same day, so the reference
      side of the disagreement is now live on prod and this is the last surface
      where the old resolution rule survives. Taken now rather than [[0126]],
      which is the other candidate: 0126's remaining substance is CORS in
      api-gateway-stack.ts, the file PR #268 rewrites, and the pattern it would
      build on (portalWebOrigin, addCorsPreflight) exists only on that unmerged
      branch. 0246 touches queries_ch.rs and views.sql, which nobody else is in.
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

- [x] For a bucket with observations, `/ohlcv` and `price_usd_series` at the same
      grain publish the **same** `close` for canonical USDC.
      `ohlcv_agrees_with_price_usd_series_on_the_same_bucket`.
- [x] An oracle gap longer than the staleness window renders as `method = 'peg'`
      on both surfaces, never as a forward-filled `'oracle'`.
      `ohlcv_does_not_forward_fill_a_stale_rate_into_later_buckets`.
- [x] A test pins the agreement across the two surfaces directly, rather than
      each surface pinning its own rule in isolation — that isolation is why the
      divergence went unnoticed. The cross-surface test queries
      `price_usd_series_1h` in the same scratch database and compares
      **surface against surface**, not against literals.
- [x] No `SETTINGS` clause; verified as a `readonly = 1` user —
      `ohlcv_peg_series_answers_for_a_readonly_user` still passes against the new
      query shape.
- [ ] ⏳ **Live on prod.** `/ohlcv` is served by the api-handler Lambda, so this
      needs a **Compute deploy** — see the note at the end.

## Out of scope

- The enrichment peg tier's flat `$1` in `close_usd` itself — a write-path change,
  tracked in [[0168]]'s "Known adjacent gap".


---

# Implementation Notes — 2026-09-01

## What changed

`packages/prices-api/src/assets/queries_ch.rs` only, plus its tests. The ASOF
became a bucket-scoped equi-join, in the shape [[0168]] landed on:

```sql
-- was: ASOF LEFT JOIN (…) AS r ON b.k = r.k AND r.rts <= b.bkt
LEFT JOIN (
    SELECT toStartOfInterval(timestamp, INTERVAL <grain>) AS rbkt,
           argMax(usd_rate, timestamp)                    AS rate,
           CAST(argMax(method, timestamp) AS String)      AS meth
    FROM usd_rate FINAL
    WHERE asset_kind = 'credit' AND asset_code = 'USDC'
      AND issuer_address = ? AND contract_address = ''
      AND method = 'oracle'
    GROUP BY rbkt
) AS r ON r.rbkt = b.bkt
```

The staleness window becomes exactly one bucket width for free: an observation
either falls inside the bucket or the bucket falls back to the labelled peg.
There is no window over which a stale reading can be dressed as a measurement.

`Granularity::interval_sql()` is new — the `toStartOfInterval` argument per
grain, deliberately identical to the intervals `rollups.sql` uses to BUILD these
buckets. A mismatch there would resolve every rate against the wrong bucket
silently rather than failing.

## Design Decisions

### From Plan

1. **`argMax` in the bucket, not a bounded ASOF.** The task suggested it and it
   is right: ClickHouse's ASOF takes exactly one inequality, so a staleness bound
   would have needed a second predicate applied after the join. Scoping to the
   bucket expresses the same rule with no extra constant.
2. **`o = h = l = c` stays.** The task offered keeping bucket-start for the
   `open`. Rejected: these are flat synthetic candles by construction, and a real
   `open` with a synthetic `high`/`low` would imply a range that was never
   traded. Documented rather than half-implemented.

### Emerged

3. **Only `method = 'oracle'` is accepted now.** The old query ranked
   `oracle > pivot > pivot2 > …` via `argMin(rate, pref)` and rendered a pivot
   row as `'traded'`. `price_usd_series` and `current.sql` both take measurements
   or nothing, so this surface was the only one that would have answered from a
   [[0154]] pivot — **a second way for the same two surfaces to disagree, on a
   bucket that HAS observations**, which AC 1 forbids. Today it is a no-op:
   nothing writes a non-`oracle` row for canonical USDC. Recorded because it
   narrows a shipped endpoint's inputs, and because 0154 must now add pivots to
   every read surface in one change rather than inheriting one here silently.
4. **`1M` has a bucket-attribution edge, stated rather than solved.** The `1M`
   rollup reads from `1w`, so a week starting in August but running into
   September files its September days under August, while an observation on
   1 September floors to September here. Both are defensible, and the difference
   is invisible to AC 1, which compares against `price_usd_series{,_1h}` — daily
   and hourly only. Noted in `interval_sql`'s doc comment.

## Issues Encountered

- 🔑 **All 26 existing tests passed against BOTH rules**, before and after. That
  is not reassurance, it is the finding: every fixture in `ohlcv_it.rs` placed
  its observations exactly on a bucket boundary (10:00, 11:00), where resolving
  at the start and resolving at the end pick the same row. The new fixture
  (`seed_0246`) puts them at 10:05 and 10:55 — strictly inside the bucket — which
  is the smallest change that makes the two rules distinguishable at all.
- **Both new tests were verified to FAIL against the old query** by reverting
  `queries_ch.rs` and re-running, rather than assumed to be regressions. The
  cross-surface one failed with `/ohlcv published 1 but price_usd_series_1h
  published 1.0007` — the divergence, in the assertion message.
- The forward-fill test originally asserted the measured bucket first, so a
  regression tripped on the sanity check instead of on the defect the test is
  named for. Reordered so the failure names the forward-fill.

## Test Results

| suite | result |
|---|---|
| `prices-api` `ohlcv_it` (`--ignored`) | **28 passed, 2 new** (was 26) |
| `prices-api` full (`--include-ignored`) | 169 lib + every IT suite, all green |
| `cargo clippy -p prices-api --all-targets` | clean |

## ⏳ Remaining — the deploy

`/ohlcv` runs in the api-handler Lambda, so prod still serves the old resolution
until a **Compute deploy**. ⚠️ That deploy is not ours alone to make: `develop`
carries Adam's merged work, so `make deploy-production-compute` ships his
changes too — the same coupling [[0244]] is waiting on. Ride his next Compute
deploy rather than triggering one, and verify with the runbook query below.

```sql
-- after the deploy, for a bucket that HAS observations, these must agree
SELECT bucket, close_usd, method FROM prices.price_usd_series_1h
WHERE asset_code = 'USDC' AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
ORDER BY bucket DESC LIMIT 5;
```

against `GET /v1/assets/USDC:GA5ZSE…/ohlcv?granularity=1h&base_currency=USD`
(⚠️ `apiKeyRequired`), same buckets.
