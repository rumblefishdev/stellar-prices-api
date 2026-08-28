---
id: "0236"
title: "Nothing detects an internally inconsistent `price_ohlcv_*` row — and 0229's clamp removed the one surface that used to surface them"
type: BUG
status: backlog
related_adr: ["0011"]
related_tasks: ["0229", "0120", "0182", "0227"]
tags: ["priority-medium", "effort-small", "data-correctness", "observability", "ohlcv", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../tools/scripts/conformance-0120.mjs"
history:
  - date: 2026-08-28
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0229]]'s code review, finding 3. Not a defect in that fix —
      a consequence of it that is worth owning explicitly rather than leaving
      implicit in a PR thread.
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

## Acceptance Criteria

- [ ] The number of internally inconsistent rows per table is **measured on
      prod** and recorded, before any alarm is designed.
- [ ] If the count is non-zero, the cause is identified and filed as its own
      task rather than absorbed here.
- [ ] A recurring check exists wherever the other data-quality probes live, with
      its threshold justified by the measured baseline.
- [ ] 0229's clamp is explicitly confirmed as the right behaviour for the read
      path, or changed — with the decision recorded in [[ADR-0011]] §3.
