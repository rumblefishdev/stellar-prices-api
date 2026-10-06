---
id: "0236"
title: "Nothing detects an internally inconsistent `price_ohlcv_*` row — and 0229's clamp removed the one surface that used to surface them"
type: BUG
status: completed
assignee: akot
related_adr: ["0011"]
related_tasks: ["0229", "0120", "0182", "0227"]
tags: ["priority-medium", "effort-small", "data-correctness", "observability", "ohlcv", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../tools/scripts/conformance-0120.mjs"
history:
  - date: "2026-08-28"
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0229]]'s code review, finding 3. Not a defect in that fix —
      a consequence of it that is worth owning explicitly rather than leaving
      implicit in a PR thread.
  - date: "2026-09-28"
    status: active
    who: akot
    note: >
      Activated. Research first (statistical/econometric treatment of OHLC
      consistency and how other data providers do it), then the prod baseline,
      before any detector or alarm is designed.
  - date: "2026-09-29"
    status: blocked
    who: akot
    by: ["0286"]
    note: >
      Detector, alarm, ITs and the ADR-0011 s3 amendment are built on
      fix/0236_ohlc-band-detector (not yet merged). Blocked on the deploy date:
      not before 2026-10-04. The per-tier windows of `_1w` (bucket 2026-09-21)
      and `_1M` (bucket 2026-09-01) still hold rollups of pre-0286 days that
      inherit the legacy `low = 0` (measured 22 in `_1w`, 109 in `_1M`), so the
      alarm would fire on deploy with nothing new broken. No floor in code
      (Adam's decision); the window moves past those buckets by 2026-10-04.
  - date: "2026-10-06"
    status: active
    who: akot
    note: >
      Unblocked: the deploy date passed. The detector's own queries read 0 on
      all seven tiers on prod, so the alarm will not fire on legacy rows. The
      branch is rebased on develop, alongside 0139's and 0203's probe checks,
      as block 5c. PR to follow; deploy is Adam's.
  - date: "2026-10-06"
    status: completed
    who: akot
    note: >
      PR #396 merged (81a48f08) after review; Observability (3 ohlc-band
      alarms) and EventBridge (probe Lambda only) deployed 2026-10-06. First
      run 13:05 UTC: 7 tiers read, 0 violations, checks_failed 0, alarms OK.
      Probe 104 unit tests, 40 ClickHouse ITs.
---

# No detector for an internally inconsistent stored candle

## Summary

Nothing checks that a stored `price_ohlcv_*` row satisfies
`low <= open,close <= high` **at the source**. Until [[0229]], the `/ohlcv` read
path leaked such rows into the response, where [[0120]]'s conformance assertion
caught them. 0229's clamp — correctly, for a read API — now returns a
well-formed candle regardless, so that accidental detector is gone.

## Why this is not an argument against 0229's clamp

The clamp is right for the surface it is on. A read API's job is to return
well-formed candles; a consumer cannot act on `high < close` except by breaking.
And the alternative the review floated — bounding the clamp so only ulp-sized
crossings are repaired — buys the signal by **serving a malformed candle to
every consumer** whenever a source row is genuinely corrupt. That is paying in
the wrong currency.

🔑 **The real problem is that the detector was in the wrong place to begin with.**
Source-row consistency was being checked, accidentally, by a conformance suite
run by hand against a deployed API, on 20 assets, over a 7-day window. That it
ever found anything was luck.

## What is actually at risk

This repo has repeatedly found corruption on these tables — [[0182]]'s reset-epoch
candles, [[0227]]'s oracle timestamps — and in both cases it was found by
stumbling over it rather than by anything watching. An internally inconsistent
row is now invisible end to end:

| layer | would it notice? |
|---|---|
| ingestion | no check exists |
| enrichment | reads `close`/`close_usd` only |
| `/ohlcv` | **no — clamped since 0229** |
| 0120 conformance | no — it sees the clamped output |

## Implementation

- A check over `price_ohlcv_{1m,15m,1h,4h,1d,1w,1M}` for
  `high < greatest(open, close) OR low > least(open, close)`, run where the other
  data-quality probes run rather than as a one-off query.
