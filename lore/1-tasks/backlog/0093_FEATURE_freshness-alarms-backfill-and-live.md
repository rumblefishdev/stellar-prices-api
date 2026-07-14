---
id: "0093"
title: "Freshness alarms — backfill watchdog + live candle-freshness (catch silent stalls)"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0090", "0091", "0056", "0082"]
tags: ["observability", "alarms", "priority-medium", "effort-small", "phase-future"]
links: []
history:
  - date: 2026-07-14
    status: backlog
    who: okarcz
    note: >
      Spawned from the 2026-07-14 proto27/backfill investigation. Two monitoring
      blind-spots let outages go unnoticed: (1) the backfill has NO watchdog/alarm
      — a host kill ~07-08 became a 6-week gap silently; (2) the live doorbell-lag
      alarm is blind to a "drains-but-doesn't-write" processor (queue healthy, no
      candles). Add freshness-based alarms for both.
---

# Freshness alarms — backfill watchdog + live candle-freshness (catch silent stalls)

## Summary

Two monitoring blind-spots let ingestion outages go unnoticed for days. Add
freshness-based alarms so a silent stall pages someone:

1. **Backfill has no supervision at all.** A host-level kill (~2026-07-08) stopped the
   Soroban-era backfill mid-run and nobody noticed → a ~6-week ledger gap. Nothing
   watches `backfill_progress`.
2. **The live doorbell-lag alarm is blind to "drains-but-doesn't-write."** It only
   fires when the SQS queue backs up. A processor that consumes messages and writes no
   candles keeps the queue healthy → no alarm, silent data loss. (Observed 2026-07-14:
   `prices-production-ledger-processor-lag` read OK while `price_ohlcv_1m` was 6 days stale.)

## Context

Spawned from the proto27/backfill investigation. Alarm infra exists (task 0056, Slack
routing to `#stellar-prices-api-bot`), and a `backfill-freshness-probe` package
(`packages/backfill-freshness-probe`) already exists to reuse. Related to 0082
(post-deploy worker/MV verification).

## Implementation

- **Live candle-freshness alarm:** alarm when `max(timestamp)` in `price_ohlcv_1m`
  (or `backfill_progress.newest_data_available`) is older than N minutes — a direct
  "are candles landing?" signal, independent of queue depth. Route to Slack.
- **Backfill watchdog:** while a backfill run is expected, alarm when
  `backfill_progress` (`current_ledger` / `last_push_at`) hasn't advanced for M
  minutes. Reuse the `backfill-freshness-probe` pattern.
- **Tune the doorbell-lag threshold:** `config.opsAlarms.ledgerProcessorLagSeconds`
  (currently 120s) false-fires during normal catch-up — widen the window or raise it
  once live is caught up.

## Acceptance Criteria

- [ ] Live candle-freshness alarm deployed; fires on a simulated stale frontier; routes to Slack.
- [ ] Backfill-progress watchdog defined (active only during expected runs) + documented.
- [ ] Doorbell-lag threshold reviewed/tuned to reduce catch-up false-fires.
