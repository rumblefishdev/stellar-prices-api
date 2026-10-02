---
id: "0207"
title: "218 XLM-quoted candles carry a close_usd the candle itself cannot justify — one is 5.1M× its quote-leg price"
type: BUG
status: completed
assignee: akot
related_adr: ["0287"]
related_tasks: ["0172", "0168", "0145", "0196", "0173", "0286", "0182"]
tags: ["priority-medium", "effort-small", "clickhouse", "data-correctness", "enrichment", "oracle"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-18
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0172]]'s sweep criterion, which asked whether the USDT peg
      defect was a class or a one-off. It was a one-off — only USDC shows the
      peg fingerprint and it is legitimately $1 — but the same sweep surfaced
      this, which is a different defect wearing the same symptom. Filed
      separately rather than widening 0172, whose scope is the peg class.
  - date: "2026-09-29"
    status: completed
    who: akot
    note: >
      Closed as superseded by [[0286]]. Measured on prod: not the oracle tier
      (Reflector's XLM series starts 2026-03-11; every affected row is 2022)
      but the pre-0286 coarse roll carrying `close_usd` from a different
      sub-bucket than `close` — 1 757 rows on 1h/4h/1d/1w/1M, XLM leg only,
      2022-01 … 2022-04. The rate form (ADR 0287 §5) cannot write it and
      phase 3 rebuilds those rows; a local replay of the phase-3 SQL on the
      real `1m` rows fixed all of them, and the legacy formula on the same
      input reproduced prod's worst values. Its after-check is re-ingest
      runbook §7e-2 and a phase-3 criterion on 0286. No code was written.
---

> **Superseded by [[0286]] (2026-09-29).** The oracle hypothesis below was
> refuted by the measurement in "Resolution"; the defect is the pre-0286
> carried product, which ADR 0287's rate form ends and 0286 phase 3 removes
> from history. The text below is kept as the original analysis.

# `close_usd` disagrees with the candle on 218 XLM-quoted rows

## Measured on prod 2026-08-18 (`price_ohlcv_1d`)

```
XLM-quoted candles with close > 0 AND close_usd > 0 : 11,012,703
  implied rate (close_usd / close), mean              0.656471
  implied rate, stddev                             1551.634408   <- the tell
  p999                                                0.548       <- a real XLM price
  rows with implied rate > 1                            218
  worst                                         5,149,014.49
```

**The bulk is sound.** `p999 = 0.548` is a plausible XLM/USD price, so 99.998% of
these rows carry a properly measured rate. The mean and the standard deviation
are both artefacts of the tail.

**The tail is not.** XLM never traded above ~$0.9 in this window, so an implied
rate above 1 means `close_usd` claims the base asset is worth more USD than its
own XLM-denominated close can support. At 5.1M× it is off by six orders of
magnitude.

## Why this is not the [[0182]] dust class

The 18 rows 0182 left at `close_usd = 0` are dust — `close` at or below ~4e-14,
where `rate × close` underflows at `Decimal(38, 14)`. That produces a **zero**,
never an inflated value: rounding cannot manufacture magnitude.

A ratio of 5.1M needs a **real** `close_usd` sitting against a near-zero `close`.
So the USD value did not come from `rate × close` at all — it came from somewhere
that disagrees with the candle.

## The prime suspect: the oracle tier

`ch_enrich.rs` runs the oracle tier **first**, and it **wins where it applies**.
Unlike the peg-pivot tier it writes `close_usd` directly from a Reflector price
rather than deriving it from the candle's own quote leg, so the two are never
reconciled. A stale or mis-attributed Reflector price on an illiquid asset
produces exactly this shape.

That is the same failure mode [[0196]] already measured once — 46,378
mis-attributed Reflector rows on the USDT identity — and the same hazard [[0168]]
is filed against. ⚠️ [[0173]] records that `usd_rate`/`oracle_prices` file real
Tether's $1 under our depegged issuer, so oracle↔asset-id attribution is a known
weak point rather than a hypothetical one.

**Not yet checked, and it is the first thing to check:** whether these 218 rows
carry `method = 'oracle'`. If they do, the diagnosis is confirmed and the
question becomes which assets and why. If they do not, the cause is elsewhere and
this task needs re-scoping before anything is written.

## Why it matters at 0.002%

A wrong-but-nonzero `close_usd` reads as a real price at the **~130 unguarded
`argMax(close_usd, …)` sites** ([[0145]]) — the same reason 0182 argued a
wrong-but-visible number is worse than an honest gap. 218 rows is small, but each
one is a maximum, and `argMax` is exactly the aggregate that a six-order-of-
magnitude outlier dominates. One such row can drive a whole asset's published
price.

⚠️ **Do not assume it is 218.** That count is `price_ohlcv_1d` only, and only for
`quote_asset_id = XLM`. The coarse tiers roll the same trades up and USDT is a
second pivot leg; both want measuring before the fix is scoped.

## Implementation

- Confirm the source: `method` on the 218 rows, and whether they cluster by
  `asset_id`, by date, or by source.
- Re-measure across all five forever-tables and both pivot legs (XLM and USDT),
  not just `_1d`/XLM.
- Decide the rule. An oracle price that contradicts the candle's own quote leg by
  orders of magnitude is not more authoritative for being an oracle — the tier
  ordering assumes it is. Either the oracle tier gains a sanity bound against the
  candle, or the attribution that lets a wrong price reach these rows is fixed at
  source.
- Whatever the rule, pin it with an IT on the 26.3.10.60 pin.

## Resolution — measured 2026-09-29

Read-only on prod as `dev_read`; SQL in the session scratchpad, not the repo.

**Still there after 0286's phase 1.** `_1d`, XLM-quoted, rate > 1: exactly 218,
worst 5,149,014 — unchanged since 2026-08-18. Phase 3 had reached 201903, and
no month it had rebuilt holds a priced XLM-quoted row, so no before/after
comparison was possible on this class yet.

**Not the oracle.** Candles store no `method`; `/ohlcv` derives it, and an XLM
or USDT leg is always `traded`. Reflector's XLM series starts 2026-03-11, and
every affected row is `sdex` from 2021/2022 — the oracle tier cannot have
priced any of them. In its own era the oracle tracks the market (Reflector vs
XLM/USDC `1h` close: p50 1.0002, p0.1 0.966, p99.9 1.119), so no sanity bound
on that tier is warranted by this task.

**The mechanism: a carried product.** The pre-0286 roll was
`argMax(close, t.timestamp)` beside `argMaxIf(close_usd, t.timestamp,
close_usd > 0)`, so `close` came from the last child and `close_usd` from the
last *priced* one — the decoupling [[0145]] accepted in
`preroll-incremental.sql`. Two ways to split them:

- a dust last child whose `close_usd` underflowed to 0 at `Decimal(38, 14)`:
  `_1h` 2022-02-11 07:00, asset 19211 — `close` 2e-14 from minute 07:54,
  `close_usd` from 07:48 (`close` 1e-7), a rate 1,146,365× XLM's;
- the enrichment sawtooth: on 2022-04-13 every carried `close_usd` comes from
  an hour ≤ 14 and never from the last child, so the roll ran while `1m` was
  priced only to ~14:59. That day is 216 of the 218.

**Population** — rate more than 10× off its quote leg's median rate in the
same bucket, `close_usd` ≥ 1e-12, both directions:

| tier | 1h | 4h | 1d | 1w | 1M |
| --- | --- | --- | --- | --- | --- |
| XLM leg | 25 | 99 | 394 | 1 236 | 3 |
| USDT leg | 0 | 0 | 0 | 0 | 0 |

All in 2022-01 … 2022-04; none after the 0286 cutoff (2026-09-22 12:00 UTC);
`1m` in the same window has none, so the source rows are sound. A 2× band is
noise on `1w`/`1M` (intra-period XLM drift, USDT's June 2022 depeg month).

**Why 0286 removes it — checked, not assumed.**

- The six prod MVs are rate-form since 2026-09-22 and refresh over short
  windows (2 h … 400 d), so they never touch 2022 either way.
- Phase 3 drops each month's 15m/1h/4h/1d partitions and re-rolls them with
  `preroll-live-gap.sql`; `finish` truncates 1w/1M and re-rolls them with
  `preroll.sql`; §7b-2's `coarse-repair` prices each row as `close × rate`
  from that row alone. All three are rate-form with the `close ≥ 1e-12` gate.
- Replay on a local 26.3.10.60: prod's `1m` for the 1 448 affected assets,
  2022-01 … 2022-04 (1.38 M rows), through exactly those files. The replay was
  scoped to the 1 771 rows an approximate-median pass flagged; every one
  lands within 0.86–1.07× of its bucket median, none at zero, and every one
  of the 1.27 M rolled rows carries a rate inside its children's range.
- Reverse check: the legacy formula on today's `1m` reproduces only the 14
  dust rows; with `close_usd` zeroed from 2022-04-13 15:00 (the sawtooth) it
  reproduces 1 626 of the 1 771 with prod's exact worst values (`1d`
  26,684,072×, `4h` 24,727,764×, `1M` 229×). The rate form on that same input: 0.
- Legacy `1m` rows have `pf_trade_count DEFAULT trade_count`, so the dust gate
  was inert in the replay; the rate form alone was enough.

**Left as is, on purpose.** No repair of the existing rows: phase 3 overwrites
them, and a write now would be overwritten. 15m … 1d are fixed when phase 3
passes 2022-04; the 1 236 `1w` and 3 `1M` rows stay until its `finish`, which
waits for every planned month. Rolling a 2022 month back restores them.

## Acceptance Criteria

- [x] Source confirmed — `method` on the affected rows, and their clustering
      → no `method` column exists; not the oracle; the pre-0286 carried
      product, `sdex`, 2022-01 … 2022-04, mostly 2022-04-13 (Resolution).
- [x] Population re-measured across all five forever-tables and both pivot legs,
      not extrapolated from `_1d`/XLM → 1 757 rows, XLM leg only (table above).
- [x] A rule decided and stated: what makes an oracle price untrustworthy
      relative to the candle it is pricing → restated for the real cause: a
      `close_usd` whose rate is more than 10× off its quote leg's median rate in
      the bucket. Now [[0286]]'s phase-3 after-check (re-ingest runbook §7e-2).
- [x] Existing rows corrected, or an explicit decision to leave them with the
      reasoning recorded → left for 0286 phase 3 (Resolution, last paragraph).
- [x] Regression test on the 26.3.10.60 pin → covered by 0286's
      `preroll_close_usd_guard_it`, `rollup_pf_it` and `rollup_chain_it`,
      whose fixtures tell the rate form from the carried product; no new test.
