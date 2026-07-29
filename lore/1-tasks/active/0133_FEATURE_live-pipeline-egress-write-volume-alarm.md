---
id: "0133"
title: "Guardrail: egress / write-volume alarm on the live pipeline so amplification shows on a dashboard, not a bill"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0132", "0039", "0056"]
tags: [observability, cost, clickhouse, egress, perf, priority-medium, effort-small, phase-future]
links: []
history:
  - date: 2026-07-29
    status: backlog
    who: okarcz
    note: >
      Spawned from 0132 future work (Step 3). 0132 was found by the BE team via
      part_log + Cost Explorer, not by our own monitoring — the ~9,413× asset
      re-emit ran for weeks billing ~$337/mo of AWS→Hetzner egress with nothing
      watching. This adds the missing meter so the next amplification surfaces on
      a dashboard/alarm instead of a surprise invoice.
  - date: 2026-07-29
    status: active
    who: okarcz
    note: >
      Promoted to active right after 0132's fix deployed + verified live
      (writes 21.7M/10min → 0). The guardrail is the direct lesson of 0132:
      the amplification ran undetected for weeks. Prioritised so the next one
      hits a dashboard, not a bill.
---

# Egress / write-volume alarm on the live pipeline

## Summary

Task 0132 (live processor re-emitting the whole asset registry every reconcile,
9,413× amplification, ~$337/mo egress) went undetected because nothing watched
write volume or Lambda egress — the BE team found it in `system.part_log` and AWS
Cost Explorer. This task adds a guardrail so a future amplification is caught by an
alarm, not a bill.

## Context

- The cost is invisible to functional tests (output is correct) and only appears in
  the split Lambda→Hetzner topology. The right detection layer is ops metrics, not
  unit tests — see 0132 rationale.
- Alarm plumbing already exists (task 0056 → Slack `#stellar-prices-api-bot`); this
  should reuse it.

## Chosen signal: CH **write amplification**, not egress

Two candidate signals; write amplification wins:

- ❌ **Lambda egress (`EUC1-DataTransfer-Out-Bytes`)** — a *lagging cost* signal, not a
  CloudWatch metric. Lambda emits no per-function egress metric; it only surfaces in
  Cost Explorer / Budgets (hours-to-a-day late, account-scoped, hard to attribute to
  one function). A Budgets/Cost-Anomaly alarm is coarse and slow.
- ✅ **Per-table write amplification from `system.part_log`** — the *leading cause*
  (write volume drives egress) and directly attributable. This is what BE actually
  used to find 0132. Measurable server-side on prod CH, off the hot path, and general:
  it watches **every** `prices.*` table, so it catches the *next unknown* amplification,
  not just a re-run of the asset bug.

## Implementation — mirror the existing probe pattern (task 0056)

Model it on `backfill-freshness-probe` / `mtls-notafter-probe`: a scheduled Rust Lambda
that queries CH (invisible to CloudWatch), republishes a custom metric, and an alarm
fires on it → the 0056 SNS topic → AWS Chatbot Slack (`#stellar-prices-api-bot`).

New crate **`write-amplification-probe`**:

- **Query** (server-side, one round-trip): over the trailing window (e.g. 1h), for each
  `prices.*` table, `sum(rows)` written (`part_log` `event_type='NewPart'`) divided by
  the table's real deduplicated row count → an **amplification factor** per table.
  ```sql
  -- shape; refine at implementation
  SELECT table, sum(rows) AS written_1h
  FROM system.part_log
  WHERE database='prices' AND event_type='NewPart' AND event_time >= now()-INTERVAL 1 HOUR
  GROUP BY table
  ```
  Real row counts per table come from a cheap `count()` (or a maintained size), so the
  factor = `written_1h / real_rows`. (0132 read 9,413×; legit RMT churn is a few ×.)
- **Metric:** publish `WriteAmplificationFactor` (and optionally `RowsWrittenPerHour`)
  under namespace **`Prices/Ingest`**, dimensioned by `Table`. Split for testability
  exactly like the other probes: pure factor-math + query-shaping are feature-free and
  unit-tested; the CH fetch + `PutMetricData` are gated behind `lambda`/`aws-mtls`.
- **CH identity:** the probe reads over mTLS as the existing **`prices_reader`** user (reuse
  the reader cert/secret — no new identity). This requires the one-time prerequisite grant
  below; `prices_reader` otherwise stays SELECT-only on `prices.*`.
- **IAM:** Lambda role gets `PutMetricData` scoped by a `cloudwatch:namespace = Prices/Ingest`
  condition (same shape as `Prices/Backfill` / `Prices/Mtls`).

## Prerequisite: one-time CH grant (RESOLVED 2026-07-29)

`prices_reader` is DB-scoped and cannot read `system.part_log` — confirmed via
`system.grants`: `prices_reader` has only `SELECT ON prices.*`; `prices_writer` has
`prices.*` **plus** `SELECT ON system.parts` (so a prices user reading a *system* table is
already an established, deliberate pattern here — we add one more). **Decision (Plan A):**
grant `prices_reader` read access to the write-log, run **once by a CH admin** on ch-prod-01
before the alarm is meaningful:

```sql
GRANT SELECT ON system.part_log TO prices_reader;
```

