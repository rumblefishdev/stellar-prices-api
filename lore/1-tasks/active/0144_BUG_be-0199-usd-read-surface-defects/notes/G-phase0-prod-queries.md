---
title: "Phase 0 — blast-radius queries for prod (ch-prod-01)"
type: generation
status: developing
spawns: []
tags: [clickhouse, prod, measurement, read-only]
links:
  - "../../../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-05
    status: seed
    who: okarcz
    note: "Query pack for 0144 phase 0. A-D lifted from the task README and sharpened; E drafted here for the first time."
---

# Phase 0 — blast-radius queries for prod

**All read-only.** Run on `ch-prod-01`. The *mechanisms* are already settled in
[`repro/`](../repro/README.md) — these size the **blast radius**, which is what
calibrates the fix priority, the [[0147]] gate threshold and the BE reply.

Results go in the table at the bottom, then into the task README.

## Before you start

**Two cost controls apply to almost everything here.**

`SETTINGS do_not_merge_across_partitions_select_final = 1` is on every `FINAL`
query below. It is **safe here specifically**: the RMT sort key is
`(asset_id, quote_asset_id, source, timestamp)` and the partition key is
`toYYYYMM(timestamp)`, so two rows that dedup against each other share a
`timestamp` and therefore always live in the *same* partition. Cross-partition
merging at read time can never change the result, and skipping it is a large
saving on the wide-history queries.

**The tables are partitioned by month**, so every time-bounded predicate prunes
partitions. The unbounded ones (E) do not — run those per tier and stop if one
misbehaves rather than firing all six at once.

**Sanity check first** — confirms the pin and that nothing else is hammering the
cluster:

```sql
SELECT version() AS ch_version, now() AS server_now,
       (SELECT count() FROM system.processes WHERE query NOT LIKE '%system.processes%') AS running_queries;
```

Expect `26.3.10.60`. If `running_queries` is high, the timings below will be
noise — the row counts stay valid, the wall-clock does not.

---

## A — is the XLM tip un-enriched, and what quote is it? (finding 1)

Confirms BE's headline complaint at the source, and tells us whether the newest
XLM candle is *pending* enrichment or **permanently unpriceable** (exotic quote,
no oracle — `ch_enrich.rs:31-32`). Those are different answers to BE.

```sql
SELECT
    p.timestamp,
    p.source,
    q.asset_code                                    AS quote,
    q.issuer_address                                AS quote_issuer,
    p.close,
    p.close_usd,
    p.volume_base,
    if(p.close_usd > 0, 'priced', 'UNPRICED')       AS state
FROM prices.price_ohlcv_1m AS p FINAL
LEFT JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
WHERE p.asset_id = (
        SELECT asset_id FROM prices.assets FINAL
        WHERE asset_code = 'XLM' AND issuer_address = '' AND contract_address = ''
        LIMIT 1)
  AND p.timestamp >= now() - INTERVAL 2 HOUR
ORDER BY p.timestamp DESC
LIMIT 20
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

**Read it as:** newest rows `UNPRICED`, older rows priced → the tip lag is
confirmed. Note the `quote` of the newest row. If it is **not** USDC/USDT/XLM,
that row is on the permanent floor and no amount of waiting prices it — which is
the concrete example the BE reply needs.

**A2 — how much of every hour XLM spends on the 0 sentinel.** This is the number
that turns "chronic" from an assertion into a measurement, and it is the single
most useful figure for [[0135]]'s contract call:

```sql
SELECT
    toStartOfHour(timestamp)                                        AS hour,
    count()                                                         AS candles,
    countIf(close_usd = 0)                                          AS unpriced,
    round(countIf(close_usd = 0) / count(), 4)                      AS unpriced_share,
    argMax(close_usd, timestamp)                                    AS published_now,
    argMaxIf(close_usd, timestamp, close_usd > 0)                   AS if_guarded
FROM prices.price_ohlcv_1m FINAL
WHERE asset_id = (
        SELECT asset_id FROM prices.assets FINAL
        WHERE asset_code = 'XLM' AND issuer_address = '' AND contract_address = ''
        LIMIT 1)
  AND timestamp >= now() - INTERVAL 24 HOUR