- ⚠️ **Establish the baseline before deciding the alarm.** The count may be zero,
  in which case this is cheap insurance; or it may be large, in which case it is
  a bug report and the threshold question is secondary. Do not design the alarm
  first.
- Consider whether the clamp firing is worth counting at the API. It is the only
  place that currently *knows*, but it has no metric path today and adding one
  per-request is not obviously worth it — decide with the baseline in hand.

## Baseline — measured on prod 2026-09-28

Read-only, `dev_read`. Pass 1 without `FINAL` over every row of every tier
(~1.33 B rows; `_1m` full scan took 10 s), so every count below is an **upper
bound** — superseded RMT versions are included.

| tier | rows | band violations | priced row with a price ≤ 0, before 2026-09-22 12:00 | same, after |
|---|---:|---:|---:|---:|
| `_1m` | 800.9 M | **0** | 7,651 | 0 |
| `_15m` | 167.8 M | **0** | 3,041 | 0 |
| `_1h` | 215.6 M | **0** | 15,003 | 0 |
| `_4h` | 95.3 M | **0** | 12,326 | 0 |
| `_1d` | 32.2 M | **0** | 9,382 | 0 |
| `_1w` | 7.4 M | **0** | 5,010 | — |
| `_1M` | 2.7 M | **0** | 4,350 | — |

- **Band violation** = `pf_trade_count > 0 AND (low > least(open, close) OR
  high < greatest(open, close) OR low > high)`. 🔑 **Zero on every tier, every
  source, since 2015** — before and after [[0286]]. Nothing to diagnose; AC 2
  does not trigger for this predicate.
- `pf_trade_count = 0` rows carrying any non-zero price: **0** on every tier.
- **Priced row with a price ≤ 0** is the only non-zero class. Every one is an
  exact **zero**, none negative; almost all `sdex`, 2021-11 onward, most often
  `low = 0` with `close > 0` (on `_1h` `low ≤ 0` outnumbers `close ≤ 0`), i.e.
  the dust-fill `low` that ADR 0287 removed. ⚠️ `zero_invariants` (invariant 3)
  only sees the `close = 0` half of it; `low = 0` beside a positive `close` is
  caught by nothing today.
- ⚠️ **The 0286 cutoff is 2026-09-22 12:00 UTC, not 00:00.** The phase-1 rollout
  ran 10:35–11:58 UTC. The one "post-0286" row the first cut found (pair
  `80343/4`, `sdex`, bucket 06:59, all four prices `0`, `pf_trade_count = 1`,
  propagated to `_15m`…`_1d`) was written by the old ingest before the rollout.
  With the right cutoff the post-0286 count is **0** on every tier.
- ⚠️ **`FINAL` matters for this class**: `_1m` 2026-09 reads 143 without `FINAL`
  and **52** with it. The detector must use `FINAL`.
- The legacy zero-price rows are pre-0286 residue that [[0286]] phase 3
  re-ingests; they are recorded here, not filed as a separate task.

## Pre-deploy check — measured on prod 2026-10-06

Read-only, `dev_read`, 10:11 UTC. These are the detector's own queries
(`ohlc_band_queries()`: `FINAL`, 2 days widened by one bucket):

| tier | band | nonpositive | scanned | oldest bucket in window |
|---|---:|---:|---:|---|
| `_1m` | 0 | 0 | 662,241 | 2026-10-04 10:12 |
| `_15m` | 0 | 0 | 296,212 | 2026-10-04 10:00 |
| `_1h` | 0 | 0 | 178,303 | 2026-10-04 10:00 |
| `_4h` | 0 | 0 | 98,634 | 2026-10-04 08:00 |
| `_1d` | 0 | 0 | 39,812 | 2026-10-04 |
| `_1w` | 0 | 0 | 44,609 | 2026-09-28 |
| `_1M` | 0 | 0 | 26,016 | 2026-10-01 |

- The legacy `_1w` 2026-09-21 and `_1M` 2026-09-01 buckets have left the
  window, so the first run after deploy should read 0.
- Cost: the seven reads together take 0.29 s of server time and read 3.7 M
  rows. That is why the check runs as block 5c, before the time-budgeted
  mismatch reads (0203), which stay last.

## Decisions (Adam, 2026-09-28)

