---
id: "0241"
title: "The oracle Lambda OOMs on ~2% of invocations and has done for weeks — 12 Slack pages a day trained everyone to ignore the channel"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0231", "0214", "0056", "0227"]
tags: ["priority-medium", "effort-small", "oracle", "observability", "infra", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../infra/src/lib/stacks/eventbridge-stack.ts"
history:
  - date: 2026-08-31
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0231]]'s loose end 2, which asked whether alarms were
      reaching Slack. They are. Chasing that question instead surfaced this:
      prices-production-oracle has been failing 3-7 invocations a day out of
      ~294 since at least 2026-08-10, with Error Type Runtime.OutOfMemory and
      Max Memory Used 256 MB against a Memory Size of 256 MB. Each failure pages
      twice, which is the 12-messages-a-day the operator saw.
---

# The oracle Lambda OOMs on ~2% of invocations, every day, and nobody acted

## Summary

`prices-production-oracle` fails **3-7 invocations a day out of ~294** — a
steady **~2%** — and has done for at least three weeks. The failure is not
subtle:

```
REPORT RequestId: 919c8acb-…  Duration: 2530.50 ms  Billed Duration: 2531 ms
Memory Size: 256 MB  Max Memory Used: 256 MB  Status: error
Error Type: Runtime.OutOfMemory
```

`Max Memory Used` equals `Memory Size` exactly. The function is being OOM-killed
at its own ceiling.

## Measured — 2026-08-31

Daily `AWS/Lambda` `Errors` for `prices-production-oracle`:

| date | errors | | date | errors |
|---|---|---|---|---|
| 2026-08-10 | 4 | | 2026-08-21 | 6 |
| 2026-08-11 | 3 | | 2026-08-22 | 7 |
| 2026-08-12 | 6 | | 2026-08-23 | 3 |
| **2026-08-13** | **134** | | 2026-08-24 | 1 |
| **2026-08-14** | **267** | | 2026-08-25 | 4 |
| 2026-08-15 | 5 | | 2026-08-26 | 1 |
| 2026-08-16 | 5 | | 2026-08-27 | 6 |
| 2026-08-17 | 4 | | 2026-08-28 | 5 |
| 2026-08-18 | 6 | | 2026-08-29 | 6 |
| 2026-08-19 | 3 | | 2026-08-30 | 6 |
| 2026-08-20 | 3 | | 2026-08-31 (part) | 3 |

Invocations run ~294/day, so the baseline is ~1-2%.

⚠️ The 08-13/08-14 spike (134, 267) is a **different** event — that is the day
BE filled the shared Hetzner disk and stalled ingest for 11.5 h
(`Code: 243`, see [[hetzner-disk-is-shared-and-be-owns-96pct]]). It resolved on
its own. The ~2% baseline sits either side of it, unchanged, and is the subject
of this task.

🔴 **The baseline is not bounded above by these three weeks.** CloudWatch alarm
history for `prices-production-oracle-errors` still shows transitions at the
oldest page examined (2026-08-18), which means the onset is older than the
window measured here. Do not record 2026-08-10 as a start date — it is only the
earliest day *checked*.

## Why nobody acted — this is the [[0214]] pattern again

`prices-production-oracle-errors` is `AWS/Lambda` `Errors`, `Sum`, period 300,
`>= 1`, **evaluation periods 1**, `notBreaching`, wired to
`prices-production-ops-alarms` → Slack.

So **one** failed invocation raises the alarm, and it clears ~5 minutes later.
Each OOM therefore produces **two** Slack messages, and 6 OOMs a day produce
**12**. On 2026-08-30 the channel received exactly 12 — six `OK → ALARM` and six
`ALARM → OK`, all from this one alarm.

That volume is the defect's real cost. [[0214]] recorded an enrichment alarm
that sat in ALARM for 24 days while nobody acted, and the lesson was written as
*the alarm worked, the process did not*. This is the same failure one step
earlier: an alarm so noisy that a genuine page is indistinguishable from the
daily churn. The operator's own reading of the channel on 2026-08-28 — that
alarms had stopped after 14:31 — is downstream of this.

## Implementation

Two separable pieces. Do them in this order; the second is not a substitute for
the first.

1. **Stop the OOM.** `Max Memory Used == Memory Size == 256 MB` says the ceiling
   is the binding constraint, but raising it blind is a guess. Establish first
   *why* a pass that normally completes in ~9-10 s at well under 256 MB
   occasionally does not — the oracle queries 2 symbols and writes 2 rows, so a
   memory profile that occasionally saturates 256 MB is not self-evidently
   proportionate. Check whether the failing passes correlate with a Reflector
   response size, an RPC retry path, or a ClickHouse result set.
   ⚠️ Raising the memory alone would convert a visible 2% failure into an
   invisible one if the underlying growth is unbounded.

2. **Make the alarm actionable.** At `1/1` on a 5-minute period, a single
   transient failure of a job that runs every 5 minutes pages twice. Consider a
   sustained-rate shape (N failures across M periods) so a genuine outage still
   fires fast while a single retryable blip does not. ⚠️ Do **not** simply widen
   it until it is quiet — 0214's lesson is that an ignored alarm and an absent
   one cost the same.

## Acceptance Criteria

- [ ] The cause of the OOM is identified from evidence, not inferred from the
      `Max Memory Used` figure alone.
- [ ] `AWS/Lambda` `Errors` for `prices-production-oracle` is **0 across a full
      day**, measured after the fix.
- [ ] `prices-production-oracle-errors` no longer flaps: no `OK → ALARM →  OK`
      cycle over a 24-hour window with the feed healthy.
- [ ] The alarm still fires when the oracle genuinely fails — **verified by
      inducing**, per [[0204]] and [[0231]], not by reading a green deploy.
- [ ] The true onset date is established from CloudWatch history by paginating
      past the 50-item page, and recorded here.

## Out of scope

- The dark-feed and timestamp-rejection alarms from [[0231]] — they are correct
  and were verified by induction. This task is about a *different* alarm on the
  same function.
- Slack delivery itself. It works; see [[0231]]'s loose end 2.