GROUP BY hour ORDER BY hour DESC
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

`published_now` vs `if_guarded`, hour by hour, **is finding 1** — and the count
of hours where `published_now = 0` is how often XLM publishes nothing.

---

## B — is half of BE's scan our fan-out? (finding 2a)

### B1 — the ratio BE's "twice" should equal

```sql
SELECT
    count()                                                            AS joined_rows,
    countDistinct(p.asset_id, p.timestamp, p.source, p.quote_asset_id) AS distinct_candles,
    round(count() / countDistinct(p.asset_id, p.timestamp, p.source, p.quote_asset_id), 3) AS fanout_ratio
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.timestamp >= now() - INTERVAL 104 WEEK
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

A ratio near **2.0** confirms BE's "scans every asset's daily candles twice" is
[[0139]]'s identity fan-out, not their query. This is the heaviest query in the
pack — it is deliberately the same shape and window BE measured at 70.7M read
rows / 4.6 s / 2.1 GiB, so **compare the reported read rows against their
number**; that comparison is itself a result worth recording.

### B2 — how many duplicate `asset_id`s actually carry candles

The task README notes this is the part still missing: 0139 measured 3,275
`asset_id`s with two or more natural identities, but the *consequence* here
depends on how many of those carry daily candles.

```sql
WITH dup AS (
    SELECT asset_id, count() AS identities
    FROM (SELECT DISTINCT asset_id, asset_code, issuer_address, contract_address
          FROM prices.assets FINAL)
    GROUP BY asset_id HAVING identities > 1
)
SELECT
    count()                                     AS dup_asset_ids,
    sum(identities)                             AS total_identities,
    countIf(c.candles > 0)                      AS dup_ids_with_candles,
    sum(c.candles)                              AS candle_rows_affected
FROM dup
LEFT JOIN (
    SELECT asset_id, count() AS candles
    FROM prices.price_ohlcv_1d FINAL
    WHERE timestamp >= now() - INTERVAL 104 WEEK
    GROUP BY asset_id
) AS c USING (asset_id)
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

