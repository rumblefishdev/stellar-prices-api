---
id: "0204"
title: "Ops alarms missed an 11.5 h outage — no free-space alarm on the shared CH volume, and the DLQ alarm fires once then goes quiet"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0202", "0203", "0137", "0142", "0056", "0201"]
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