1. **Predicate**: band violations **and** priced rows with any price ≤ 0 — the
   second closes the `low = 0, close > 0` gap `zero_invariants` cannot see.
2. **Scope**: all seven tiers, `FINAL`, a 2-day window on bucket time (as
   `zero_invariants`), alarm on `> 0`. The legacy backlog falls outside the
   window by construction and is documented above, not metered.
3. **Clamp**: 0229's read-path clamp stays, with **no** API metric. The stored
   side is now watched at the source with zero tolerance, and the USD crossing
   is a structural ulp between exact `close_usd` and rate-derived extremes.
   ADR-0011 §3 records the confirmation, the as-stored (`QuoteLeg`) arm that
   does not clamp O/H/L, and this baseline.

Research behind the decisions (econometrics, TradFi/crypto/DEX vendors, DQ
frameworks): nobody repairs inconsistent bars; the invariant is kept by
construction and checked with zero tolerance where it is checked at all.

## Acceptance Criteria

- [x] The number of internally inconsistent rows per table is **measured on
      prod** and recorded, before any alarm is designed.
- [x] If the count is non-zero, the cause is identified and filed as its own
      task rather than absorbed here. Band violations are 0. The legacy
      zero-price rows are recorded above and left to [[0286]] phase 3.
- [x] A recurring check exists wherever the other data-quality probes live, with
      its threshold justified by the measured baseline. Block 5c of
      `rollup-freshness-probe`, every 15 min, alarm at >= 1 (baseline 0).
- [x] 0229's clamp is explicitly confirmed as the right behaviour for the read
      path, or changed — with the decision recorded in [[ADR-0011]] §3.
      Confirmed, merged with #396.

## Implementation Notes

- `packages/rollup-freshness-probe/src/ohlc_band.rs`: seven per-tier `FINAL`
  queries, the `band` and `nonpositive` classes, `ohlc_band_totals` (the one
  completeness rule) and the `CandleBandViolations` metric with its two
  refusals (`EmptyScan`, `Incomplete`).
- `main.rs` block 5c: the seven reads run in parallel on a `JoinSet`, each
  under block 6's `mismatch_read_bound`, server- and client-side.
- Observability: `prices-{env}-ohlc-band-{1,100,10000}` on `zeroLadder`.
- ADR-0011 §3: the clamp stays; the as-stored arm does not clamp.

## Deployed to production (2026-10-06)

| step | result |
|---|---|
| merge | #396 → `develop` 81a48f08, 12:43 UTC |
| Observability | 3 alarms created, dashboard 86 alarms, 14:48 CEST |
| EventBridge | only `prices-production-rollup-freshness-probe` changed, 12:57 UTC |
| first run | 13:05:21 UTC: 7 tiers, 0 violations, `checks_failed` 0, 3.1–3.4 s, 51 MB |
| alarms | `ohlc-band-*` INSUFFICIENT_DATA → OK |

## Design Decisions

### From Plan

1. **Predicate, scope, threshold**: Adam's decisions of 2026-09-28 above.

### Emerged

2. **Block 5c, not last**: block 6 (0203) is time-budgeted and stays last.
   The seven reads cost 0.29 s of server time on prod.
3. **Bounded, parallel reads** (review of #396): a stalled tier is a
   recorded failure, not a Lambda timeout that loses block 6 and the log.
4. **One completeness rule** (review): `ohlc_band_totals`, which also
   rejects duplicate or unknown tier names.
5. **Review finding 1 declined**: `pf_trade_count > 0` means priced on every
   tier, because the ingest floor is the rollup floor (0286 WR-03). Prod had
   0 rows that break this from 2026-09-23 to 2026-10-06. A break would be the
   WR-03 defect, so paging on it is right. Documented in the module doc.
6. **The old blocked entry carries `by: ["0286"]`**: the validator requires
   `by`, and the block was waiting for pre-0286 buckets to leave the window.

## Issues Encountered

- **Rebase onto 299 commits of develop**: 0139 (block 5b) and 0203 (block 6,
  time budget, `%reading` log style) had grown the same probe. Four files
  conflicted; the band block was fitted in between.
- **`client()` became async on develop**: it now stops the shared reconcile
  MVs before a test seeds tiers. The 0236 ITs were adapted in c2eeddaf.
