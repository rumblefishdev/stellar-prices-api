---
id: "0137"
title: "Rollup freshness alarm — a starved rollup MV reports success and nothing notices"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0136", "0104", "0109", "0056", "0143"]
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
      exceeds its bound while the fresh one does not. Detection latency is
      **15–30 min** (15-min cadence × 1 evaluation period), far inside "a day".
- [ ] Pending-mutation age and part-count checks are covered, here or in
      [[0109]], without duplicating each other. **DEFERRED** — these need
      `system.mutations` / `system.view_refreshes`, whose readability by the
      scoped mTLS user is unmeasured, and the runtime users are XML-managed by BE
      so we cannot `GRANT` them ([[0134]]). The primary signal deliberately needs
      no `system.*` access at all, so it ships without them. See §Remaining Work.
- [ ] Alarm routes somewhere a human reads, and a fire-test has passed.
      **HALF DONE.** Routing is wired — every alarm gets both an ALARM and an OK
      action on the `prices-{env}-ops-alarms` SNS topic that [[0056]] points at
      the Slack channel, confirmed in the template (`act=1/1` on all nine). The
      **fire-test has NOT been run**: it needs a deploy, and this task is
      prepare-only. See §Remaining Work.
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

## Remaining Work

Both are genuinely blocked on things this task cannot do, not descoped:

- **Fire-test after deploy** (AC 4). Deliberately not run — infra work here is
  prepare-only. Cheapest fire-test without waiting for a real freeze: set one
  tier's threshold to a value below its current lag, deploy, confirm Slack, then
  restore. Do it on `1M`, whose 45-day bound makes an accidental page harmless.
- **Leading indicators** (AC 3) — pending-mutation age, part counts,
  `view_refreshes` exceptions. Blocked on measuring whether the scoped mTLS user
  can read `system.mutations` and `system.view_refreshes`; spawned as **0179**.

## Notes

- Keep it boring. The failure this catches is "a number stopped moving"; it does
  not need to be clever, it needs to exist.
- The alarm has standalone value regardless of [[0136]]'s outcome — the same
  blind spot covers any future rollup stall, cadence regression, or upstream
  ingestion halt.
