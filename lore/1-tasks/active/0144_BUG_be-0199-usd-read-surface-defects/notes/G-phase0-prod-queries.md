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
    candles,
    joined_rows,
    round(joined_rows / candles, 3) AS fanout_ratio
FROM (
    SELECT
        (SELECT count() FROM prices.price_ohlcv_1d FINAL
         WHERE timestamp >= now() - INTERVAL 104 WEEK)                        AS candles,
        (SELECT count() FROM prices.price_ohlcv_1d AS p FINAL
         INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
         WHERE p.timestamp >= now() - INTERVAL 104 WEEK)                      AS joined_rows
)
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

> ⚠️ **Do not use `countDistinct(asset_id, timestamp, source, quote_asset_id)`
> here** — the first draft of this query did. It builds exact `uniqExact` state
> over ~35M distinct 4-tuples and is a good bet to hit the **5.59 GiB memory
> quota** that already bit the [[0090]] pre-roll. Two plain `count()`s give the
> same ratio *exactly*, for almost nothing: the un-joined count is the candle
> population by definition, since the join adds rows and never removes them.

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

### B3 — compare our read cost against BE's measurement

Run **immediately after B1**, in the same session. BE measured 70.7M read rows /
4.6 s / 2.1 GiB for the 104-week window; this says what the same shape costs us
and how much of it the fan-out accounts for.

```sql
SELECT
    query_duration_ms,
    read_rows,
    formatReadableSize(read_bytes)   AS read_bytes,
    formatReadableSize(memory_usage) AS peak_memory
FROM system.query_log
WHERE type = 'QueryFinish'
  AND event_time >= now() - INTERVAL 15 MINUTE
  AND query LIKE '%fanout_ratio%'
  AND query NOT LIKE '%system.query_log%'
ORDER BY event_time DESC
LIMIT 3;
```

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
  AND p.timestamp >= now() - INTERVAL 48 HOUR
