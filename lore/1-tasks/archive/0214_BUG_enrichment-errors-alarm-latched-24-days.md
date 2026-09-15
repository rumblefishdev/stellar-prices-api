---
id: "0214"
title: "prices-production-enrichment-errors has been in ALARM for 24 days and nobody acted — the alarm worked, the process did not"
type: BUG
status: completed
related_adr: []
related_tasks: ["0204", "0209", "0212", "0026", "0111", "0214", "0215", "0223", "0226", "0243", "0283"]
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
  - date: 2026-09-15
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Item 10 of Oskar's backlog summary, taken after [[0243]]
      shipped; [[0223]] is the other half and follows this one rather than
      sharing its branch. ⚠️ Problem 1 is already over and this task's first
      three criteria are now a write-up, not an investigation: the errors ended
      on 2026-08-24 when [[0111]] deployed, and the alarm has read OK since,
      apart from the deliberate 0215 induction on 2026-09-11. Problem 2 — a
      latched or noisy alarm that nobody re-surfaces — is untouched, and the
      live instance today is prices-production-oracle-errors: 62 state changes
      in 7 days, each one a Slack message, measured 2026-09-15.
  - date: 2026-09-15
    status: completed
    who: stkrolikiewicz
    note: >
      All four criteria closed. The first three were a write-up of a defect that
      [[0111]] had already ended; the fourth was the actual work — a daily
      re-read of every prices-{env}- alarm, riding the mTLS probe's existing
      rate(1 day) rather than becoming a tenth Lambda. PR #316, merge 5018e21,
      deployed 12:22:20 UTC. 17 unit tests (+7 new), 4 review findings from
      okarcz applied, 2 real publishes to the ops channel to settle the message
      format. ⚠️ Two things a later session must not re-derive: the ops topic's
      ONLY subscriber is Chatbot, which drops unrecognised payloads silently and
      logs nothing, so the envelope could only be settled by publishing for real;
      and `sns:Publish` from inside the Lambda is still unexercised on production
      because 0 of 52 alarms were off OK — deferred to [[0283]] part B, which
      induces it with a throwaway alarm rather than falsifying a real one. That
      gap is acceptable only because a failed publish fails the invocation and
      pages, so it cannot reproduce the silence this task is about.
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

- [x] The cause of the errors is **measured** from logs, not inferred, with the
      first occurrence dated and the frequency stated. See the measurement below.
- [x] Either the errors stop, or the alarm is re-tuned to something actionable
      and the reasoning is written down. **They stopped**, on 2026-08-24, and the
      alarm was left exactly as it is — see below for why no re-tune is owed.
- [x] The relationship to [[0209]] is stated explicitly — related or not — so
      the next person does not re-derive it. Same worker, different symptom,
      different fix; below.
- [x] ⚠️ A mechanism exists that would surface a latched alarm within a day.
      Without this the same thing happens again, and [[0204]] gap 3 has two
      alarms deliberately designed to latch. **Shipped and deployed 2026-09-15**
      — see below. One link is deferred to [[0283]] part B and named explicitly.

## Measured 2026-09-15 — problem 1 is over, and here is the arithmetic

CloudWatch, `AWS/Lambda Errors` on `prices-production-enrichment`, daily sums:

| day | errors |
|---|---|
| 2026-08-16 … 08-23 | **72 every single day** |
| 2026-08-24 | 24, then nothing |
| 2026-08-25 … 09-10 | 0 |
| 2026-09-11 | 1 |

72 a day is exactly 3 an hour — one scheduled trigger plus two Lambda async
retries, on an hourly worker. So [[0214]]'s 2026-08-21 reading ("every
invocation, 3x/hour") held for at least the nine days the 30-day metric window
still shows, and the alarm's own history dates the start at 2026-07-27T00:20:07Z:
**28 days of an every-invocation failure behind one latched alarm.**

**What ended it: [[0111]], not a fix aimed at this task.** 0111 deployed
2026-08-24 **08:03:24 UTC** (partition-bounded enrichment passes) and the alarm
went ALARM → OK at **08:31:48 UTC**, 28 minutes later. The 24 errors on 08-24 are
the eight hours before that deploy. Nothing has errored since, apart from the
single error on 2026-09-11, which is [[0215]]'s deliberate 12:57 UTC induction of
the execution bound — the alarm fired for it at 14:58 CEST and cleared an hour
later, which is the behaviour we want.

**Why the alarm is not re-tuned.** The criterion offers "either the errors stop
or the alarm is re-tuned". They stopped, so the alarm is correct as written: it
fired on a real, continuous failure and it cleared when the failure ended. The
defect was never the alarm's sensitivity — it was that nobody re-read it for 28
days. Re-tuning would have hidden a true positive.

**Relationship to [[0209]], stated so nobody re-derives it.** Same worker, two
different symptoms, two different fixes. The errors here are
`Clickhouse(BadResponse(""))` from a statement cut at the proxy, ended by 0111's
partition bound and structurally removed by [[0215]]'s execution bound (with
[[0281]] making the empty error body readable at last). 0209 — the USDT pivot
never pricing a `price_ohlcv_1m` row — was a silent no-op with no error at all,
starting 2026-08-13, and was closed by 0215. Neither caused the other; both were
invisible for the same reason.

### The live instance of problem 2, measured the same day

`prices-production-oracle-errors` changed state **62 times in 7 days** (about 31
ALARM/OK pairs), and every transition posts to the ops Slack channel because
`createWorkerLambda` wires an OK action as well ([[0112]]). Two of those pairs
landed this morning, at 02:13 and 06:35 UTC. That is the same failure as the
latch, from the other end: a channel nobody can read is a channel nobody reads.
The noise half belongs to [[0223]] and [[0226]]; this task owns the "nobody
re-surfaced it" half.