Read-only, low-sensitivity (metadata only — no price data), and fully reversible
(`REVOKE SELECT ON system.part_log FROM prices_reader`). Reusing `prices_reader` (rather than
a dedicated probe user) is the deliberate choice: no new mTLS cert/identity to provision. The
mild trade-off — the read-API's identity also gains this one metadata read — is accepted;
it grants no write/alter/delete and touches no `prices.*` data. `system.parts` (already
granted to the writer) is **not** a substitute: it is a current-parts snapshot, so the
over-written rows are merged away before it could see them; `part_log` is the event log that
records the write *rate*, which is what detects amplification.
- **Schedule:** an `events.Rule` in `eventbridge-stack.ts` (hourly is enough; the bug
  bled for weeks — sub-hour detection is unnecessary), passing `errorAlarmActions` like
  the other two probes so a *dead probe* also alarms.
- **Alarm:** in `observability-stack.ts`, fire when `max(WriteAmplificationFactor)`
  across tables breaches the threshold for N datapoints → `opsAlarmsTopic` (SNS) →
  existing Slack channel. Add `OkAction` too (recovery notice), matching the repo.

## Threshold

Operator-tunable via `config`. Legit ReplacingMergeTree churn is single-digit × (BE's
sister `default.assets` ran 4.6×); 0132 ran 9,413×. A threshold of **~50×** catches a
runaway with wide margin above normal churn. Also consider an absolute floor (e.g.
`RowsWrittenPerHour` per table) so a low-real-row table can't hide a large absolute write.

## Implementation Notes (built 2026-07-29)

Mirrors the 0056 probe pattern end-to-end. New crate **`write-amplification-probe`**
(`src/lib.rs` pure logic + tests, `src/main.rs` `lambda`-gated entrypoint), added to the
workspace members. Infra: `eventbridge-stack.ts` (rule + `createWorkerLambda` on the `api`/
`prices_reader` bundle + `PutMetricData` scoped to `Prices/Ingest`), `observability-stack.ts`
(alarm → existing SNS/Slack), `types.ts` + `envs/production.json` (schedule + threshold).
Verified: 4 unit tests pass, clippy clean, arm64 bootstrap builds, infra `tsc` build passes,
full app `cdk synth` succeeds, and the alarm/probe/IAM land correctly in the synthesized CFN.

## Design Decisions

### Emerged

1. **General sweep, not an assets-specific watch** — the point of a guardrail is the *next*
   unknown regression, not re-watching the one we already fixed. The probe reads every
   `prices.*` table's write volume.
2. **`prices_reader`, not a dedicated probe user** — reuse the existing `api` mTLS bundle (no
   new cert to provision); the trade-off (the read-API identity gains one metadata read) is
   accepted (user decision).
3. **Absolute `MaxRowsWrittenPerHour`, not a written÷real *ratio*** — the ratio would need
   `system.parts` too, but `prices_reader` is granted only `system.part_log` (+ `prices.*`).
   For a guardrail, absolute rows/hour is sufficient: the busiest legit table is <1M/h and
   0132 was ~130M/h, so a 10M threshold has wide margin both ways. Publishing a single scalar
   (max across tables) rather than a per-`Table` metric also keeps CloudWatch dimensions
   bounded and the alarm a trivial threshold. A true ratio is a documented future enhancement
   that would add a `system.parts` read grant.

## Open Questions / Risks

- ✅ **`system.part_log` read grant — RESOLVED** (see Prerequisite above): Plan A, grant
  `SELECT ON system.part_log TO prices_reader`. The Plan-B fallback (ledger-processor emits
  its own `AssetRowsWrittenPerRun`) is no longer needed but kept in Alternatives for record.
- **`part_log` TTL** — hourly window is well inside typical retention; verify on ch-prod-01.
- Threshold false-positives during a legitimate heavy backfill/pre-roll writing to a
  live table — document the "expected during a known backfill" caveat in the runbook.

## Acceptance Criteria

- [ ] Prerequisite grant applied on ch-prod-01: `GRANT SELECT ON system.part_log TO prices_reader`
      (one-time admin op at **deploy time**; verify the probe can then read `system.part_log`).
- [x] `write-amplification-probe` crate: pure metric-shaping (`max_rows_written`) + query-shaping
      unit-tested (4 tests); CH fetch (as `prices_reader` over mTLS) + `PutMetricData` gated behind
      `lambda`/`aws-mtls` (mirrors 0056 probes). Clippy clean; arm64 bootstrap builds.
- [x] Publishes `MaxRowsWrittenPerHour` (see Emerged #3 — scalar max, not per-`Table` ratio)
      under `Prices/Ingest`; IAM `PutMetricData` scoped by the namespace condition.
- [x] EventBridge rule schedules it `rate(1 hour)`, with `errorAlarmActions` so a dead probe alarms.
- [x] CloudWatch alarm on the metric breaching the threshold → 0056 SNS/Slack; `OkAction` set.
      Verified in synthesized CFN (threshold 10,000,000, GreaterThanThreshold, notBreaching).
- [x] Threshold operator-tunable via `config.opsAlarms.writeAmplificationRowsPerHour` (default 10M);
      documented (busiest legit table <1M/h vs 0132's ~130M/h).
- [~] Runbook: the alarm *description* embeds "check `system.part_log` per table to find the
      offender"; a fuller runbook note (links 0132's part_log day-slice + anti-join) is a small
      follow-up.
- [x] `system.part_log` read-access question resolved — Plan A (grant to `prices_reader`);
      see Prerequisite. Grant *application* tracked as the first AC above.

## Alternatives Considered

- **AWS Budgets / Cost Anomaly Detection on DataTransfer** — coarse, account-scoped, and
  lags by up to a day; useful as a cheap backstop but not the primary (it's what let 0132
  run for weeks). Could add later as a low-effort secondary net.
- **In-processor `PutMetricData` per run** — narrow (asset write only) and on the hot path;
  kept as the fallback if `system.part_log` proves ungrantable.
