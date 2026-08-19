---
id: "0209"
title: "USDT-quoted candles sit at close_usd = 0 for 17 h+ after the hourly sweep should have priced them"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0182", "0172", "0165", "0145"]
tags: ["priority-medium", "effort-small", "clickhouse", "enrichment", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-19
    status: backlog
    who: okarcz
    note: >
      Spawned from 0182. Its post-repair damage check found 82 USDT-quoted rows
      still at close_usd = 0 across four tiers. All of them postdate the repair
      and none are damage from it, but the oldest is 17 h+ old, which is longer
      than an hourly sweep should leave a candle unpriced. Never investigated —
      0182 closed on the repair being verified, not on this.
  - date: 2026-08-19
    status: backlog
    who: okarcz
    note: >
      MEASURED while validating 0204's gap-4 alarm thresholds against prod. The
      first acceptance criterion is met: the cause is USDT-SPECIFIC, not the
      sweep. On 2026-08-18 the USDC leg ran 759 priced / 8 unpriced and the XLM
      leg 3,446 / 49 — both healthy — while the USDT leg ran 8 / 12. Enrichment
      is alive. Candidate 2 is the direction, refined by two further findings:
      prices.usd_rate holds ONLY USDC, so USDT has no direct rate and must be
      derived, and the USDT/USDC reference market carries exactly ONE candle per
      day. Still backlog — the lever is chosen but not built, and this now also
      blocks a threshold decision on 0204.
---

# USDT-quoted candles stay unpriced far longer than the sweep interval

## Summary

Measured 2026-08-19 while verifying [[0182]]'s repair. 82 USDT-quoted candles
(`quote_asset_id = 111`) sit at `close_usd = 0` with a real `close` and real
volume, across four granularities:

| tier | rows | window |
|---|---|---|
| `_1h` | 38 | `2026-08-18 14:00` → `2026-08-19 07:00`, 9 assets |
| `_4h` | 22 | same window |
| `_1d` | 14 | yesterday + today |
| `_1w` | 8 | the current week bucket, `2026-08-17 00:00` |

These are **not** [[0182]]'s stranded rows — that defect was at the 2021-02-07
epoch boundary and is repaired, `at_boundary` 0 on every tier. These postdate the
run entirely. They are also **not dust**: `close` ranges from `0.045` to `524.78`
with ordinary `volume_quote`.

The problem is only the age. An in-flight bucket at zero is expected; a bucket
from **17 hours ago** is not, if an hourly sweep is running.

## Two candidate causes — ✅ RESOLVED 2026-08-19, it is candidate 2

1. **The sweep is not keeping up, or is not running.** Would show as a lag
   affecting all quote legs, not just USDT.
2. **The pivot cannot find a USDT/USDC reference inside its window.** Live
   enrichment prices USDT-quoted candles by pivoting off the measured USDT/USDC
   market ([[0172]]). The default `--pivot-window-s` is **1 day**; if that market
   is thin enough that a bucket has no reference at-or-before within the window,
   the `ASOF LEFT JOIN` + `AND r.usd IS NOT NULL` drops the row and it stays at
   zero until a later trade rescues it. Would show as USDT-specific.

⚠️ **Distinguishing them is the first task, and it is one query** — compare the
unpriced-row age distribution for `quote_asset_id = 111` against XLM-quoted and
USDC-quoted legs over the same window. If only USDT lags, it is the reference
window; if everything lags, it is the sweep.

## Measured 2026-08-19 — USDT-specific, and the mechanism is a two-hop chain

Run against prod while validating [[0204]] gap 4's alarm thresholds, which is why
these numbers exist at all: that alarm's rung 1 turned out to be untunable
without answering this task first, so the two are now coupled.

⚠️ **Two wrong conclusions preceded the right one and are recorded so they are
not re-derived.** From the roster alone it looked *asset-specific* (a near-identical
set of 8 assets failing two days running); from a single asset's history it looked
like *enrichment had stopped* (four assets priced cleanly 08-06 → 08-17 then went
to zero together on 08-18, the day [[0182]]'s repair ran). Both readings survive
their own evidence and are false. **Only the by-leg split settles it.**

### The discriminating query and its answer

```sql
SELECT toDate(c.timestamp) AS day, q.asset_code AS quote,
       countIf(c.close_usd > 0) AS priced, countIf(c.close_usd = 0) AS unpriced
FROM (SELECT timestamp, quote_asset_id, close_usd FROM prices.price_ohlcv_1d FINAL
      WHERE timestamp >= now() - INTERVAL 5 DAY AND close > 0) AS c
INNER JOIN (SELECT asset_id, asset_code FROM prices.assets FINAL) AS q
        ON q.asset_id = c.quote_asset_id
WHERE q.asset_code IN ('USDT', 'USDC', 'XLM')
GROUP BY day, quote ORDER BY quote, day;
```

| 2026-08-18 | priced | unpriced |
|---|---|---|
| USDC leg | 759 | 8 |
| XLM leg | 3,446 | 49 |
| **USDT leg** | **8** | **12** |

**Enrichment is alive.** Two legs healthy on the same day in the same table rules
out candidate 1 completely. (Today's elevated unpriced counts on every leg are
just the current day still being worked, and are not evidence of anything.)

⚠️ This query matches USDT by **code only**, so it sweeps in other issuers' USDT
tokens — which is why it shows 6 unpriced on 08-15 where the issuer-pinned query
shows 0. Those extras are genuinely unpriceable and unrelated. Pin the issuer
(`GCQTGZQQ…`) when the canonical leg is what you mean.

### Two further readings that refine candidate 2

- **`prices.usd_rate` holds ONLY USDC** — 1,440 rows in 5 days, fresh to
  `2026-08-19 16:30`. USDT has **no direct rate at all**, so its value is always
  derived. ⚠️ A query looking for a USDT row in `usd_rate` returns empty and that
  is **normal**, not a finding — an empty result there proves nothing.
- **The USDT/USDC reference market carries exactly ONE candle per day** on `_1d`,
  present every day, priced through 08-18 and not yet for 08-19. That is the
  thinness candidate 2 predicts, now measured rather than assumed.

### The mechanism

`usd_rate(USDC)` → the USDT/USDC reference candle's `close_usd` → every
USDT-quoted candle.

A USDT-quoted candle cannot be priced until its own day's reference has been
priced first, so **the USDT leg is structurally one hop behind every other leg**.
USDC and XLM are one hop shorter, which is exactly why they do not show this.

⚠️ **This is a steady state, not a degradation.** At any moment the most recent
day-and-a-bit of USDT-quoted candles is unpriced and everything older is filled —
the same query on 08-17 would have shown 08-16 and 08-17 dark. The "17 h+" in this
task's title is not an incident; it is the leg's normal latency.

⚠️ **The residual, left open honestly:** on 08-18 the reference *was* priced, yet
only half that day's dependants filled. The chain explains 08-19 completely and
08-18 only partially. Do not treat it as fully proven — the cheap confirmation is
to re-read the per-day counts after **2026-08-20 00:00**: if 08-18's eight have
filled, the chain holds; if they are still zero past 48 h, something is stuck and
this is a different problem.

### The latency is ~30 hours, measured — and that supersedes this task's title

Bucketing every USDT-quoted `_1h` candle by age (prod, 2026-08-19):

| age band | priced | unpriced |
|---|---|---|
| 0-24 h | 0 | 74 |
| 24-30 h | 9 | 12 |
| **30 h → 162 h** | **all** | **0** |

**Every band from 30 h out to 162 h is 100% priced.** So the ceiling is ~30 h and
it is *stable*, not widening — which answers the question this task's "17 h+"
title left open. That 17 h was a single observation from [[0182]]'s damage check;
the distribution supersedes it, and the honest reading is that ~30 h is this
leg's **normal** latency rather than evidence of a fault.

⚠️ **This does not make the task go away, it re-scopes it.** The USDT leg is
structurally ~30 h behind every other leg because of the two-hop chain, on a
market thin enough to produce 9-18 hourly reference buckets a day. That is worth
shortening — BE's window is 48 h, so we run at ~60% of a consumer's tolerance
with no margin for a bad day — but it is a *latency* problem, not a *correctness*
one, and nothing is currently being lost.

⚠️ Also measured while resolving this: the USDT/USDC reference **does** exist
hourly (9-18 buckets/day, essentially all priced). Candidate 2's "the pivot
cannot find a reference inside its window" is therefore NOT the mechanism as
stated — the reference is there. What costs the time is the ordering: the
reference must be *enriched* before its dependants can be.

### ✅ It no longer blocks [[0204]] — resolved 2026-08-19 by moving the tier

Gap 4's stranded alarm has a 48 h grace, chosen because it is the window BE
actually read. The collision it appeared to have with this task's latency was
**real on `price_ohlcv_1d` and absent on `price_ohlcv_1h`**, because a bucket's
`timestamp` is its START — a daily candle burns 24 of the 48 grace hours before
its data even exists, an hourly one burns one. 0204's check moved to `_1h`; the
grace and the rung are unchanged.

⚠️ **The link is not severed, it is slack.** 0204's alarm now sits ~18 h above
this task's measured ceiling. If that ceiling widens past ~36 h the alarm starts
firing on ordinary operation again, so **this task's fix is what protects that
margin** — and any future reading here should be a fresh age distribution, not a
single old row's age.

## Why it matters despite being small

BE's pool-list TVL takes the last `close_usd > 0` **within 48 hours** and renders
`--` when there is none ([[0182]], BE answers 2026-08-13). 82 rows is nothing, but
a 17-hour hole consumes a third of that margin, and the failure mode is silent on
our side — a zero is indistinguishable from "not yet written" at every one of the
~130 unguarded `argMax(close_usd, …)` sites ([[0145]]).

If cause 2 is the answer, the hole grows with USDT market thinness rather than
staying bounded, which is the shape that eventually crosses 48 h.

## Implementation

- Measure first: age distribution of `close_usd = 0 AND close > 5e-14` rows by
  quote leg, over the last 7 days, on `_1h`.
- If USDT-specific → the reference window is the lever. Decide whether the live
  pass should widen `pivot_window_s` for this leg, or whether a stale reference
  is *better* than no value here (⚠️ that is the [[0165]] peg-fallback argument
  and it was settled against a fixed peg, not against a stale measurement — do
  not reopen it as "just use $1").
- If leg-agnostic → the sweep's schedule or batch budget is the lever, and this
  becomes an [[0111]]-adjacent throughput question.

## Acceptance Criteria

- [x] The cause is **measured**, not inferred — USDT-specific or leg-agnostic,
      with the comparison query recorded. ✅ **2026-08-19 — USDT-SPECIFIC.**
      Query and results in "Measured 2026-08-19" below. Candidate 2 (the
      reference path), not candidate 1 (the sweep).
- [ ] Whichever it is, the fix is verified by the age distribution moving, not by
      the row count on one day.
- [ ] If the pivot window is widened, a note records why a stale measured
      reference is acceptable where a $1 peg was not.
