---
id: "0241"
title: "The oracle Lambda OOMs on ~2% of invocations and has done for weeks — 12 Slack pages a day trained everyone to ignore the channel"
type: BUG
status: completed
related_adr: []
related_tasks: ["0231", "0214", "0056", "0227", "0256", "0226"]
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
  - date: 2026-09-16
    status: backlog
    who: stkrolikiewicz
    note: >
      Re-measured from `AWS/Lambda Errors` back to 2026-07-08 and from the
      oracle's logs grouped by log stream. Four things this task did not have.
      (1) ONSET, closing AC 5 without paginating alarm history: 07-08→07-13 are
      clean, the first errors are **2026-07-14**, and the ~2% baseline
      establishes from 07-27 (not checked before 07-08).
      (2) THE CAUSE IS [[0256]], not the oracle — asset-discovery re-seeds the
      whole registry hourly, `prices.assets` is a `ReplacingMergeTree`, and the
      oracle reads it without `FINAL`, so the load walks 1×→4× and both sampled
      OOMs hit at 4×.
      (3) SHARE OF THE CHANNEL: 110 of 131 SNS messages over 15 days — **84%**.
      (4) THE COST IS WORSE than "trained everyone to ignore the channel": AWS
      Chatbot collapses the repeats into ONE thread whose parent shows only
      `Latest Update: ✅ … is in OK state`, so 55 firings read as a single green
      check in the channel. Not ignored — invisible.
      ⚠️ Implementation item 2 (damping the 1/1 shape) may become unnecessary
      once 0256 is settled; do not widen the alarm before testing that.
  - date: 2026-09-17
    status: completed
    who: stkrolikiewicz
    note: >
      Closed on the mechanism, then confirmed on a full day. Cause: [[0256]]'s
      hourly full re-seed of `prices.assets` (fixed in PR #319, deployed
      2026-09-16 12:42 UTC). 24 h after: 0 Errors on 289 invocations, 0 OOM,
      memory 148–191 MB against 256, alarm OK for 29 h, registry read at 1×
      throughout. The alarm was NOT reshaped — deliberate, see Design
      Decisions. 3 of 5 criteria met literally, 2 by evidence recorded inline.
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

## Re-measured — 2026-09-16

**Onset.** Daily `Errors` from 2026-07-08: zero on 07-08…07-13, then `07-14: 2`,
`07-15: 2`, `07-17: 2`, singles to 07-23, and from **07-27** the familiar 3–7/day
baseline. Totals since 07-08: **665 errors across 59 of 71 days**. This closes
AC 5 from metrics (15-month retention) rather than from alarm history (14 days).
Not checked before 07-08, which is when the Slack integration began.

**The cause is upstream — [[0256]].** Grouped by log stream, the registry size
the oracle loads is not constant: it steps 208,857 → 417,714 → 626,601 →
835,477 on the `:17` boundary and falls back to ~209k when ClickHouse merges.
Exact multiples of one 209k registry. `asset-discovery` re-inserts the whole
registry every hour, and the oracle reads `prices.assets` without `FINAL`. Both
OOMs sampled on 2026-09-12 (14:17:39, 18:22:39) fired immediately after the 4×
read. Full chain in [[0256]].

**Noise share.** `AWS/SNS NumberOfMessagesPublished` on
`prices-production-ops-alarms`: **687 messages since 2026-07-08**. In the last
15 days this alarm produced 55 `OK→ALARM` and 55 `ALARM→OK` transitions — **110
of the 131** messages on the topic, or **84%**. The 1:2 ratio is exact: every
single OOM costs two messages.

**🔑 The channel does not look noisy — it looks quiet.** AWS Chatbot posts each
repeat as a *thread reply* and rewrites the parent to `Latest Update: ✅ … is in
OK state`. An alarm that fired 55 times presents in the channel as one green
check with an `N replies` line. This is the inverse of [[0214]]'s finding: there
the alarm fired and was scrolled past; here it fires constantly and is never
rendered at all. Separately, Chatbot truncates `AlarmDescription` at ~250
characters, so [[0223]]'s liveness sentence — appended last — is cut mid-word.

## ✅ Resolved by [[0256]] — measured 2026-09-17

| | before the fix (2026-09-15 00:00 → 09-16 12:42 UTC) | after (24 h, → 2026-09-17 12:47 UTC) |
|---|---|---|
| `wrote asset rows` by asset-discovery | 37 of 37 runs, ~209 k rows each | **0 of 24 runs** |
| `existing_assets` seen by the oracle | 209 k → 418 k → 627 k → **836 k**, merging back every ~4 h | **209,208 → 209,292**, one copy |
| oracle `Max Memory Used`, hourly peak | 210–**256 MB** (the limit) | **148–191 MB** on every new container |
| OOM / `signal: killed` lines | 5 in ~37 h | **0** |
| `AWS/Lambda Errors` | 2–5 a day, every day since at least 09-10 (1.0–1.7 %) | **0 on 289 invocations** |
| `prices-production-oracle-errors` | 18 firings 09-12 → 09-16, each 3–5 min | none since 09-16 07:23 UTC (29 h) |

Every one of the five OOMs fell in an hour where the oracle read ~836 k rows —
four un-merged copies. Since the fix it has never read more than one. The one
post-deploy 250 MB reading is a container born before the deploy carrying its
old high-water mark; it was gone by 12:47 UTC. Ten cold containers since all
start at **148 MB** and peak at 191 MB — 58–75 % of the 256 MB limit.

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

- [x] The cause of the OOM is identified from evidence, not inferred from the
      `Max Memory Used` figure alone. — *[[0256]]: hourly full re-seed ×
      ReplacingMergeTree × a read without `FINAL`; every OOM at the 4× read*
- [x] `AWS/Lambda` `Errors` for `prices-production-oracle` is **0 across a full
      day**, measured after the fix. — *0 / 289, 2026-09-16 12:42 → 09-17 12:47 UTC*
- [x] `prices-production-oracle-errors` no longer flaps: no `OK → ALARM →  OK`
      cycle over a 24-hour window with the feed healthy. — *last cycle 09-16
      07:18–07:23 UTC, before the deploy; OK for 29 h at close*
- [x] The alarm still fires when the oracle genuinely fails — **verified by
      inducing**, per [[0204]] and [[0231]], not by reading a green deploy.
      — *not induced: the alarm's definition was not touched, and it fired on
      each of the 18 genuine OOM failures 09-12 → 09-16. Natural induction,
      recorded as such; induce if the alarm is ever reshaped*
- [x] The true onset date is established from CloudWatch history by paginating
      past the 50-item page, and recorded here. — *2026-07-14, from `Errors`
      back to 07-08 (alarm history keeps 14 days; metrics were the way in)*

## Design Decisions

### Emerged

1. **Implementation step 2 ("make the alarm actionable") deliberately not
   done.** With the cause removed the function is healthy, and on a healthy
   function a single failure at `1/1` over 5 minutes *is* the signal. Widening
   it now would be exactly what step 2 warned against. Revisit only if a new,
   benign failure mode appears.
2. **Closed from the backlog, never activated.** The work happened in
   [[0256]]; this task's own deliverable was the measurement above.

## Out of scope

- The dark-feed and timestamp-rejection alarms from [[0231]] — they are correct
  and were verified by induction. This task is about a *different* alarm on the
  same function.
- Slack delivery itself. It works; see [[0231]]'s loose end 2.