GROUP BY issuer, bucket
ORDER BY bucket DESC
SETTINGS do_not_merge_across_partitions_select_final = 1;
```

**48 hours, not 24, deliberately:** BE measured the **2026-08-04 13:00** bucket,
which a 24-hour window run on 08-05 afternoon would just miss.

The last two columns are BE's two options side by side, per bucket. Expect
`close_usd_if_unfiltered` to be **catastrophically low** wherever
`priced_volume_share < 1` — that is TEST B's 0.000023 reproducing on prod, and
it is the evidence for telling BE their option B is worse than the status quo.

**Two things to expect, so the output is not misread:**

- **BE's own 08-04 13:00 bucket should now read `priced_volume_share = 1.0`
  and a sane `close_usd` (~0.170).** Enrichment has had a day to finish it. That
  is not a refutation of their report — it is the *proof* of it: the bucket they
  saw at 1.3085 has silently become a different number, so **the view's answer
  for a historical bucket changed retroactively**, which is precisely the
  behaviour BE cannot build on. Compare it against their reported 1.3085.
- **The live case is only visible in the newest one or two buckets.** The
  current hour is the one that can still be part-enriched, so that is where a
  `priced_volume_share < 1` will show up.

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
| **B1** | fan-out ratio (expected ~2.0) | 🔵 **1.047 — the ~2× expectation is falsified.** `candles 11,685,065`, `joined_rows 12,233,490`; the join adds **548,425 rows = +4.7%**. | 2026-08-05 ~13:55Z |
| **B2** | duplicate `asset_id`s that carry candles | **3,278** dup `asset_id`s / 6,562 identities; **2,493 carry candles**, totalling **548,439 candle rows**. Cross-checks B1's surplus to within 14 rows. | 2026-08-05 ~13:55Z |
| **B3** | our read cost vs BE's 70.7M / 4.6 s / 2.1 GiB | **B1: 344 ms, 33,003,949 read rows, 707.33 MiB, 274 MiB peak. B2: 255 ms, 16,943,576 rows, 385.69 MiB.** The join is *not* the cost — see below. (First attempt returned empty on `query_log` flush lag; needed `SYSTEM FLUSH LOGS`.) | 2026-08-05 13:50:25Z |
| **C1** | yXLM `priced_volume_share`; shipped vs unfiltered close | **BE's 08-04 13:00 bucket now reads 0.16931** against the **1.3085** they saw live — the retroactive change, confirmed. Their 12:00 bucket reads `vol_total 42,037.752` = their "42,038", so we are on exactly their data. **The two newest buckets (08-05 13:00, 14:00) are 100% unpriced and absent from the view.** | 2026-08-05 ~14:05Z |
| **C2** | distribution of `priced_volume_share` → picks [[0147]]'s X | 🔴 **Not bimodal — three modes and a fat middle.** 1.00: 32.1% · **0.50: 16.9%** · 0.00: 16.8% · ~34% spread across (0,1). **Plus 11 buckets with a *negative* share.** ~34,970 buckets over 48h. | 2026-08-05 ~14:05Z |
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

### What B1/B2 changed in finding 2a — the "~2×" claim is falsified

The task's verdict table records finding 2 as *"**No** — a valid request, but
~2× of the measured scan **is** a bug: [[0139]]'s `asset_id` fan-out"*, and §2a
calls a 2× multiplication "exactly what that produces". **At prod scale it
produces 4.7%.**

Both halves of §2a's TEST D were sound *as a mechanism demo* — one `asset_id`
with two identities does join to two rows, and the second identity does publish
a series it never traded. What does not survive is the **generalisation from one
synthetic asset to the whole table**. The population is what decides the ratio,
and the population is thin:

- 3,278 `asset_id`s carry two or more natural identities (0139 measured 3,275 —
  it has drifted by 3, consistent and still current).
- Only **2,493** of them carry daily candles at all.
- Those account for **548,439 of 11,685,065** candle rows — **4.7%**.

So the duplicated identities are concentrated in **thin, long-tail assets**:
220 candles each on average, against a table-wide mean that is several times
higher. The fan-out is a **correctness** bug affecting the long tail, not a
**performance** bug affecting the headline scan.

**Three consequences:**

1. **The BE reply must not promise a read-cost improvement from fixing 0139.**
   The draft said "roughly half of that scan is our bug". It is 4.7%. Fixing the
   fan-out will not measurably change BE's 4.6 s, and telling them otherwise
   sets up a broken promise at exactly the moment we are rebuilding their trust
   in these views.
2. **BE's finding 2 is therefore *more* clearly a real request, not less.** Their
   read cost is not half our defect — it is the shape of the view. The
   pre-authorised materialisation ([[0150]]) is justified on its own merits.
3. **[[0139]]'s ordering ahead of [[0150]] still holds, on the original
   correctness grounds** — materialising would bake 548,439 rows' worth of
   wrong-identity attribution into a physical table. Only the *performance*
   rationale for that ordering is withdrawn.

### B3 — the join is cheap; the *aggregation* is the cost

```
B1  344 ms   33,003,949 read rows   707.33 MiB   274 MiB peak
B2  255 ms   16,943,576 read rows   385.69 MiB   290 MiB peak
BE  4.6 s    70,700,000 read rows   2.1 GiB      (their 104-week window)
```

**B1 performs the identical `FINAL` join over the identical window and costs
344 ms.** BE's query costs 4.6 s — 13× more. The difference is not the join and
not the fan-out; it is everything B1 deliberately skipped: the weighted
aggregation `sum(close_usd × volume_base) / sum(volume_base)` and the `GROUP BY`
on **four computed identity columns** (`multiIf`/`if` expressions, so no index
can help and the whole window must be materialised before grouping).

**This is good news for [[0150]]** — it means the pre-authorised fix targets the
actual bottleneck. Materialising precomputes exactly the aggregate that costs
the 4.6 s. Had the cost been in the join, an identity-keyed table would have
helped far less.

**A second reading, worth confirming:** B1 scans the window twice (two
subqueries) and read 33.0M rows against 11.685M deduped — ~16.5M raw per scan,
a **raw:deduped ratio of ~1.41**. B2's single scan read 16.9M, corroborating.
So roughly **30% of the rows `FINAL` reads in that window are unmerged RMT
duplicates**. That is a real cost multiplier on every read of these views, it is
independent of the fan-out, and it may be a [[0136]] after-effect — the coarse
tables were frozen for 17 days and merges only resumed on 08-03. Worth
re-measuring once merges have caught up, before sizing [[0150]] against it.

⚠️ **Caveat: cache state.** BE reported 4.6 s "per uncached request"; our 344 ms
may have hit a warm page cache. The read-row counts are comparable regardless —
the wall-clock is not.

### What C1 established

**BE's report is corroborated on their own data, down to the volume figure.**
The 2026-08-04 12:00 bucket reads `vol_total 42,037.752` — their "42,038 units".
Same rows, same asset, same window.

**The retroactive change is real and now documented.** Their 13:00 bucket, which
they watched read **1.3085** at 13:29 and then vanish by 14:13, today reads
**0.16931024**. A consumer that stored the published value at 13:29 holds a
number our own view no longer agrees with, and nothing signalled the change.
That is the part of finding 3 that makes these views unusable for BE's purpose,
independent of the dust-print magnitude.

**Finding 3ii is happening live, right now.** The two newest buckets —
2026-08-05 13:00 and 14:00 — have `rows_priced = 0` of 3, so
`close_usd_as_shipped` is `NULL` and **both are absent from
`price_usd_series_1h` entirely**. That is BE's 14:13 observation reproducing on
prod while we watch. ⚠️ **It is not yet proven that the `argMax` zeroed them**
rather than there being no priced data underneath — the `_1m` frontier was at
13:24, so the 13:00 bucket *should* have priced sub-buckets. **D2 settles it**,
and this is now the sharpest possible test case for it.

**The filter behaves correctly where coverage is high**, which is worth saying
plainly because it bounds the fix: at `priced_volume_share` 0.999997 (08-05
01:00) the shipped and unfiltered closes differ in the 7th decimal. The filter
only misleads when coverage is *low* — which is exactly what the gate targets.

**Incidental: `asset_code = 'yXLM'` is ambiguous across three issuers.** Besides
Ultra Capital's `GARDNV3Q7…` (the real one, ~0.167), the window contains
`GCWXXTUR…` and `GD23M4RJ…YXLM` — both trading at **~0.000009 USD**, i.e.
imitations. BE keys on natural identity so they are not exposed, but anyone
keying on `asset_code` alone would blend a real asset with two worthless ones.
Worth a line in the reply.

### 🔴 C2 — the distribution is not bimodal, and it changes [[0147]]'s design

The note above predicted "a healthy world is bimodal". It is not:

| `priced_volume_share` | buckets | share |
|---|---|---|
| **1.00** (fully priced) | 11,226 | **32.1%** |
| **0.50** exactly | 5,920 | **16.9%** |
| **0.00** (fully unpriced) | 5,885 | 16.8% |
| everything in (0,1) | ~11,900 | ~34% |
| **negative** | 11 | — |

**~51% of buckets sit strictly between 0 and 1.** A gate at X = 0.95 would
therefore suppress roughly **half of all buckets**, not the thin tail the design
assumed. That is the single most important number for [[0147]] and it argues
strongly that:

1. **The threshold cannot be chosen for BE.** Shipping `priced_volume_share` as
   a column so consumers set their own bar moves from "nice to have" to
   **required** — the alternative is imposing a 50% blackout on a consumer who
   may be perfectly happy weighting a 0.7-coverage bucket.
2. **A single global X is probably the wrong shape.** The middle is broad and
   flat (~0.3–0.5% per 0.01 bin), so no threshold is a natural cut point.

⚠️ **Caveat before this drives a decision: the distribution is unweighted.**
Every bucket counts equally, so the thin long tail — which [[B2]] already showed
dominates by count — dominates here too. A volume-weighted distribution could
look completely different and is the one that matters for a consumer reading
majors. **Re-run weighted by `vol_total` before fixing X.**

### 🔴 New finding: `volume_base` can be negative

**11 buckets returned a *negative* `priced_volume_share`** (−0.63, −0.48, −0.46,
−0.38, −0.11, −0.08, −0.07, −0.03 ×2, −0.01 ×2). The expression is
`sumIf(volume_base, close_usd > 0) / nullIf(sum(volume_base), 0)`; a negative
result requires numerator and denominator of opposite sign, so **at least one
`price_ohlcv_1h` row carries a negative `volume_base`.**

`volume_base` is `Decimal(38,14) DEFAULT 0` with no constraint
(`init.sql:112`). Nothing prevents it.

This matters beyond a curiosity:

- **It breaks the coverage gate's arithmetic.** A share can be negative or
  exceed 1, so `share >= X` is not a safe predicate as written. [[0147]] must
  either clamp or exclude non-positive volume explicitly.
- **It corrupts the volume-weighted mean itself** — `sum(close_usd × volume) /
  sum(volume)` with a negative weight can produce a price outside the range of
  its inputs, in *any* bucket containing one, priced or not. That is a
  correctness bug in `price_usd_series*` independent of every finding in this
  task.
- 11 of ~34,970 buckets is small, but the *unweighted* count says nothing about
  the volume involved.

**This is not in scope for 0144 and should not be absorbed into it.** It needs
sizing (how many rows, which sources, since when) and probably its own task
under [[0116]]'s umbrella — that task already owns junk candles reaching every
granularity. Query below.

### The 0.50 spike needs explaining before X is chosen

**16.9% of all buckets land on exactly 0.50.** A continuous distribution
rounding to 2 decimals would put ~1% in that bin — this is ~17×. Something
structural produces "exactly half the volume is priced", and until we know what,
any threshold near 0.5 is being set against a mechanism we do not understand.

The obvious hypothesis is **two rows per bucket with equal `volume_base`, one
priced and one not** — which would follow if the same logical candle exists
twice under different `(quote_asset_id, source)` keys, or if an enriched and an
un-enriched copy of a row are both surviving `FINAL`. The latter would tie
directly to [[0149]]'s version-ownership problem. Query below.

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
