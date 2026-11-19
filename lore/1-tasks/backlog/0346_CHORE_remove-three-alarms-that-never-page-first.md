---
id: "0346"
title: "Remove three alarms that another alarm always beats: ledger-processor-no-invocations, oracle-no-invocations, mtls-notafter-probe-duration-near-timeout"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0223", "0222", "0231", "0137", "0056", "0200", "0294", "0139", "0093", "0347"]
tags: [layer-infra, priority-low, effort-small, observability, cloudwatch, alarms, noise]
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../infra/src/lib/lambda-baseline.ts"
  - "../../../docs/runbooks/xdr-protocol-watch.md"
  - "../../../docs/runbooks/0139-asset-id-migration.md"
  - "https://github.com/rumblefishdev/stellar-x402/pull/186"
  - "https://github.com/rumblefishdev/soroban-block-explorer/pull/733"
history:
  - date: 2026-11-13
    status: backlog
    who: stkrolikiewicz
    note: >
      From the 2026-11-13 read-only CloudWatch review. Re-checked on
      origin/develop (66724665) and live CloudWatch the same day. Line numbers in
      the review came from origin/master; the develop ones are used here. Saves
      3 alarms ($0.30/month) and the duplicate messages on every ingest stall.
---

# Remove three alarms that another alarm always beats

## Summary

Each of these alarms watches a failure that a kept alarm catches earlier, or at
the same time with a clearer message. Removing them loses no coverage.

## 1. `prices-production-ledger-processor-no-invocations`

**Evidence.** Live `describe-alarm-history` (30-day retention, from 2026-10-14)
shows 4 OK→ALARM transitions. Each time, `rollup-freshness-1m` had already fired:

| stall | freshness-1m ALARM (UTC) | no-invocations ALARM (UTC) | lead |
|---|---|---|---|
| 10-17 | 00:36:11 | 00:38:42 | 2.5 min |
| 10-21 | 11:51:22 | 11:57:19 | 6.0 min |
| 11-06 | 00:36:54 | 00:44:30 | 7.6 min |
| 11-06 | 13:51:54 | 13:59:30 | 7.6 min |

A fifth stall (10-28 21:21 UTC) fired freshness-1m only. Both alarms send ALARM
and OK to the same ops topic, so each stall produced 4 messages instead of 2. The
review attributes every stall to an upstream Galexie stall, based on the explorer
alarm `production-galexie-ingestion-lag` firing first. That was not re-checked here.

**Edit.**
- Remove the alarm (`observability-stack.ts` ~2180-2215).
- Reword the references to it:
  - `:124`, the doc comment that calls it the pattern's origin;
  - `:1254`, the lag alarm's comment "a real halt is caught by the separate
    no-invocations alarm below";
  - `:1272`, the `-errors` description "Liveness is …-ledger-processor-no-invocations";
  - `:2191-2192`, "the only alarm watching a total ingestion halt";
  - `docs/runbooks/xdr-protocol-watch.md:75-76`.
