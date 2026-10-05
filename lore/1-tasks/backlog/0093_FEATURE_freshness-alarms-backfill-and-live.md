---
id: "0093"
title: "Backfill watchdog that can actually fire + doorbell-lag review (live candle-freshness is done by 0137)"
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
  - date: 2026-09-25
    status: backlog
    who: okarcz
    note: >
      NARROWED on the operator's 2026-09-25 decision, from the 2026-09-24 read-only
      check. AC1 (live candle-freshness) is MET by [[0137]]'s
      prices-production-rollup-freshness-1m, live on prod since 2026-08-12
      (infra/src/lib/stacks/observability-stack.ts:949-951, one alarm per OHLCV
      table). Still open: (1) a backfill watchdog that can fire - the
      sdex-push-freshness alarm (observability-stack.ts:823) can never fire again
      because resolve_status (packages/sdex-backfill/src/sink.rs:379) will not
      downgrade a stored `completed`, and both push-freshness alarms sit at
      604800 s = 7 days (infra/envs/production.json:35-36); (2) the doorbell-lag
      review (ledgerProcessorLagSeconds = 120, production.json:48). The deployed
      state is NOT yet confirmed: describe-alarms was not run (AWS SSO expired),
      so that is now an AC. Original text kept below.
---

# Backfill watchdog that can actually fire + doorbell-lag review

## Summary (narrowed 2026-09-25)

The live half of this task is done: [[0137]]'s `prices-production-rollup-freshness-1m`
is the "are candles landing?" alarm AC1 asked for, and it has been live on prod
since 2026-08-12. Two gaps are left:

1. **No backfill watchdog that can fire in time.** The only backfill freshness
   alarms read `Prices/Backfill PushAgeSeconds` from the `backfill-freshness-probe`:
   - `prices-production-sdex-push-freshness` (`infra/src/lib/stacks/observability-stack.ts:823`)
     **can never fire again.** `resolve_status` (`packages/sdex-backfill/src/sink.rs:379`)
     refuses to downgrade a stored `completed`, so `sdex_archive` never returns to
     `running`, and the probe never publishes for it. A future SDEX backfill runs
     with no freshness cover (see the comment at `observability-stack.ts:882-886`).
   - `prices-production-amm-push-freshness` (`observability-stack.ts:894`) can
     fire, but both alarms use a threshold of `604800` s, which is **7 days**
     (`infra/envs/production.json:35-36`). A stall is paged a week late, which
     is too slow to prevent the 6-week gap this task was spawned from.
2. **Doorbell-lag review.** `ledgerProcessorLagSeconds` is still `120`
   (`infra/envs/production.json:48`). The original concern was that it
   false-fires during catch-up. That has not been reviewed against its alarm
   history.

The alarm state on prod has **not** been confirmed yet: `describe-alarms` was not
run on 2026-09-24 because AWS SSO had expired. The code says what should be
deployed, but that is not proof of what is live ([[cdk-is-not-the-state-of-production]]).

## Acceptance Criteria

- [x] ~~Live candle-freshness alarm deployed; fires on a simulated stale frontier;
      routes to Slack.~~ **Met by [[0137]]:** `prices-production-rollup-freshness-1m`
      (created per table at `observability-stack.ts:949-951`), live on prod since
      2026-08-12.
- [ ] The operator confirms the deployed state with `aws cloudwatch describe-alarms`
      (profile `soroban-readonly`, `eu-central-1`) on
      `prices-production-{rollup-freshness-1m,ledger-processor-lag,sdex-push-freshness,amm-push-freshness}`.
      The command, its date, and each alarm's state, threshold and actions are
      recorded here.
- [ ] A backfill watchdog exists that **can fire** for a future SDEX run (either
      the `resolve_status` `completed` lock is resolved, or a signal that does not
      depend on it is used) and pages within a window measured in hours, not 7
      days. When it should be active, and how it is armed and disarmed around a
      run, is documented.
- [ ] The doorbell-lag threshold (`ledgerProcessorLagSeconds = 120`) is reviewed
      against `describe-alarm-history` for `prices-production-ledger-processor-lag`.
      It is either tuned, or kept with the reason recorded.

## Original scope (before 2026-09-25 narrowing)

> Kept verbatim for the record (headings demoted one level). Original title: *Freshness alarms — backfill watchdog + live candle-freshness (catch silent stalls)*. The Summary and Acceptance Criteria above supersede it.

### Summary

Two monitoring blind-spots let ingestion outages go unnoticed for days. Add
freshness-based alarms so a silent stall pages someone:

1. **Backfill has no supervision at all.** A host-level kill (~2026-07-08) stopped the
   Soroban-era backfill mid-run and nobody noticed → a ~6-week ledger gap. Nothing
   watches `backfill_progress`.
2. **The live doorbell-lag alarm is blind to "drains-but-doesn't-write."** It only
   fires when the SQS queue backs up. A processor that consumes messages and writes no
   candles keeps the queue healthy → no alarm, silent data loss. (Observed 2026-07-14:
   `prices-production-ledger-processor-lag` read OK while `price_ohlcv_1m` was 6 days stale.)

### Context

Spawned from the proto27/backfill investigation. Alarm infra exists (task 0056, Slack
routing to `#stellar-prices-api-bot`), and a `backfill-freshness-probe` package
(`packages/backfill-freshness-probe`) already exists to reuse. Related to 0082
(post-deploy worker/MV verification).

### Implementation

- **Live candle-freshness alarm:** alarm when `max(timestamp)` in `price_ohlcv_1m`
  (or `backfill_progress.newest_data_available`) is older than N minutes — a direct
  "are candles landing?" signal, independent of queue depth. Route to Slack.
- **Backfill watchdog:** while a backfill run is expected, alarm when
  `backfill_progress` (`current_ledger` / `last_push_at`) hasn't advanced for M
  minutes. Reuse the `backfill-freshness-probe` pattern.
- **Tune the doorbell-lag threshold:** `config.opsAlarms.ledgerProcessorLagSeconds`
  (currently 120s) false-fires during normal catch-up — widen the window or raise it
  once live is caught up.

### Acceptance Criteria

- [ ] Live candle-freshness alarm deployed; fires on a simulated stale frontier; routes to Slack.
- [ ] Backfill-progress watchdog defined (active only during expected runs) + documented.
- [ ] Doorbell-lag threshold reviewed/tuned to reduce catch-up false-fires.
