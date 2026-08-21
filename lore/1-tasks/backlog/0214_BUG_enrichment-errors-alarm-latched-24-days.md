---
id: "0214"
title: "prices-production-enrichment-errors has been in ALARM for 24 days and nobody acted — the alarm worked, the process did not"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0204", "0209", "0212", "0026", "0111"]
tags: ["priority-high", "effort-small", "observability", "enrichment", "ops", "milestone-M2"]
milestone: 2
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-20
    status: backlog
    who: okarcz
    note: >
      Spawned from 0204 while reading the full alarm table during its deploy
      verification. prices-production-enrichment-errors has been latched in
      ALARM since 2026-07-27T00:20:07Z — 24 days — while every sibling
      enrichment alarm reads OK. Not caused by 0204 and not related to its
      changes; found because 0204's verification listed every alarm in the
      account rather than only its own.
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      ⬆️ PRIORITY EVIDENCE — this alarm's latch has been hiding a CONTINUOUS
      failure, not a stale one-off. CloudWatch shows the enrichment pass erroring
      with `Clickhouse(BadResponse(""))` on EVERY invocation, 3x/hour (one
      trigger plus two Lambda async retries), across the full 48 h window checked
      on 2026-08-21. It has not completed successfully in at least two days and
      no page was raised. ⚠️ It is also invisible from the data: ClickHouse
      finishes the abandoned statement server-side and logs QueryFinish, so the
      rows land and every data-level signal reads normal. This alarm is the ONLY
      thing that could have caught it. See 0215 for the timeline and 0111 for the
      throughput consequence.
---

# An enrichment alarm has been in ALARM for 24 days

## Summary

```text
prices-production-enrichment-errors    ALARM    2026-07-27T00:20:07+00:00
```

It fired on **2026-07-27** and has been in `ALARM` ever since. Nobody acted on
it. It surfaced only because [[0204]]'s deploy verification listed *every*
alarm in the account instead of just its own.

⚠️ **Two separate problems, and the second is the more important one.**

## Problem 1 — whatever the errors are

The alarm watches `AWS/Lambda Errors` on the enrichment worker. Its declared
impact, from `addWorkerHealthAlarms`, is:

> USD columns (`close_usd` / `volume_quote_usd`) stop being filled in, so new
> candles serve as 0 to the API.

Every sibling alarm reads `OK`: `enrichment-backlog`,
`enrichment-duration-near-timeout`, `enrichment-no-invocations`. So the worker
**is** being invoked, is **not** timing out, and its backlog gauge is not
tripping — while it throws errors. Unknown whether that is every invocation or
an occasional one; a latched alarm cannot tell you.

⚠️ **Do not assume this is [[0209]].** That defect (the USDT pivot has never
priced a `price_ohlcv_1m` row) starts **2026-08-13**, seventeen days *after*
this alarm latched, and it is a silent no-op rather than an error. They are
probably distinct. But they are the same worker, and 0209 proves this worker is
capable of failing in ways nothing was watching — so check whether the errors
predate, coincide with, or are unrelated to it before concluding anything.

## Problem 2 — 24 days is the real finding

[[0204]] exists because the 2026-08-13 outage was found by reading Lambda panic
logs after the fact, and because the DLQ alarm *"fires once and never
re-notifies"*. This is the same failure with the alarm working perfectly: it
fired, it routed to Slack, and it was then scrolled past for three and a half
weeks.

⚠️ **This is direct evidence against 0204 gap 3's design decision 1.** That
decision accepted a single latched alarm on MV drift, on the argument that
latching costs *"somebody may forget"* rather than *"we are blind to an
escalation"* — a cheap failure, and *"one a ticket closes"*. This alarm is the
measurement of what that costs in practice: **24 days, and only found by
accident.**

That does not automatically make the decision wrong — drift really does not
deteriorate the way a filling DLQ does — but it prices the trade honestly, and
it should be re-read before the next alarm is designed to latch.

## Implementation

- Read the worker's error logs from 2026-07-26 onward and classify: continuous
  or intermittent, one error or several, and what changed on 2026-07-27.
- Establish whether the errors have any relationship to [[0209]] or to
  [[0111]]'s 657M-row backlog (the pass hands off that many candidates every
  invocation).
- Fix, or record why the errors are acceptable and re-tune the alarm so it stops
  asserting something nobody acts on. ⛔ An alarm that is permanently in ALARM
  and permanently ignored is worse than no alarm — it trains people to scroll.
- ⚠️ Separately, propose how a latched alarm gets re-surfaced. A periodic digest
  of *alarms currently in ALARM* is the cheapest option and would have caught
  this on day two. That question is bigger than this task; record the proposal
  and spawn it if it needs its own work.

## Acceptance Criteria

- [ ] The cause of the errors is **measured** from logs, not inferred, with the
      first occurrence dated and the frequency stated.
- [ ] Either the errors stop, or the alarm is re-tuned to something actionable
      and the reasoning is written down.
- [ ] The relationship to [[0209]] is stated explicitly — related or not — so
      the next person does not re-derive it.
- [ ] ⚠️ A mechanism exists that would surface a latched alarm within a day.
      Without this the same thing happens again, and [[0204]] gap 3 has two
      alarms deliberately designed to latch.