## The mechanism — a daily re-read, shipped 2026-09-15

PR [#316](https://github.com/rumblefishdev/stellar-prices-api/pull/316), merged
`5018e21`, deployed to production 12:22:20 UTC.

Once a day the `mtls-notafter-probe` reads every `prices-{env}-` alarm and
publishes one message naming those off OK for over an hour, oldest first. It is
silent when the account is healthy.

### Design decisions

#### From plan

1. **It rides the mTLS probe rather than becoming a tenth Lambda.** That probe
   already runs on `rate(1 day)`, which is exactly the cadence this wants. The
   two jobs share nothing but the trigger, so the digest collects its own failure
   the way the rollup probe collects one per check.
2. **Silent when healthy.** A daily "all good" would be new noise on the very
   channel this task exists to keep readable.
3. **A one-hour floor (`MIN_STUCK_SECONDS`).** Against a once-a-day snapshot,
   anything lower reports flaps as zators — `prices-production-oracle-errors`
   changed state 62 times in the week measured above, and reprinting those would
   defeat the point. The 2026-07-27 latch would have been caught on its first
   digest either way. Flap volume belongs to [[0223]] and [[0226]].
4. **`INSUFFICIENT_DATA` counts as off OK.** A publisher that dies leaves its
   alarm there, not in ALARM, and nothing else re-surfaces that state.

#### Emerged

5. **The payload is AWS Chatbot's custom-notification envelope, not a string.**
   Found while reading the topic: its only subscriber is Chatbot, which forwards
   only payloads it recognises and drops the rest **silently**. There is no email
   subscriber to catch the difference and — since our config logs at `ERROR` and
   a silent drop is not an error — no log either. ⚠️ So this could not be settled
   by reading config; it took a real publish to the real channel, verified
   2026-09-15 13:45 CEST (`MessageId 641a8633-…`, SNS `Delivered=1, Failed=0`).
6. **Console deep links instead of a code fence.** A second real publish
   (`MessageId 20750a69-…`) confirmed Chatbot renders Slack `<url|text>`. The
   ``` fence had to go, since links render as literal text inside one — trading
   column alignment for one-click access. `each_alarm_is_one_click_from_its_console_page`
   pins the exact URL **and** asserts no fence, so a later tidy-up that restores
   alignment cannot silently kill the links.
7. **`StateTransitionedTimestamp`, not `StateUpdatedTimestamp`** — from review.
   The latter also moves when `EvaluationState` changes, so a transient
   `PARTIAL_DATA` flip on a 28-day latch would drop that alarm from the digest or
   print `2h 13m` for it: this task's own bug, one field over. Currently a no-op
   — all 52 production alarms have the two equal — which is exactly why it had to
   be reasoned from the docs rather than measured.
8. **`asyncRetryAttempts: 0` on the probe** — from review. Lambda's default of 2
   was harmless while the handler only did idempotent `PutMetricData`; the digest
   `sns:Publish`es before the handler can fail on an unreadable cert, so a day
   with both would post the identical digest three times. Retries bought nothing
   for alarming anyway: the error alarm is threshold 1 over 1 period.
9. **Zero matched alarms is an error, not `Ok(0)`.** An unset `ENV_NAME` yields
   the prefix `prices-unknown-`, which matches nothing and would read as a
   healthy account — the same silent no-op this task is about. Went further than
   the reviewer's suggestion of logging it.
10. **`cloudwatch:DescribeAlarms` granted on `*`**, though an `alarm:prices-{env}-*`
    ARN simulates as allowed. It is a LIST call, where a resource-scoped grant is
    the kind that passes the simulator and denies at runtime; a denial fails the
    probe and pages, which is worse than what the scope buys — hiding other
    teams' alarm *names* from a read-only Lambda in our own account.

### Verified on production, 2026-09-15

| link | evidence |
|---|---|
| deploy | `✅ Prices-production-EventBridge`, 74.7 s, function `LastModified 12:22:20Z` |
| config | `OPS_ALARMS_TOPIC_ARN` set; `MaximumRetryAttempts: 0` on both the EventInvokeConfig and the rule target |
| alarm read | probe log `{"message":"alarm digest read","prefix":"prices-production-","matched":52}` |
| zero-match guard | passed (52 > 0) |
| decision to stay silent | response `"stuck_alarms": 0`; SNS `NumberOfMessagesPublished = 0` over the window |
| cert half still works | 281 days for both roles, 920 ms, no error |
| Chatbot renders the envelope | real publish, screenshot on the channel |
| Chatbot renders the links | real publish, name clickable to the console |
| `sns:Publish` from the Lambda role | `simulate-principal-policy` on the real role → `allowed` |

⚠️ **What is NOT proven, stated plainly.** With 0 of 52 alarms off OK the
publishing branch never executed, so `sns:Publish` **from inside the Lambda** is
unexercised on production; the two verification messages were published by the
admin principal, not by the function's role. Deferred to [[0283]] part B, which
induces it with a throwaway `prices-production-tmp-…` alarm rather than falsifying
a real one.

This is [[0218]]'s **"test-covered, not prod-induced"** standard, the same one
[[0243]] closed under a day earlier. It is acceptable here for a specific reason:
**a broken publish path cannot go quiet.** The digest failure lands in `problems`
and fails the invocation, which trips the probe's ops-wired error alarm — so the
one failure mode this task exists to prevent is the one this gap cannot produce.

## Future Work

- [[0283]] part B — induce the digest on production and close the gap above.
- Flap volume on the ops channel ([[0223]], [[0226]]) — the other half of
  "a channel nobody can read is a channel nobody reads". Not this task.
