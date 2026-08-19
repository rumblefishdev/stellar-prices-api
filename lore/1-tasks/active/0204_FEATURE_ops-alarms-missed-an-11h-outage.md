---
id: "0204"
title: "Ops alarms missed an 11.5 h outage — no free-space alarm on the shared CH volume, and the DLQ alarm fires once then goes quiet"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0202", "0203", "0137", "0142", "0056", "0201", "0182", "0172", "0196"]
tags:
  ["priority-high", "effort-small", "observability", "alarms", "resilience", "milestone-M2"]
milestone: 2
links:
  - "../../../apps/infra/src"
history:
  - date: 2026-08-14
    status: backlog
    who: okarcz
    note: >
      Spawned from 0202. The 2026-08-13 disk-full stall ran 11.5 h and was
      discovered from three Lambdas panicking, not from any alarm that watches
      the actual condition. Two concrete gaps, both cheap to close.
  - date: 2026-08-17
    status: backlog
    who: okarcz
    note: >
      Added gap 3 — [[0142]] shipped `prices-clickhouse-drift` but nothing runs
      it, and it watches a condition no existing alarm sees (0137 measures
      whether the rollups produce data; a drifted MV produces data perfectly
      well while producing the wrong numbers). Recorded the connection
      constraint that decides where it can run, and the three design traps:
      drift is a standing condition not an event, exit 1 carries two different
      severities, and all-six-MISSING is a grant gap rather than an outage.
  - date: 2026-08-17
    status: active
    who: okarcz
    note: >
      Activated for **gap 1 only** — the free-space alarm — ahead of the
      0201/0182 repair campaign scheduled for the morning of 2026-08-18. This
      task's own gap-1 note makes it a precondition for that run ("it should not
      start without this alarm in place and a word with BE").
      ⚠️ The premise of that warning has moved and the task text is now stale:
      it was written at ~91% used, and the volume measured **430.6 GiB free /
      75.5% used** on 2026-08-17. So the campaign has headroom it did not have,
      and the alarm is no longer "the run cannot start" but "the run is
      unwatched". Still worth having before the morning, because the failure
      mode is BE filling the volume independently — which is exactly what
      happened on 2026-08-13 and is not something we control.
      Gaps 2 (DLQ re-notify) and 3 (scheduled drift check) are NOT in this
      activation; they have no bearing on the campaign.
  - date: 2026-08-19
    status: active
    who: okarcz
    note: >
      Added gap 4, absorbed from 0182's last open acceptance criterion — a
      data-level check that close_usd is RIGHT, not merely present. close_usd
      has been wrong on prod through three different doors (0172's peg, 0196's
      oracle mis-attribution, 0182's epoch boundary) and two of the three never
      touch the writer, so 0172's writer tests cannot be the guard. Same
      category as gap 3 rather than gaps 1-2: correctness, not liveness — gap
      3's own sentence about a drifted MV producing data perfectly well while
      producing wrong numbers transfers directly. Lands on
      rollup-freshness-probe alongside gap 1, which has already paid the
      aws-mtls wiring that gap 3 identifies as the blocker for anything
      AWS-side, and keeps it off eventbridge-stack and its CleanupRule. Checks
      both directions because 0182's own repair caused the inverse failure on
      2026-08-19 — 157 candles zeroed with nothing to refill them. Three traps
      recorded: it is a standing condition so the alarm must not latch; it must
      be scoped to the quote leg because exotic-quoted zeros are by design
      (~74M on _1h alone); and the pre-epoch par window is legitimately ~1.0 so
      the ratio check must be bounded to the post-break era.
---

# Ops alarms missed an 11.5 h outage

## Summary

The 2026-08-13 stall ([[0202]]) was found by reading Lambda panic logs after the
fact. Every alarm that fired was a **downstream symptom**; nothing watched the
condition itself, and the one alarm that returned to OK did so for the wrong
reason. Three gaps, all small.

## Gap 1 — no free-space alarm on the ClickHouse host

`system.disks` had the answer the entire time. We learned about a full disk from
`asset-discovery`, `supply` and `ledger-processor` failing.

⚠️ **This matters more here than on a dedicated host: the volume is SHARED with
BE and we are 3.3% of it** (58.93 GiB of 1.72 TiB; BE's `default` is 951 GiB).
We cannot control what fills it and cannot free meaningful space ourselves — so
**warning time is the only lever we have**. It sat at 91.4% used after recovery,
meaning the next comparable event repeats this.

- Alarm on free space with enough headroom to act (the incident consumed ~150
  GiB, so a threshold at ~15-20% free would have given hours of warning).
- ⚠️ **[[0201]] writes to this volume for 10-15 h.** It should not start without
  this alarm in place and a word with BE.

## Gap 2 — the DLQ alarm fires once and never re-notifies

Slack showed `ApproximateNumberOfMessagesVisible >= 1`. By morning the DLQ held
**91**. Nobody reading Slack could tell 1 from 91.

- Re-notify on growth, or alarm on a rate/threshold ladder rather than a single
  `>= 1` edge.

## ⚠️ And the recovery signal was actively misleading

The lag alarm returned to **OK** at 07:56 — truthfully, the queue *was* empty.
But it emptied partly by messages **being given up on**, not processed: the age
series eased (26,155 → 26,117 → 25,969) exactly as the DLQ filled.

**An empty queue is not a processed queue.** Recovery must be verified on the
**data** (`max(timestamp)` on `price_ohlcv_1m`), never on alarm state — the same
lesson [[0137]] already records for the rollup alarm, arriving through a new
door.

⚠️ Note that even the data check is insufficient alone: on 2026-08-13
`max(timestamp)` was 63 s behind while **eight hourly buckets were missing**. A
completeness signal is [[0203]]'s scope; this task covers the disk and the DLQ.

## Gap 3 — the rollup drift check is manual and nothing runs it

[[0142]] built `prices-clickhouse-drift`: read-only, exits 0 when all six MVs
match `rollups.sql` and are `APPEND`, 1 otherwise. **Nothing runs it.** A check
nobody runs is a check that does not exist, and this one covers a condition no
other alarm sees — [[0137]] watches whether the rollups are *producing data*,
which a drifted MV does perfectly well while producing the wrong numbers.

Cheapest home is a cron on the Hetzner host against `localhost:8123`, which
sidesteps the connection problem entirely: prod's HTTP endpoint is mTLS-only
behind Caddy and `prices_clickhouse::client()` builds a plaintext client, so
anything running from AWS needs the crate's `aws-mtls` feature wiring first.
Folding it into 0137's Lambda is the tidier end state and the more expensive one.

Three things this task's own findings say to design in:

- ⚠️ **Drift is a standing condition, not an event.** It stays wrong until a
  person fixes it, so a single-edge alarm is the gap 2 failure again — the DLQ
  alarm that fired once while the queue grew to 91.
- ⚠️ **Exit 1 is not one severity.** `CRITICAL` (an MV that lost `APPEND`) means
  history is being destroyed on every refresh and should page; `DRIFT` means
  someone's edit silently did not land and can wait for morning. The severity is
  in the output, not the status code — an alarm that reads only the exit code
  throws that distinction away.
- ⚠️ **All six reporting `MISSING` is a grant gap, not six dead tiers.**
  `system.tables` is filtered by grant. The binary prints a note when it sees
  that shape; an alarm must not page as if the chain were gone.

## Gap 4 — nothing watches whether the USD values are RIGHT

Added 2026-08-19 from [[0182]], which needed a "guard against re-introduction"
and found this task already holds the shape of it.

`close_usd` has now been wrong on prod through **three different doors**, and no
alarm saw any of them:

| how it broke | task | what the writer test would have caught |
|---|---|---|
| USDT priced at a $1 peg it no longer held | [[0172]] | ✅ the writer |
| Reflector rows mis-attributed to the USDT identity | [[0196]], and [[0168]] before it | ❌ never touches the writer |
| a reset epoch 19 h below its pivot reference | [[0182]] | ❌ never touches the writer |

Two of the three bypass the writer entirely, so a unit test on the writer — which
[[0172]] already has — cannot be the guard. **The condition is in the data, so
the check has to be in the data.**

This is **the same category as gap 3, not the same as gaps 1-2.** Gaps 1-2 and
[[0137]] watch liveness and capacity: is data arriving, is there disk, is the
queue growing. Gap 3 and gap 4 watch correctness — and gap 3's own sentence
transfers with one noun changed: *0137 measures whether the rollups produce data;
a drifted MV produces data perfectly well while producing the wrong numbers.* A
USDT candle valued at par produces data perfectly well too.

### What it asserts — both directions, and the second one is new

- **No USDT-quoted candle carries `close_usd / close ≈ 1.0`.** The original
  defect: the peg re-applied, by any writer.
- **No USDT-quoted candle sits at `close_usd = 0` with a representable `close`.**
  ⚠️ Added because [[0182]]'s repair *caused* this on 2026-08-19 — 157 candles
  zeroed with nothing to refill them. The inverse failure is as real as the
  original, and an asymmetric check would have passed while the damage stood.
  The `close` bound is the arithmetic one (`rate × close` rounding to zero at
  `Decimal(38, 14)`, i.e. ~`5e-14`), **not** a round number that looks small —
  0182's runbook records a first attempt at `1e-11` that counted dust which had
  priced perfectly well.

### Where it runs

`packages/rollup-freshness-probe`, alongside gap 1. Same reasoning as gap 1, and
the three costs are already paid there: the `aws-mtls` wiring that gap 3's note
identifies as the blocker for anything AWS-side, an EventBridge schedule and a
`PutMetricData` grant scoped by `cloudwatch:namespace`, and dead-probe cover from
`addWorkerHealthAlarms`. The crate is already split so the query construction and
the metric shaping are unit-testable without the AWS SDK.

⚠️ **Reusing `Prices/Rollup` keeps this off `eventbridge-stack`** — the same
reason gap 1 landed there. That stack owns `CleanupRule`, and every deploy
touching it can silently re-enable cleanup ([[0200]]).

### Design traps — two inherited, one specific

- ⚠️ **Standing condition, not an event** — identical to gap 3 and to gap 2's
  structural defect. A wrong `close_usd` stays wrong until a person repairs it,
  so the alarm must keep its OK action on every rung or it latches and goes
  silent while the population grows.
- ⚠️ **Scope it to the quote leg, not to "all candles".** Exotic-quoted rows sit
  at `close_usd = 0` **by design** — there is no USD reference and no tier can
  price them ([[0182]] measured ~74M such rows on `_1h` alone). A check that
  counts every zero would breach permanently on healthy data.
- ⚠️ **The pre-epoch window is a legitimate `≈ 1.0`.** USDT was at *measured*
  par from 2021-02 until the June 2022 break ([[0172]]), and rows below
  `2021-02-07 19:00` deliberately keep `close × $1`. So the ratio check must be
  bounded to the post-break era or it fires on correct history — the mirror of
  the epoch mistake that made [[0182]] necessary twice.

## Acceptance Criteria

- [ ] Free-space alarm on the CH host, threshold chosen to give hours of
      warning, routed to the same Slack channel as the existing ops alarms
- [ ] DLQ alarm distinguishes 1 from 91 — re-notifies on growth or uses a
      threshold ladder
- [ ] Runbook note: an ingest stall is verified recovered on the DATA, never on
      alarm state, and freshness alone does not prove completeness
- [ ] Alarms verified by inducing the condition, not by reading the CDK — the
      0137 lesson that an alarm must be tested against the failure it exists for
- [ ] `prices-clickhouse-drift` runs on a schedule and reports somewhere a person
      reads, re-notifying while drift persists, with `CRITICAL` separated from
      `DRIFT` rather than collapsed into "exit 1"
- [ ] **Gap 4** — a data-level USD-correctness check runs on a schedule and
      alarms on **both** directions: no USDT-quoted candle at
      `close_usd / close ≈ 1.0` in the post-break era, and none at
      `close_usd = 0` with a `close` above the `Decimal(38, 14)` underflow
      bound. Scoped to the quote leg, so exotic-quoted zeros do not breach it.
- [ ] **Gap 4 verified by inducing it** — write a par-valued USDT candle and a
      zeroed one with a representable `close` into a test fixture and confirm
      each breaches; the [[0137]] lesson applied to this alarm too. Closes
      [[0182]]'s last acceptance criterion.