- Move its diagnosis ("no doorbells arriving: check the SNS subscription on
  prices-ingest and BE ledger publication") into the `rollup-freshness-1m`
  description.
- Keep `ledger-processor-lag`: its 2-minute threshold is finer than the 15-minute
  freshness bound.

**Accepted risks.** Ingest-halt detection then rests on `rollup-freshness-1m`,
fed by `rollup-freshness-probe`, which has its own `-errors` and
`-no-invocations` alarms. Two cases slip past both:
- An operator backfill that writes recent 1m buckets during a halt could hide the
  halt. That case is rare and operator-driven.
- `rollup-freshness-1m` is `notBreaching` ([[0093]]'s table), so a probe that runs
  and succeeds but publishes no `RollupLagSeconds` reads as healthy, and a halt
  behind it goes unseen. Today `no-invocations` would still fire on the halt.

## 2. `prices-production-oracle-no-invocations`

**Evidence.**
- The authors' own comment (`observability-stack.ts:2344-2346`): "if the function
  stops being invoked, -dark-feed still fires (missing data is BREACHING there)".
- `oracle-dark-feed` is `FILL(written, 0) < 1`, 3×600 s, `breaching`. It also
  covers a run that is invoked but dies before it publishes (`:696-702`), which
  no-invocations cannot see.
- Live: `prices-production-oracle-watcher` ENABLED `rate(5 minutes)`. Neither
  alarm changed state in the retained history, so this rests on configuration,
  not on observed co-firing. The cost is detection at ~30 min instead of ~15 min,
  on a feed the spec calls non-critical (§2.2).

**Edit.**
- This is a type change, not a flag. `WorkerHealthAlarms.noInvocations` is
  required (`observability-stack.ts:59`), and `withLivenessNote`
  (`lambda-baseline.ts:200-209`) only branches on `WORKERS_WITHOUT_HEALTH_ALARMS`.
  Add a per-worker opt-out with a reason, like `noDurationAlarm` (`:76-86`).
- Reword `:735-737` ("…covered by prices-{env}-oracle-no-invocations instead")
  and the oracle `-errors` liveness sentence so they name dark-feed.
- Reword the state table at `packages/oracle-worker/src/metrics.rs:33` ("never
  invoked → the `-no-invocations` alarm's job") so it names dark-feed. [[0347]]
  edits `:34-35` of the same table.
- Add one sentence to dark-feed: if the worker rows show no invocations, the
  watcher rule is disabled.

## 3. `prices-production-mtls-notafter-probe-duration-near-timeout`

**Evidence.**
- Live `get-metric-statistics AWS/Lambda Duration`, 2026-08-15..11-13: 91 runs,
  average 1,077 ms, maximum 1,333 ms, against a 60 s timeout
  (`eventbridge-stack.ts:842`, `observability-stack.ts:2383`).
- A run that reaches 60 s is a timeout, which Lambda counts as an Error. So
  `-errors` (daily, 1 of 1) pages on day 1, while this alarm needs 2 consecutive
  days at ≥ 48 s.

**Edit.**
- Set `noDurationAlarm` with a reason on the `mtls-notafter-probe` `workerHealth`
  entry (`:2380`).
- Widen the option's doc comment (`:76-86`). Today it describes only
  budget-length workers.

**Accepted risk.** The early warning for a probe that slows down is lost: a run
between 48 s and 59 s pages nobody, and the first page is the timeout's `-errors`.
Runs take about 1 s today, so that is a slowdown of almost 50 times.

## Not taken from stellar-x402#186

That plan (its G- note at
[b99fad33](https://github.com/rumblefishdev/stellar-x402/blob/b99fad33b4c91ad411f958a24c5840e94e5b7858/lore/1-tasks/active/0138_RESEARCH_monitoring-cost-and-grafana-decision/notes/G-alarm-signals-and-triage-runbooks.md))
also drops these alarms. This task does not:
- **Five more `*-duration-near-timeout` alarms** (asset-discovery,
  backfill-freshness-probe, coarse-sweep, oracle, rollup-freshness-probe). The plan
  drops them for a `PR-WORKERS-FAILING` alarm and a daily headroom line, and
  neither exists yet. This review did not read their Duration either, so it can't
  say the timeout's `-errors` would come first. Only mtls-notafter-probe's runs
  were measured (§3).
- **`mtls-notafter-probe-no-invocations` and `mtls-notafter` turned `breaching`.**
  They are one change: the no-invocations alarm can go only once `mtls-notafter`
  treats missing data as breaching. This review did not look at `mtls-notafter`'s
  missing data, so the pair is left to that plan.

## Considered and rejected

- **`prices-production-cleanup-errors`.** It cannot fire while the schedule is
  disabled. But `SCHEDULE_DISABLED_WORKERS` is not tied to the rule state by any
  guard, and five runbooks re-enable the rule with `aws events enable-rule`
  outside CDK. Removing the alarm would leave a destructive nightly job with no
  error alarm once it is re-enabled. Do not remove it unless a synth guard ties
  the alarm to the rule's DISABLED state.

## Acceptance Criteria

- [ ] All three alarms are absent from synth output. Synth tests are updated, and
      the reason for each change is noted here.
- [ ] No text in `infra/`, `docs/runbooks/` or `docs/` outside `docs/scf/` points
      at a removed alarm (`git grep`).
- [ ] `docs/runbooks/0139-asset-id-migration.md:265-278` MUTED array and its
      `(32)` count are updated, or the runbook is marked historical ([[0139]] is
      archived).
- [ ] SCF evidence (`docs/scf/milestone-1-evidence.md:728, :857`,
      `milestone-3-evidence.md:377, :388`) is left as the record of what was
      submitted ([[0294]], 2026-10-06). A dated note is added only if those docs
      must describe the current state.
- [ ] Agreed with the other teams on the shared account before the deploy, in
      stellar-x402#186 and soroban-block-explorer#733.
