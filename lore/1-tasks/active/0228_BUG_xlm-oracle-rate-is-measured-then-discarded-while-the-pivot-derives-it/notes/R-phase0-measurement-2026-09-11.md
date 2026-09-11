---
title: "Phase 0 — what the pivot actually does to XLM-quoted candles, measured on prod"
type: research
status: mature
tags: ["oracle", "enrichment", "pivot", "usd", "measurement", "prod"]
links:
  - "sql/q1_population.sql"
  - "sql/q1b_population_1m.sql"
  - "sql/q2_depeg_day.sql"
  - "sql/q3_pivot_vs_reflector.sql"
  - "sql/q5_post_epoch_day.sql"
  - "sql/q6_affected_by_depeg_band.sql"
  - "sql/q7_oracle_tier_wins.sql"
  - "sql/q8_span.sql"
  - "../../../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: "2026-09-11"
    status: mature
    who: akot
    spawned_from: ["../README.md"]
    note: >
      Eight read-only queries against prod as dev_read (mTLS). Every number
      below is from that run; the SQL is in sql/ and re-runnable. Decides the
      task's scope — see README "Phase 0 outcome".
---

# Phase 0 — what the pivot actually does, measured

All figures: prod cluster `ch.sorobanscan.rumblefish.dev`, ClickHouse 26.3.10.60,
UTC, 2026-09-11 ~11:10 UTC, user `dev_read`. Asset ids: **XLM = 4**, canonical
**USDC = 3**, canonical **USDT = 111**. USDC oracle epoch = `1773237600` =
2026-03-11 14:00 UTC (`USDC_ORACLE_EPOCH_S`).

## 1. The task's premise holds only before the oracle epoch

Reflector's `XLM` symbol maps to `AssetIdentity::Native`
(`prices-ingest-core/src/soroban.rs:111`) and the oracle tier joins
`oracle_prices` on `o.asset_id = p.quote_asset_id` (`ch_enrich.rs:1259`). So an
XLM-quoted candle inside the oracle window is priced from the **measured**
XLM/USD reading, and the pivot never reaches it.

