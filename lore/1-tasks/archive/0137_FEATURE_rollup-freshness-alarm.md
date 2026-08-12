---
id: "0137"
title: "Rollup freshness alarm — a starved rollup MV reports success and nothing notices"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0136", "0104", "0109", "0056", "0143", "0112", "0181"]
tags:
  [
    "priority-high",
    "effort-small",
    "clickhouse",
    "observability",
    "milestone-M2",
  ]
milestone: 2
links:
  - "../../../docs/runbooks/0136-coarse-rollup-merge-recovery.md"
history:
  - date: 2026-07-30
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0136]]. Every coarse OHLCV table was frozen for nine days
      and nothing alarmed, because a rollup MV that reads stale input still
      reports `status = Scheduled` with an empty exception. Health was measured
      on the wrong thing — the MV, not the data.
  - date: 2026-08-12
    status: active
    who: okarcz
    note: >
      Activated for implementation, as the first of the two acceptance criteria
      still holding [[0136]] open.
      **The §Implementation "prefer folding into an existing scheduled worker
      over a new Lambda" preference is REVERSED**, decided with the operator
      before any code was written. That preference was written 2026-07-30; the
      codebase has since established the opposite pattern for exactly this
      failure mode. [[0112]] found that three scheduled workers each had a single
      alarm reading a custom metric the worker publishes *only if it survives to
      the end of a pass*, so none of them could detect the worker dying — and
      added `addWorkerHealthAlarms` to cover it. Folding the rollup freshness
      signal into `enrichment-worker` would rebuild that exact blind spot one
      layer up: an enrichment stall (the component behind [[0111]]'s four-day
      outage) would publish no metric, the `NOT_BREACHING` alarm would sit
      silently OK, and a frozen rollup would again go unreported. A dedicated
      `rollup-freshness-probe` mirrors the tested `backfill-freshness-probe`
      ([[0056]]) 1:1 and earns dead-probe cover by adding one entry to the
      existing `workerHealth` array.
      Sequencing note: the leading-indicator criteria (`system.mutations` age,
      part counts, `system.view_refreshes` exceptions) depend on system-table
      readability by the scoped mTLS user, which is XML-managed by BE and cannot
      be `GRANT`ed by us ([[0134]]). The primary `max(timestamp)` freshness
      signal needs no system-table access at all, so it ships first and the
      leading indicators are gated on that measurement.
  - date: 2026-08-12
    status: completed
    who: okarcz
    note: >
      COMPLETE and LIVE on production. Four of five acceptance criteria met; the
      fifth (leading indicators) is owned by [[0181]].
      Shipped `rollup-freshness-probe` — a rate(15 minutes) Lambda publishing each
      OHLCV tier's `now() - max(timestamp)` as `Prices/Rollup RollupLagSeconds`,
      with seven per-tier alarms plus dead-probe cover from [[0112]]'s
      `workerHealth` array. It measures the DATA, not the MV, which is the only
      signal that could have caught [[0136]]'s nine-day freeze.
      Fire-tested on prod: `OK → ALARM` in under a minute, recovery in exactly
      30 min, Slack on both transitions; all 10 alarms independently demonstrated
      their SNS route. Probe runs in 90 ms / 48 MB against 735M-row tables,
      confirming on real data that `max(timestamp)` is answered from per-part
      metadata rather than a column scan.
      Code review (PR #199) caught three issues, all fixed before merge — the
      most important being that the empty-tier gate created a false RECOVERY
      (a tier emptied by retention mid-freeze read as healthy), rebuilding the
      0136 blind spot one layer up. Fixed with a fine→coarse rule rather than the
      reviewer's proposal, which would have paged on bootstrap.
      11 unit + 2 CH integration tests on the 26.3.10.60 pin; fmt and clippy
      clean. Spawned [[0181]] (leading indicators + the coarsest-tier empty hole).
      ⚠️ Left undone deliberately: the [[0136]] note to BE, which should carry
      0181's two-line grant request with it.
---

# A starved rollup reports success — measure freshness, not exit status

## Summary

[[0136]] froze `price_ohlcv_15m` through `_1M` for **nine days** with no alert.
Eight of the nine refreshable MVs reported `status = Scheduled`, empty
`exception`, every single cycle. Only `mv_ohlcv_1m_to_15m` carried the error;
everything downstream of it rolled up stale input and called that success.

Rolling up nothing is not a failure, so no failure was reported. The health
signal has to be **data freshness**, not MV exit status.

## Context

The gap was found by accident — [[0072]]'s rollout verification noticed
`change_7d_pct` was 0 for every asset, which traced back to `price_ohlcv_1h`
having no rows in the trailing 7 days. Without that coincidence it could have
run indefinitely.

Existing alarms ([[0056]]) cover Lambda/API failure modes. Nothing watches
whether the data at rest is advancing.

## Implementation

- **Signal.** Per coarse table, `now() - max(timestamp)` against an expected
  bound derived from its cadence. A rough starting shape:

  | table | expected lag bound |
  |---|---|
  | `price_ohlcv_1m` | 15 min |
  | `price_ohlcv_15m` | 1 h |
  | `price_ohlcv_1h` | 3 h |
  | `price_ohlcv_4h` | 12 h |
  | `price_ohlcv_1d` | 48 h |
  | `price_ohlcv_1w` | 10 d |
  | `price_ohlcv_1M` | 45 d |

  Tune against real cadence — [[0104]] owns the cadence-vs-window question and
  the bounds should not contradict it.

- **Also alarm on the leading indicators**, which would each have fired days
  before the freeze became visible:
  - any row in `system.mutations` with `is_done = 0` older than ~1 h
    (these sat for 13 days) — overlaps [[0109]]'s guard, which already has to
    watch this table;
  - any `prices` table above ~1,000 active parts (`parts_to_delay_insert`), well
    before the 5,000 throw limit;
  - a non-empty `exception` on any row of `system.view_refreshes`.

- **Where it runs.** Prefer folding into an existing scheduled worker over a new
  Lambda — the enrichment worker already runs hourly and already talks to CH.
  Route to the existing Slack channel used by [[0056]].

- **Access.** Reads `system.parts` (already granted to `prices_writer`),
  `system.mutations` and `system.view_refreshes`. Confirm the latter two are
  readable by the scoped user before designing around them — the runtime users
  are XML-managed by BE and cannot be SQL-`GRANT`ed by us (see [[0134]]). If
  they are not readable, the freshness check on `max(timestamp)` alone is
  sufficient for the primary signal and needs no system-table access at all.

## Acceptance Criteria

- [x] A freshness check runs on a schedule and alerts when any coarse table
      exceeds its lag bound. `rollup-freshness-probe` on `rate(15 minutes)`
      publishes `Prices/Rollup` `RollupLagSeconds` per tier; **seven** alarms
      (one per granularity) fire on it. Verified in the synthesized template.
- [x] Replaying the [[0136]] conditions (a stalled `1m → 15m` rollup) fires the
      alarm within a day. Covered by
      `freshness_query_executes_deserializes_and_gates_empty_tiers`, which seeds
      a 20-day-stale `1h` alongside a fresh `1m` and asserts the stale tier
      exceeds its bound while the fresh one does not.
      ✅ **Measured on production 2026-08-12:** breach → ALARM in **≤60 s**
      (`11:57:52`), recovery in **exactly 30 min** (`12:27:52`) — the 1-of-2
      evaluation over 15-min `Maximum` periods behaving as designed. Far inside
      "within a day".
- [ ] Pending-mutation age and part-count checks are covered, here or in
      [[0109]], without duplicating each other. **DEFERRED to [[0181]]**, and the
      access question is now **measured rather than assumed** (2026-08-12):
      `prices_writer` holds `SELECT ON system.parts` and nothing else under
      `system.*`, so part counts are unblocked while mutation-age and
      `view_refreshes` need a two-line BE grant. ⚠️ The deferral also has a
      non-access half that survives regardless: this AC says "here **or in
      0109**", and 0109's guard already watches `system.mutations` — ownership
      must be settled before either builds it, or both page for the same event.
- [x] Alarm routes somewhere a human reads, and a fire-test has passed.
      ✅ **DONE 2026-08-12, and proven more broadly than planned.** The fire-test
      (`put-metric-data`, synthetic 9,999,999 s on `price_ohlcv_1M`) drove
      `OK → ALARM → OK` with Slack messages on **both** transitions. Separately,
      all **10** alarms delivered to `#stellar-prices-api-bot` on first
      resolution, so the SNS → Slack route is demonstrated per-alarm rather than
      for one sample. The OK direction is the half that matters most: it is what
      proves a real freeze *ending* would be reported as ended rather than
      falling silent — review finding 1's whole concern.
- [x] Lag bounds are recorded with their rationale, and do not contradict
      [[0104]]. Rationale is the bucket-width sawtooth (§Design Decisions 1),
      documented on `ROLLUP_TIERS` and on `opsAlarms.rollupLagSeconds`, and
      **enforced by both a unit test and the synth-time config validator**.

## Implementation Notes

**New crate `packages/rollup-freshness-probe/`** (~250 lines lib, ~80 main,
~200 IT), mirroring the tested `backfill-freshness-probe` shape: pure
metric-shaping compiled in every build and unit-tested without the AWS SDK, the
CloudWatch publish gated behind the `lambda` feature.

- `freshness_query()` builds one `UNION ALL` over all seven granularities from
  `ROLLUP_TIERS`, so a tier cannot be added to the threshold table and forgotten
  in the query. Each branch is
  `now() - max(timestamp) … HAVING count() > 0`.
- `ROLLUP_TIERS` carries each tier's bucket width **and** its bound, so the
  "bound must exceed bucket width" invariant is unit-testable.

**Infra** — `rate(15 minutes)` rule, worker Lambda (256 MB / 1 min), a
`PutMetricData` grant scoped to the `Prices/Rollup` namespace, seven per-tier
alarms, and one new entry in the [[0112]] `workerHealth` array so the probe
itself is watched. Config: `scheduleExpressions.rollupFreshnessProbe` and
`opsAlarms.rollupLagSeconds` (a per-table map), both validated at synth.

**Verification.** 6 unit + 2 Docker-gated CH integration tests green on the
**26.3.10.60** prod pin; `cargo fmt --check` clean; **clippy clean** (no
pre-existing warnings in this crate). `cdk synth` produces 9 new alarms
(13 → 22 total) with correct thresholds, `Table` dimensions and SNS actions;
EventBridge synth confirms the rule, the function and the namespace-scoped
grant. The `bootstrap` was built and `strings`-verified to contain this code
before trusting the synth, per the [[0141]] stale-asset trap.

## Issues Encountered

- **`cdk synth` runs `dist/`, not `src/`.** The first synth after the alarm code
  was written produced **zero** new alarms and reported success. `cdk.json` is
  `"app": "node dist/bin/production.js"`, so `tsc --noEmit` type-checking passes
  while the synthesized template is still the previous build. `nx build infra`
  is mandatory between editing a stack and synthesizing it. **Silent, and it
  looks exactly like "my change had no effect"** — the same shape as [[0142]]'s
  no-op MV edits and [[0141]]'s stale lambda assets.
- **Local ClickHouse carried state from earlier test runs.** The first
  measurement of the empty-tier gate appeared to show it failing — four tiers
  returned huge lags. They had leftover rows from previous integration tests;
  the gate was correct. Truncate before measuring, and do not read a shared
  local container as a clean room.
- **Clippy's doc-list lints are stricter than the neighbouring file.** A
  numbered list with an indented command block below it produced both
  `doc_overindented_list_items` and `doc_list_item_without_indentation`; the
  indented block was being parsed as list continuation. Fixed with a bullet list
  plus an explicit ```` ```text ```` fence.

## Design Decisions

### From Plan

1. **Bounds are sized off bucket width, not off latency.** `timestamp` is the
   bucket **start**, so a healthy tier's lag sawtooths from 0 up to one full
   bucket width before the next bucket opens — a `1w` tier reports a six-day lag
   the day before rollover while perfectly healthy. Any bound at or below the
   bucket width false-fires once per bucket forever, and **a permanently-firing
   alarm gets muted, which is the exact state this task exists to end.** Every
   bound is `bucket + headroom`. Enforced in two places: a unit test over
   `ROLLUP_TIERS`, and a synth-time validator that rejects the config outright.
   ⚠️ `1M` buckets are weeks-attributed-by-start, so the width is ~31 d, not 30.

### Emerged (from code review, PR #199)

9. 🔴 **The empty-tier gate created a false-RECOVERY, and that is worse than the
   false-fire it prevented.** Review finding 1, confirmed. `price_ohlcv_15m` is
   retained 30 days and `_1m` 7 days by `cleanup-worker` dropping partitions
   (`cleanup-worker/src/lib.rs:31-32`). A 0136-style freeze alarms correctly at
   first — then cleanup drops the last partition, the table goes **empty**, the
   gate emits no datum, `NOT_BREACHING` scores that healthy, and the alarm
   transitions to **OK, announcing a recovery into Slack while the tier is still
   frozen.** A tier emptied by a `DETACH`/`ATTACH` could likewise never alarm.
   That is the 0137 blind spot rebuilt one layer up.
   **Fix — but not the one the review proposed.** "Publish a breaching sentinel
   for empty tiers" would page on all seven alarms during bootstrap, and keep
   `1M` firing for up to a month, since a new environment legitimately has empty
   coarse tiers. Data flows fine → coarse, so a coarser tier can only hold data
   if every finer tier did first, which gives a rule with no such false positive:
   > **an empty tier is anomalous iff some COARSER tier is populated.**
   Fresh env → nothing published → OK. Bootstrap (`1m` filling, coarse tiers not
   yet rolled) → no coarser tier populated → nothing synthesised. `15m` emptied
   by retention mid-freeze while the forever-tables hold history → sentinel →
   **alarm stays firing.** ⚠️ Known limit: `1M` has no coarser tier, so an empty
   `1M` cannot be caught this way — carried to [[0181]].
10. **Alarm floors were bucket width; the real healthy peak is bucket +
    feeding-MV refresh.** Review finding 2, confirmed against `rollups.sql`
    (`1d→1w` and `1w→1M` are both `REFRESH EVERY 1 DAY`). A bucket cannot appear
    until its feeding MV next runs, so `1w = 8 d` cleared the old floor while
    false-firing weekly. Floor is now `bucket + mv_refresh`, and the documented
    headroom is corrected — it had **overstated** the true margin.
    ⚠️ **The review's `1M` example was slightly off and the correction matters.**
    It called `1M = 40 d` a false-firing config; measured, `1M`'s real peak is
    **38 d** — 31 d month + **6 d** until a week actually *starts* in the month
    (buckets are weeks-attributed-by-start) + 1 d MV refresh — so 40 d is
    genuinely, if thinly, safe. The floor encodes 38 d and the validator boundary
    was verified: 38 d rejected, 39 d accepted. `1M` is the tightest tier at 7 d
    of headroom.
11. **Per-tier alarms are now 1-of-2, not 1-of-1.** Review finding 3, confirmed.
    With the period equal to the probe cadence, a single probe outage makes every
    datum go missing and `NOT_BREACHING` flips **all seven** alarms to OK — seven
    "recovered" messages for tiers that are still frozen, arriving *before* the
    probe's own `-errors` alarm fires. `evaluationPeriods: 2` /
    `datapointsToAlarm: 1` latches a real breach across one missed cycle while
    still alarming on the first bad reading. Pre-existing shape shared with
    `sdexPushFreshnessAlarm`; not changed there, since the blast radius is 1×.

### Emerged

2. **A dedicated probe Lambda, reversing the task's stated preference.** Decided
   with the operator before any code. Rationale in the activation history entry
   above: [[0112]]'s finding makes "publish from an existing worker" the one
   shape that cannot detect its own failure.
3. **Seven alarms, not one aggregate.** *Which* tier is stale is the diagnosis —
   in 0136 the break was at `mv_ohlcv_1m_to_15m` and every coarser tier merely
   inherited it. One alarm over all seven would say "rollups are stale" and lose
   the fact that `1m` was healthy, which is what localises the fault.
4. **`HAVING count() > 0` gates empty tiers.** Measured on 26.3.10.60: `max()`
   over zero rows returns `1970-01-01`, **not** NULL and not an empty result, so
   an ungated empty tier reports a lag of **1,786,526,859 s (~56 years)** and
   breaches every threshold. Ungated, a freshly-provisioned environment pages on
   all seven alarms on its first run. Same "absent means unknown, not broken"
   shape as [[0056]]'s finding-A gate. A dedicated IT pins the ClickHouse
   behaviour itself, so the gate cannot decay into cargo cult.
5. **Read the tables directly; no `system.*` access.** `max(timestamp)` looked
   like it should scan the column — `timestamp` is only the *fourth* sort-key
   column — but ClickHouse answers it from per-part min/max metadata:
   **47 rows / 1.10 KiB / 1 ms** on a 2M-row, 47-part table, and *cheaper* than
   the equivalent `SELECT max(max_time) FROM system.parts` (15 ms). So the
   obvious optimisation was both unnecessary and slower, and avoiding it removes
   the [[0134]] grant dependency entirely.
6. **No `FINAL`.** Duplicate or superseded `ReplacingMergeTree` rows cannot
   change a `max()` — an unmerged older version carries the same `timestamp` —
   so `FINAL` would force a merge pass over up to 735M rows every 15 minutes for
   an answer that cannot differ.
7. **The union is wrapped in a subquery.** Written flat,
   `… UNION ALL SELECT … ORDER BY x` binds the `ORDER BY` to the final branch
   only and returns unsorted rows — observed directly on 26.3.10.60.
8. **Thresholds live in CDK config and are duplicated in Rust.** The alarm needs
   them in CDK; the invariant test and the rationale want them in Rust. The
   duplication is documented in both places with CDK named authoritative, in the
   same spirit as `observability-stack.ts`'s existing note on duplicated
   timeouts/cadences. ⚠️ A drift mis-tunes the alarm without failing any test.

## Deployment — 2026-08-12, production

Deployed by the operator from `develop`, EventBridge first so the probe was
publishing before the alarms began evaluating. Both stacks have **no CDK
dependency edges**, verified from the synth manifest before deploying — which
mattered, because `ApiGateway` was concurrently carrying [[0157]]'s undeployed
key-destroying diff and `make deploy-production` would have shipped it.

| step | result |
|---|---|
| `verify-lambda-assets.sh` | 10/10 crates |
| `strings … bootstrap \| grep RollupLagSeconds` | present ([[0141]] stale-asset check) |
| `make deploy-production-eventbridge` | ✅ 120 s |
| manual `lambda invoke` | 7 tiers, values matching the pre-flight |
| `make deploy-production-observability` | ✅ 12 s |
| all 10 alarms | `OK` |
| fire-test | `OK → ALARM` 11:57:52 → `OK` 12:27:52 |

**Probe performance — the design decision, confirmed on production data.**
Warm invocation **90 ms**, cold 484 ms, **48 MB of 256 MB**. That is seven
`max(timestamp)` reads across tables up to 735M rows, over mTLS to Hetzner, in
under a tenth of a second — proving on real data what §Design Decisions 5 only
measured locally on 2M rows: ClickHouse answers these from per-part min/max
metadata, not a column scan. It also sits at **0.19%** of the duration alarm's
48,000 ms threshold.

**Pre-flight lags** (all seven present, so no sentinel; tightest `4h` at 28% of
bound). Re-reading them 30 min later gave a clean confirmation of the
bucket-start model: every coarse tier had aged by *exactly* the elapsed 1,782 s,
while `1m` and `15m` had dropped, having rolled into new buckets. The sawtooth
that §Design Decisions 1 is built on, observed directly.

⚠️ **Deploying alarms emits one Slack message per alarm.** All 10 fired their OK
action on first resolution from `INSUFFICIENT_DATA`, so a 10-message burst
arrived within ~8 minutes of the deploy for something that was never broken.
Expected, one-time (alarms only sit in `INSUFFICIENT_DATA` when freshly created),
and **not worth suppressing** — dropping `addOkAction` would cost the recovery
signal, which is the half that proves a freeze has ended. Warn the channel before
deploying a batch of alarms.

## Remaining Work

None blocking. The one open acceptance criterion (leading indicators) is owned by
[[0181]], which now carries the measured grants and the two-line BE ask. Batch
that ask with the [[0136]] note already owed to BE rather than pinging twice.