`dup_ids_with_candles` is the real exposure: every one of those publishes a
price series under an identity that never traded it (TEST D's `DUPB`).

---

## C — the population shift, and the threshold [[0147]] needs (finding 3i)

### C1 — BE's exact asset

The README elides the issuer as `GARDNV3Q…`; resolving by `asset_code` avoids
transcribing it and prints the address for the record. (BE's own audit docs give
`GARDNV3Q7YGT4AKSDF25LT32YSCCW4EV22Y2TV3I2PU2MMXJTEDL5T55`, ultracapital.xyz —
**confirm it matches what this returns** before quoting it back to them.)

```sql
SELECT
    a.issuer_address                                                      AS issuer,
    p.timestamp                                                           AS bucket,
    count()                                                               AS rows_total,
    countIf(p.close_usd > 0)                                              AS rows_priced,
    sum(p.volume_base)                                                    AS vol_total,
    sumIf(p.volume_base, p.close_usd > 0)                                 AS vol_priced,
    round(sumIf(p.volume_base, p.close_usd > 0) / nullIf(sum(p.volume_base), 0), 6) AS priced_volume_share,
    -- what the shipped view publishes vs what full weighting would give
    sumIf(toFloat64(p.close_usd) * toFloat64(p.volume_base), p.close_usd > 0)
        / nullIf(sumIf(toFloat64(p.volume_base), p.close_usd > 0), 0)     AS close_usd_as_shipped,
    sum(toFloat64(p.close_usd) * toFloat64(p.volume_base))
        / nullIf(sum(toFloat64(p.volume_base)), 0)                        AS close_usd_if_unfiltered
FROM prices.price_ohlcv_1h AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE a.asset_code = 'yXLM'
  AND p.timestamp >= now() - INTERVAL 24 HOUR
GROUP BY issuer, bucket
ORDER BY bucket DESC
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

The last two columns are BE's two options side by side, per bucket. Expect
`close_usd_if_unfiltered` to be **catastrophically low** wherever
`priced_volume_share < 1` — that is TEST B's 0.000023 reproducing on prod, and
it is the evidence for telling BE their option B is worse than the status quo.

### C2 — the distribution the threshold must be picked from

The acceptance criterion says pick X from the real distribution, not from taste.
One asset cannot tell us that.

Aggregate per bucket first, then histogram the result:

```sql
SELECT
    priced_share_rounded,
    count()                                  AS buckets,
    round(count() / sum(count()) OVER (), 4) AS share_of_buckets
FROM (
    SELECT
        asset_id,
        timestamp,
        round(sumIf(volume_base, close_usd > 0) / nullIf(sum(volume_base), 0), 2) AS priced_share_rounded
    FROM prices.price_ohlcv_1h FINAL
    WHERE timestamp >= now() - INTERVAL 48 HOUR
    GROUP BY asset_id, timestamp
)
GROUP BY priced_share_rounded
ORDER BY priced_share_rounded
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

**Read it as:** a healthy world is bimodal — a big pile at `1.00` (fully priced)
and a pile at `0.00`/`null` (fully unpriceable). **The mass in between is the
population the gate has to rule on**, and its size decides whether X = 0.5, 0.9
or 0.99. If the middle is fat, a high threshold suppresses a lot of real
buckets; tell BE that trade-off explicitly.

---

## D — how much of the coarse estate is zeroed (finding 3ii)

All six tiers, not the two in the README.

```sql
SELECT 'price_ohlcv_15m' AS tbl, countIf(close_usd = 0 AND close > 0) AS zeroed, count() AS total,
       round(countIf(close_usd = 0 AND close > 0) / count(), 4) AS share
FROM prices.price_ohlcv_15m FINAL WHERE timestamp >= now() - INTERVAL 48 HOUR
UNION ALL SELECT 'price_ohlcv_1h', countIf(close_usd = 0 AND close > 0), count(),
       round(countIf(close_usd = 0 AND close > 0) / count(), 4)
FROM prices.price_ohlcv_1h  FINAL WHERE timestamp >= now() - INTERVAL 48 HOUR
UNION ALL SELECT 'price_ohlcv_4h', countIf(close_usd = 0 AND close > 0), count(),
       round(countIf(close_usd = 0 AND close > 0) / count(), 4)
FROM prices.price_ohlcv_4h  FINAL WHERE timestamp >= now() - INTERVAL 7 DAY
UNION ALL SELECT 'price_ohlcv_1d', countIf(close_usd = 0 AND close > 0), count(),
       round(countIf(close_usd = 0 AND close > 0) / count(), 4)
FROM prices.price_ohlcv_1d  FINAL WHERE timestamp >= now() - INTERVAL 30 DAY
UNION ALL SELECT 'price_ohlcv_1w', countIf(close_usd = 0 AND close > 0), count(),
       round(countIf(close_usd = 0 AND close > 0) / count(), 4)
FROM prices.price_ohlcv_1w  FINAL WHERE timestamp >= now() - INTERVAL 180 DAY
UNION ALL SELECT 'price_ohlcv_1M', countIf(close_usd = 0 AND close > 0), count(),
       round(countIf(close_usd = 0 AND close > 0) / count(), 4)
FROM prices.price_ohlcv_1M  FINAL WHERE timestamp >= now() - INTERVAL 400 DAY
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

> ⚠️ **`close_usd = 0 AND close > 0` is an upper bound, not the defect count.**
> It counts every *unpriced* row, which includes the permanent exotic-quote floor
> that is working as designed. The rows the `argMax` actually zeroed are the
> subset with a **priced sub-bucket underneath** — that is D2.

### D2 — the rows the `argMax` actually zeroed

The precise 0146/0148 population: a coarse row reading 0 while the tier below it
has a priced row inside the same bucket. Bounded to 7 days to stay cheap; widen
once the shape is known.

```sql
SELECT
    count()                       AS wrongly_zeroed_1h,
    uniqExact(h.asset_id)         AS assets_affected,
    min(h.timestamp)              AS oldest,
    max(h.timestamp)              AS newest
FROM prices.price_ohlcv_1h AS h FINAL
INNER JOIN (
    SELECT asset_id, quote_asset_id, source,
           toStartOfInterval(timestamp, INTERVAL 1 HOUR) AS bucket
    FROM prices.price_ohlcv_15m FINAL
    WHERE timestamp >= now() - INTERVAL 7 DAY AND close_usd > 0
    GROUP BY asset_id, quote_asset_id, source, bucket
) AS s
    ON  s.asset_id = h.asset_id AND s.quote_asset_id = h.quote_asset_id
    AND s.source = h.source     AND s.bucket = h.timestamp
WHERE h.timestamp >= now() - INTERVAL 7 DAY
  AND h.close_usd = 0 AND h.close > 0
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

**This number is the one to quote.** D is "how much is unpriced"; D2 is "how much
we broke". Swap `_1h`/`_15m` for `_1d`/`_4h` (and `INTERVAL 1 DAY`) to size the
daily tier, which is the one BE reads.

---

## E — the frozen estate only the [[0114]] sweep can reach

**New.** After [[0146]] guards the `argMax`, rows *inside* each MV's
re-aggregation window self-heal — the MV re-appends a correct value. Rows
*outside* it stay frozen forever and need [[0148]]. E measures that split.

The six windows, read from `rollups.sql:95,116,137,158,179,200`:

| Table | Fed by | Re-aggregation window (self-heal reach) |
|---|---|---|
| `price_ohlcv_15m` | `_1m` | `now() - 2 HOUR` |
| `price_ohlcv_1h` | `_15m` | `now() - 8 HOUR` |
| `price_ohlcv_4h` | `_1h` | `now() - 1 DAY` |
| `price_ohlcv_1d` | `_4h` | `now() - 7 DAY` |
| `price_ohlcv_1w` | `_1d` | `now() - 60 DAY` |
| `price_ohlcv_1M` | `_1w` | `now() - 400 DAY` |

**Run these one tier at a time.** They are unbounded in time by design — that is
the point — so they scan whole tables and will not prune partitions.

```sql
-- template: substitute TABLE and WINDOW from the table above
SELECT
    'price_ohlcv_1h'                                              AS tbl,
    countIf(ts_in)                                                AS zeroed_inside_window,
    countIf(NOT ts_in)                                            AS zeroed_outside_window,   -- <- sizes 0148
    uniqExactIf(asset_id, NOT ts_in)                              AS assets_outside,
    minIf(timestamp, NOT ts_in)                                   AS oldest_outside,
    maxIf(timestamp, NOT ts_in)                                   AS newest_outside
FROM (
    SELECT asset_id, timestamp, close, close_usd,
           timestamp >= toStartOfInterval(now() - INTERVAL 8 HOUR, INTERVAL 1 HOUR) AS ts_in
    FROM prices.price_ohlcv_1h FINAL
    WHERE close_usd = 0 AND close > 0
)
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

Repeat with `(_15m, 2 HOUR, 15 MINUTE)`, `(_4h, 1 DAY, 4 HOUR)`,
`(_1d, 7 DAY, 1 DAY)`, `(_1w, 60 DAY, 1 WEEK)`, `(_1M, 400 DAY, 1 MONTH)`.

**Read it as:** `zeroed_outside_window` is the estate that will still be wrong
the day after [[0146]] ships. If it is small, [[0148]] is a footnote and the
sweep picks it up. If it is large — and [[0136]]'s 17-day freeze plus the
2026-07-21→08-03 gap make that plausible — [[0148]] needs a real backfill plan
and the BE reply should say historical `close_usd` stays unreliable until it
runs.

⚠️ **Same upper-bound caveat as D.** For the precise figure, apply D2's
sub-bucket join with the window predicate added.

---

## Results

Fill in as they come back; then fold into the task README's acceptance criteria.

| Query | What it sizes | Result | Run at |
|---|---|---|---|
| A | XLM tip state + quote of newest candle | **UNPRICED tip confirmed.** 13:18 + 13:19 `close_usd = 0`, 13:17 and older priced. **Quote is USDC** (`GA5ZSEJY…`), *not* an exotic pair — so this is plain enrichment lag, not the permanent floor. | 2026-08-05 13:19:41Z |
| A2 | hours/24 where XLM publishes `price_usd = 0` | **Query was mis-specified — see below.** Per-hour `argMax` returns `published_now == if_guarded` for all 24 completed hours; only the in-flight hour shows 0 (2 of 27 candles unpriced, share 0.0741). `mv_current_prices` aggregates over the trailing **24h as a whole**, not per hour, so this framing cannot see the defect. **A3 replaces it.** | 2026-08-05 13:19:41Z |
| **A3** | **what `/price` actually serves for XLM** | 🔴 **`published_now = 0`, `if_guarded = 0.16720799309045`.** Finding 1 confirmed on the published surface. `vwap_24h = 0.16726314490953` (non-zero), `updated_at 13:22:00` (MV ticking normally). **`sources` omits `sdex` entirely** — see C2 below. | 2026-08-05 13:22Z |
| **A4** | enrichment frontier / cadence | `newest_candle 13:22`, `newest_priced 13:20`, `lag_minutes = 2`. ⚠️ **Not the steady state** — this and the A sample both landed in the catch-up window of the pass that began ~13:17. Deployed rule confirmed `rate(1 hour)`, `ENABLED`, no drift from `production.json`. **Re-run at :45–:55 past the hour** for the worst-case lag. | 2026-08-05 13:23Z |
| B1 | fan-out ratio (expect ~2.0) + read rows vs BE's 70.7M | | |
| B2 | duplicate `asset_id`s that carry candles | | |
| C1 | yXLM `priced_volume_share`; shipped vs unfiltered close | | |
| C2 | distribution of `priced_volume_share` → picks [[0147]]'s X | | |
| D | unpriced coarse rows, six tiers (upper bound) | | |
| D2 | **wrongly zeroed** rows (the real defect count) | | |
| E | frozen estate outside the MV windows → sizes [[0148]] | | |

### What A–A4 changed in the task's account of finding 1

Three corrections, all from data. The finding itself is **confirmed and more
severe**; two of the *reasons* the README gives for it are wrong.

**1. The quote is USDC, so the exotic-quote floor is not what's biting.** The
README gives "XLM's newest candle is often an exotic-quote pair … the
**permanent** deep-history floor" as compounding reason #2. The observed tip is
XLM/USDC on `sdex`, which enrichment prices from an oracle, and all 24 completed
hours show `unpriced = 0` — nothing on this pair is stranded. **Plain enrichment
lag fully explains finding 1 on its own.** The exotic-quote floor is real (it is
why option A cannot terminate) but it is a *separate* argument and should not be
offered to BE as the cause of the XLM zero.

**2. XLM publishes 0 essentially *always*, not "most of every hour".** The
README's reasoning — hourly enrichment leaves the tip un-enriched for most of
the hour — is correct as far as it goes, but understates the result. `argMax`
takes the **newest** candle, and XLM emits one nearly every minute. Enrichment
advances the frontier at most once an hour. So except for the brief moment right
after a pass completes, XLM's newest candle is *always* behind the frontier and
the aggregate *always* reads a zero. A3 measured exactly that:
`published_now = 0` against `if_guarded = 0.16720799309045`.

> ⚠️ **Correction (2026-08-05, same session).** This section first claimed the
> lag was a *steady ~2 minutes* rather than an hourly sawtooth, inferred from
> `newest_priced` sitting 2 minutes behind `newest_candle` in two samples and
> the frontier advancing 13:17 → 13:20 in 4 minutes of wall clock. **That was
> wrong.** `aws events describe-rule --name prices-production-enrichment`
> returns `"ScheduleExpression": "rate(1 hour)", "State": "ENABLED"` — the
> deployed rule matches `production.json` exactly, no drift. Both samples fell
> inside the *same* catch-up window right after a pass that began around 13:17;
> a frontier advancing while the pass executes is the pass walking forward to
> live, not a steady state. **The steady-state lag remains unmeasured and can be
> up to ~60 minutes just before the next run.**
>
> The conclusion above survives the correction — it holds *a fortiori* under
> hourly enrichment — but the **cost** estimate it was used for does not. The
> claim that guarding costs "~2–3 minutes of staleness" is **withdrawn**: the
> guarded value is up to **one enrichment cycle** old. That still beats a
> permanent zero, but it is a different number and [[0135]]'s contract call
> should be decided against the real one.

**Measured 2026-08-05 13:45:40 — the sawtooth is confirmed, and it is the
README's original picture.**

```
sampled_at 13:45:40   newest_candle 13:45   newest_priced 13:24   lag_minutes 21
```

Reconstructing the cycle from the three samples: the pass began ~13:17, ran for
about 7 minutes, carried the frontier to **13:24**, and stopped. The frontier
has been **static at 13:24 ever since** while candles kept arriving every minute
— which is why the lag reads 2 minutes mid-pass and 21 minutes twenty-one
minutes later. It grows linearly until the next pass.

| Time | Frontier | Lag | Phase |
|---|---|---|---|
| 13:19:41 | 13:17 | ~2 min | pass running |
| 13:23:00 | 13:20 | 2 min | pass running |
| 13:45:40 | **13:24** | **21 min** | frozen — pass finished |

**Worst case ≈ 52 minutes**, just before the next pass at ~14:17 (13:24 frontier
→ 14:16 tip). That is an extrapolation from a static frontier, not a
measurement; one more A4 at ~14:15 would confirm it, but the shape is no longer
in doubt.

**So the guarded value is up to ~50 minutes stale, averaging ~25.** That is the
number [[0135]] must decide against — not the "2–3 minutes" this note first
claimed, and not zero.

### Consequence: freshness improvements are gated on [[0111]]

If ~50 minutes is too stale for BE, the fix is a shorter enrichment cadence —
and **that is blocked by [[0111]]**, which measured enrichment re-scanning the
whole table every batch (490–545M rows, ~35 s/batch under backfill load). Going
from hourly to, say, 5-minutely multiplies that cost by 12 and walks straight
back into the outage 0111 documents.

This is a dependency the fix plan does not currently record: **[[0135]] fixes
the zero, but only [[0111]] can make the guarded value fresh.** Worth stating in
the BE reply so they calibrate expectations, and worth reconsidering 0111's
priority — it is currently filed as "not acutely urgent" on the grounds that
cost scales with table size rather than era.

**3. A2 could not see the defect** — it grouped by hour, while
`mv_current_prices` aggregates over the trailing 24h as a whole, so only the
in-flight hour could ever show a zero. A3 replaces it.

### C2 confirmed — and the filter discriminates against the busiest venue

`sources` for XLM returned **`aquarius`, `phoenix`, `soroswap`** — **no
`sdex`** — while query A showed `sdex` trading in almost every minute of the
sample window, including a 24,079-unit print at 13:06.

`per_source` (`current.sql:117-126`) applies no source whitelist: every source in
`price_ohlcv_1m` participates. The only thing that can remove one is
`WHERE src_price > 0` at `current.sql:140`, acting on the unguarded
`argMax(close_usd, timestamp)` above it. `sdex`'s newest candle was unpriced, so
its `argMax` returned 0 and the source was dropped. **That is scope correction
C2, measured on prod.**

Two consequences the task did not anticipate:

- **`vwap_24h = 0.16726314490953` was computed without `sdex`** — the venue with
  by far the most frequent XLM prints. The published VWAP is not "the 24h VWAP";
  it is the 24h VWAP over whichever venues happened to be enriched at refresh
  time.
- **The drop is biased toward high-frequency venues, which is exactly backwards.**
  A source is dropped iff its newest candle in 24h is unpriced. A venue trading
  every minute *always* has a candle inside the enrichment lag window; a venue
  that trades rarely usually has its newest candle behind the frontier, already
  priced. **The more a venue trades, the more likely it is to be excluded.**
- The source count fell 4 → 3, which is the threshold at which the median
  outlier filter stops being a no-op (`current.sql:72-76`). So the filter's own
  documented safety property is enrichment-timing-dependent — the README's C2
  predicted this; here it is with real numbers.

### Notes on running

- If a query errors on `do_not_merge_across_partitions_select_final`, drop the
  `SETTINGS` clause — the result is identical, just slower.
- B1 and E are the expensive ones. Everything else is bounded and cheap.
- Nothing here writes. There is no `ALTER`, `INSERT`, `OPTIMIZE` or `SYSTEM`
  statement in this file by design — see [[feedback-flag-container-restarts]].