Checked on one post-epoch day (`q7`): of the 3,643 XLM-quoted 1d candles on
2026-08-01, **3,601 carry a Reflector XLM reading** (within 0.5 bps of one of
the day's polls) and **4 carry the XLM/USDC vwap** (the pivot value). The
remainder are outliers of neither shape.

🔑 The "we discard the measurement and infer it instead" claim is therefore
**false for 2026-03-11 →** and true only for the deep history, where there is
no measurement to discard. What is discarded is the *durable copy*: `usd_rate`
has zero native rows (`q-usd_rate`), so the readings exist only in
`oracle_prices`.

## 2. AC 1 — derived vs measured, over the whole oracle window (`q3`)

Per hour, 2026-03-11 14:00 → 2026-09-11 10:00, **4,413 hours** where all three
exist: Reflector XLM/USD, Reflector USDC/USD, and an XLM/USDC 1h candle.

| ratio − 1, in bps | p05 | p50 | p95 | \|p99\| | \|max\| |
|---|---|---|---|---|---|
| `pivot_raw / measured` (what `pivot_sql` computes, USDC units) | −35.1 | **−1.2** | +30.1 | 143.9 | 2,894 |
| `pivot_raw × USDC_usd / measured` (pivot scaled by the measured USDC rate) | −31.5 | **+1.5** | +34.1 | 143.7 | 2,899 |

Reading: the pivot tracks Reflector to within ±35 bps 90 % of the time, and
the USDC/USD scaling moves the median by under 3 bps — in the oracle window
USDC sits at par, so scaling cannot matter there. The residual ±35 bps is SDEX
XLM/USDC liquidity noise, not a systematic bias. The 2,894 bps tail is a
handful of hours with a thin or stale XLM/USDC market.

⚠️ This is the task's gating criterion and it comes out **"agrees within
noise"** — for the window where a measurement exists. That would close the
task as filed. It does not close the defect in §3, which the filed task did
not name.

## 3. The actual defect: the pivot never multiplies by the USDC/USD rate

`pivot_sql` (`ch_enrich.rs:2146`) computes `ref_usd` as the volume-weighted
XLM/USDC close, i.e. in **USDC units**, and applies it as if it were dollars.
After task 0268 rescaled every USDC-quoted candle by the measured USDC/USD rate,
the XLM-quoted (and USDT-quoted — same statement) candles are the only ones
left carrying the "USDC = $1" assumption.

The falsifier day (`q2`), daily table, 2023-03-11:

| what | value |
|---|---|
| stored `close_usd / close` on XLM-quoted 1d candles (= the ref rate the pivot applied) | 0.05878 – 0.05882, **7,328 candles** |
| XLM/USDC 1d candle: `close` / `close_usd` (the latter rescaled by 0268) | 0.05882 / **0.05695** |
| USDC/USD external rate that day (24 hourly rows) | 0.8833 – 0.99503 |

So every XLM-quoted candle that day is stored **≈ 3.2 % too high** (0.0588 vs
0.0569). On the 1h table it is 49,090 candles for that one day.

How much of the pre-epoch XLM-quoted population sits on a day where USDC was
off par (`q6`), joined to the daily external rate:

| \|USDC − 1\| on the candle's day | 1d candles | 1h candles | days |
|---|---|---|---|
| < 10 bps | 10,317,681 | 64,277,349 | 1,851 |
| 10 – 50 bps | 66,074 | 412,392 | 12 |
| 50 – 100 bps | 8,260 | 48,424 | 1 |
| ≥ 300 bps | 7,328 | 49,090 | 1 |

Every pre-epoch XLM-quoted priced candle falls on or after 2021-01-25 (`q8`),
so every one of them has an external USDC rate for its day — there is no
"no reference → must not reset" population for XLM. (USDT was not checked
separately; its pivot reference market begins 2021-02-07 per task 0182.)

## 4. Population a re-enrichment touches (`q1`, `q1b`)

Rows with `close_usd > 0`, no `FINAL` (sizing only):

| table | XLM-quoted, pre-epoch | XLM-quoted, post-epoch | USDT-quoted, pre-epoch | USDT-quoted, post-epoch |
|---|---|---|---|---|
| 1m | 72,509,319 | 5,783,162 | 1,550,966 | 19,810 |
| 15m | 8,891,491 | 2,119,735 | 573,703 | 11,110 |
| 1h | 64,787,255 | 3,737,956 | 697,042 | 40,051 |
| 4h | 28,993,564 | 1,870,716 | 260,678 | 21,432 |
| 1d | 10,399,343 | 794,658 | 70,741 | 6,998 |
| 1w | 3,344,815 | 399,564 | 11,824 | 1,301 |
| 1M | 1,405,100 | 185,175 | 4,449 | 228 |
| **total** | **≈ 190.3 M** | | **≈ 3.17 M** | |

For scale: 0268's campaign re-priced 9.94 M USDC-quoted candles in ~24 min
(0276 run record). Same shape here is ~19× the rows, so plan for hours, not
minutes, and for the partition FREEZE set to be correspondingly larger.

The `unpriced` (`close_usd = 0`) XLM-quoted counts are large (569 M on 1m)
and are candles with `volume_quote = 0` or predating the XLM/USDC market —
not this task's concern.

## 5. Retention: there is no TTL, and the 13 months is a dark worker's policy

`system.tables.engine_full` for `prices.oracle_prices` carries **no `TTL`
clause**, and `init.sql` defines none. The "pruned at INTERVAL 13 MONTH"
the task (and `init.sql:297`, `writer.rs:366`) refer to is
`cleanup-worker/src/lib.rs:33` — the cleanup worker, which is deployed
**dark** since the 0182/0201 incident.

`oracle_prices` today: Reflector rows for XLM and USDC, **52,607 each**,
2026-03-11 14:00 → 2026-09-11 11:05, no rows before 2026-03 (the 1970 rows
0227 describes are gone). Earliest possible loss of an XLM reading is therefore
**2027-04-11**, and only if the cleanup worker is re-lit. The `oracle-worker`
comment at `lib.rs:65-70` ("if 0154 has not started before `202509` ages out
(~2026-10/11)") is stale on both counts.

## 6. Side finding, not chased

`oracle_prices` also holds **419,519 rows** with `oracle_name = 'redstone'`,
`asset_id = 0`, `price_usd = 0`, 2025-09-08 → now (partitions 202509–202609),
`raw_data` = a base64 XDR blob. Nothing in the enrichment reads
`oracle_name = 'redstone'`, so they price nothing; they do inflate the table and
`asset_id = 0` resolves to no asset. Routed to Adam as a candidate backlog
item; out of scope here.

## What this decides

1. AC 1 as filed is **met** ("agrees within noise") — but the task's gating
   logic was aimed at the wrong question. The measured-vs-derived gap is not
   where the money is; the missing USDC/USD factor is.
2. The fix is 0268's shape on the pivot: multiply `ref_usd` by the USDC/USD
   rate at the bucket end, then a bounded, resumable re-enrichment of the
   pre-epoch XLM/USDT-quoted population. Ratified by Adam 2026-09-11 as the
   **full** population, not a ≥10 bps day subset.
3. The durable copy of XLM readings (a snapshot into `usd_rate`) has no
   deadline any more, but it stays in scope — ratified 2026-09-11 — as a
   small, separately named set beside `peg_identities()`.
