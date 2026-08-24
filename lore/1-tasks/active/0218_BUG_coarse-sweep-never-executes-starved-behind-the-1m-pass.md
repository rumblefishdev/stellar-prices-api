---
id: "0218"
title: "The coarse-table sweep has never executed — it sits behind the 1m pass's `?` and is starved by the Lambda deadline"
type: BUG
status: active
related_adr: []
related_tasks: ["0215", "0114", "0111", "0026"]
tags: ["priority-high", "effort-small", "enrichment", "observability", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/main.rs"
history:
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      Spawned from 0215's post-fix verification. Found by reading the CloudWatch
      log of three consecutive invocations: no "enrichment pass complete" line
      and no "coarse sweep complete" line in any of them. Not a regression from
      the Caddy fix — the sweep was unreachable before it too, for a different
      reason.
  - date: 2026-08-24
    status: active
    who: okarcz
    note: >
      Activated. 0111 deployed to prod 2026-08-24 08:03:24 UTC and the sweep is
      now REACHED: "coarse sweep complete" appears on both post-deploy
      invocations with tables_swept=6, tables_failed=0, tables_skipped=0,
      rows_enriched=204,769 then 204,428, start_month=202607 end_month=202608.
      The 1m pass now returns in ~7.2 s (duration_ms in its own log line) and
      the invocation finishes in 26.7-29.6 s against the 300 s timeout, so
      remaining_ms is no longer saturated at 0.
      ⚠️ That satisfies AC 1 ONLY. The task's own argument stands: this is
      "starved by design, not by accident" — the sweep still sits after the 1m
      pass in one fixed budget, so the next growth in the 1m pass silently
      starves it again. ACs 2, 3 and 4 are untouched: a never-reached stage is
      still indistinguishable from one that ran and found nothing, the sweep's
      budget can still be driven to zero by the preceding stage, and nothing is
      published on an invocation that hits the deadline. Option 1 (separate
      EventBridge rule + Lambda) remains the preferred fix and is the only one
      that survives future growth.
  - date: 2026-08-24
    status: active
    who: okarcz
    note: >
      PR #244 merged and DEPLOYED to production (EventBridge then Observability).
      `prices-production-coarse-sweep` live on cron(30 * * * ? *), 512 MB, 300 s,
      reserved concurrency 1, async retries 0. ACs 1, 3 and 4 verified on
      production by induction; AC 2 is 2 of 3 states. Evidence: scheduled run
      14:30:47 UTC rows_enriched=1020 tables_swept=6 budget_ms=120000
      duration_ms=11348; CoarseSweepTableFailures 0.0 -> 1.0 and
      CoarseSweepDeadlineHit 0.0 -> 1.0 on adjacent buckets. Remaining: the
      "never reached" induction needs a ~3.5 h attended window (the
      -no-invocations alarm wants three empty 1-hour periods) — started 14:42
      UTC and REVERTED the same session rather than leave a production schedule
      disabled unattended. Deferred to 2026-08-25 working hours. Task stays
      active.
---

# The coarse sweep is unreachable, and has been on both sides of the fix

## Summary

`main.rs:167` is `let stats = pass.run().await?;`. The recurring coarse-table
sweep — [[0114]]'s remedy for the coarse tables carrying no USD values — sits
**after** that `?`. It has therefore never executed in production:

| period | why it never runs |
|---|---|
| ← 2026-08-21 14:46 | `run()` returned `Err(BadResponse(""))` on every invocation ([[0215]]); `?` propagated and the sweep was skipped |
| 2026-08-21 14:46 → | `run()` no longer errors, but the 1m pass consumes the whole budget and the Lambda is killed **inside** it (`Status: timeout`, `Duration: 300000.00 ms`) |

Confirmed by absence in CloudWatch across three consecutive attempts
(`RequestId 198d7653-…`, 15:17-15:36 UTC): neither
`"enrichment pass complete"` nor `"coarse sweep complete"` appears.

## The budget arithmetic cannot rescue it

`main.rs:195-203`:

```rust
const MARGIN_MS: u64 = 60_000;
let remaining_ms = lambda_deadline_ms.saturating_sub(now_ms).saturating_sub(MARGIN_MS);
let budget_ms = sweep_budget_secs.saturating_mul(1_000).min(remaining_ms);
```

The design is sound in isolation — the sweep defers rather than blowing the
timeout. But it assumes the 1m pass *returns*. A pass that runs to the hard
deadline leaves `remaining_ms` saturated at **0**, so even if the code were
reached the sweep would do nothing. The two failure modes compound.

⚠️ **The `time_budget_secs: 120` in the startup log is aspirational.** It is
logged from config at cold start (`main.rs:135`), so `"coarse sweep config
enabled=true tables=5 max_batches=20 time_budget_secs=120"` appears on every
invocation and reads like the sweep is configured and running. It is neither.

## Why this is worth its own task rather than a note on 0111

[[0111]] will free the budget, and that is necessary — but it is not sufficient
and it is not the whole defect:

1. **The failure is silent by construction.** The sweep's own errors are
   deliberately swallowed (`"coarse sweep failed (non-fatal)"`), which is correct
   for a best-effort stage — but there is no signal distinguishing *swept
   nothing*, *swept and failed*, and *never reached*. All three look identical
   from outside.
2. **It is starved by design, not by accident.** Any stage placed after an
   unbounded stage in a fixed budget gets whatever is left, which is zero when
   the first stage is time-bound. Fixing 0111 makes it work *today*; the next
   growth in the 1m pass silently starves it again.
3. **It gates [[0114]]**, which [[0111]] itself calls "the more serious defect"
   that "outranks this task".

## Implementation — options to cost

1. **Separate schedule.** Move the sweep to its own EventBridge rule and Lambda
   so it has an independent budget and cannot be starved by the 1m pass. Cleanest
   and removes the coupling permanently.
2. **Run it first, bounded.** Give the sweep its 120 s before the 1m pass rather
   than after. Preserves one Lambda, but inverts which stage absorbs the squeeze.
3. **Keep the order, add a floor.** Reserve the sweep's budget up front and make
   the 1m pass respect the reduced deadline. Smallest change; still one Lambda.

Option 1 is preferred and is the only one that survives future growth in the 1m
pass. Note that all three depend on the pass not running to the hard deadline —
so [[0111]] is a prerequisite for the sweep doing useful work, whichever is
chosen.

## Acceptance Criteria

- [x] `"coarse sweep complete"` appears in CloudWatch on a recurring schedule,
      with `rows_enriched` recorded before/after.
      → scheduled run 2026-08-24 14:30:47 UTC, `rows_enriched=1020`; 204,769
      (embedded, backlog) → 2,924 → 1,020 (standalone, steady state).
- [ ] A stage that is never *reached* is distinguishable in logs and metrics from
      one that ran and found nothing — verified by inducing, not inferred.
      → **2 of 3 states induced.** *ran* and *ran and failed* done
      (`CoarseSweepTableFailures` 0.0 → 1.0 on adjacent buckets); *never reached*
      needs a ~3.5 h attended window, deferred to 2026-08-25 (see ⏳ HANDOFF).
- [x] The sweep's budget cannot be reduced to zero by the preceding stage.
      → `budget_ms=120000`, full and unreduced; there is no preceding stage left.
- [x] `EnrichmentPassDurationMs` and the sweep's own metric are both published on
      an invocation that hits the Lambda deadline, so a starved run is visible.
      → **RESTATED, not ticked as written** — the split dissolved the
      shared-invocation premise. Verified as `CoarseSweepDeadlineHit=1` on a
      budget-truncated run. See "Two gaps to state, not tick".

## Out of scope

- The full-table scan that consumes the budget — that is [[0111]].
- The coarse tables' missing USD values themselves — that is [[0114]].

---

## Deploy + verification record (2026-08-24)

### Deployed

`Prices-production-EventBridge` then `Prices-production-Observability`, in that
order — the two new alarms name `prices-production-coarse-sweep`, and an alarm
pointed at a function that does not exist yet settles to `INSUFFICIENT_DATA`
instead of watching anything ([[0204]]'s failure).

`prices-production-cleanup` read `DISABLED` **before and after** both deploys.

Post-deploy state: rule `ENABLED cron(30 * * * ? *)`, function 512 MB / 300 s,
all six coarse tables in `COARSE_SWEEP_TABLES`.

⚠️ **Build note.** The wide `cargo lambda build` re-triggered the feature-
unification diff on `RollupFreshnessProbeFunction` (see
[[lambda-asset-diff-is-feature-unification]]). Rebuilding `-p
rollup-freshness-probe` alone restored the prod-matching 12,039,704-byte
bootstrap and it dropped out of the diff; the shipped EventBridge diff was then
exactly the seven `CoarseSweep*` additions plus `EnrichmentFunction` losing its
four `COARSE_SWEEP_*` variables.

⚠️ The 8 changes `cdk diff` omits as "likely mangled non-ASCII" on this stack are
rule/alarm descriptions containing `→`. Read once with `--strict`; benign, and
`CleanupRule` is not among them.

### AC 1 — satisfied

Scheduled run, `RequestId 693bb544-2136-444c-8a69-111958416b8f`, 2026-08-24
14:30:47 UTC (EventBridge-triggered, distinct from the manual probe invoke at
14:18:26):

```
rows_enriched=1020  rows_remaining=10115084  tables_swept=6
tables_failed=0  tables_skipped=0  deadline_hit=false
duration_ms=11348  budget_ms=120000
```

`rows_enriched` before/after: **204,769** on the last enrichment-embedded run
(08:03 UTC, backlog draining) → **2,924** then **1,020** standalone. Steady-state
drip, not a regression.

### AC 3 — satisfied, and `budget_ms` is the evidence

`budget_ms=120000` is the **full configured budget**. Under the old design this
field was whatever the 1m pass left over, which is how it reached zero. There is
no preceding stage left to reduce it. 11.3 s of a 120 s budget and a 300 s
timeout, cold start included — every hourly run pays one, reserved concurrency 1
does not keep it warm.

### Measured floor — do NOT alarm on `rows_remaining`

`rows_remaining` **rose** between consecutive runs (10,113,610 → 10,115,084)
while 1,020 rows were enriched: new candles arrive faster than the priceable
remainder is filled, because most of what remains has no USD reference at all.
This is the permanent floor ([[0114]]'s surface, not this task's), and it is why
`CoarseSweepRowsRemaining` is deliberately not published. Per-table residual at
14:30: `_15m` 4,810,151 · `_1h` 2,935,460 · `_4h` 1,622,260 · `_1d` 586,347 ·
`_1w` 119,283 · `_1M` 41,583.

Also observed naturally: month 202607 enriched **0** across all six tables with
`zeros_before == zeros_after` — the "ran and found nothing" state occurring
without being induced.

### AC 2 state 2 of 3 — "ran and failed" — INDUCED 2026-08-24 14:32 UTC

`price_ohlcv_zzznope` appended to `COARSE_SWEEP_TABLES`. It passes
`is_coarse_table` (starts with `price_ohlcv_`, is not `_1m`), so it reaches the
driver and errors there rather than being filtered as a config skip.

```json
{"failed":["price_ohlcv_zzznope"],
 "swept":["price_ohlcv_15m","price_ohlcv_1h","price_ohlcv_4h",
          "price_ohlcv_1d","price_ohlcv_1w","price_ohlcv_1M"]}
```

🔑 **The discriminator, in one series, on adjacent datapoints:**

| `CoarseSweepTableFailures` | bucket | run |
|---|---|---|
| **0.0** | 14:27 UTC | scheduled run, healthy |
| **1.0** | 14:32 UTC | induced failure |

Invocation still returned `StatusCode: 200` and all six real tables swept — one
bad table does not kill the sweep, which is the intended isolation.

### AC 4 — INDUCED 2026-08-24, `COARSE_SWEEP_TIME_BUDGET_SECS=1`

```json
{"deadline_hit":true,
 "swept":["price_ohlcv_15m"],
 "deferred":["price_ohlcv_1h","price_ohlcv_4h","price_ohlcv_1d",
             "price_ohlcv_1w","price_ohlcv_1M"]}
```

The deadline is checked *before* each table, so table 1 always runs and the rest
defer — the run is cut short and **still publishes**. That is the whole point: a
starved run is a datapoint (`CoarseSweepDeadlineHit=1`), not silence. Under the
old design a starved sweep produced nothing and was indistinguishable from one
that never ran.

| `CoarseSweepDeadlineHit` | bucket | run |
|---|---|---|
| **0.0** | 14:30 UTC | scheduled run, full 120 s budget |
| **1.0** | 14:35 UTC | induced, 1 s budget |

Env restored from the saved copy immediately afterwards and diffed against it —
byte-identical. A redeploy also restores the CDK values.

### ⏳ HANDOFF — AC 2 state 3 of 3, DEFERRED to 2026-08-25 working hours

Started 2026-08-24 14:42 UTC and **reverted the same session**: the alarm needs
~3.5 h and the operator had to stop. The rule was re-enabled rather than left off
unattended overnight — an indefinitely disabled production schedule is worse than
an unproven AC, and it would have reproduced this task's own defect.

`prices-production-coarse-sweep-no-invocations` needs **three consecutive empty
1-hour `Invocations` periods** (`treatMissingData: BREACHING`). Buckets align to
the clock hour and the hour you disable in still holds a run, so:

🔑 **Disable right after a `:30` run** — no partial hour wasted. Disable ~09:35 →
empty hours 10, 11, 12 → alarm about **13:05-13:15 UTC**.

```bash
# 1. start it, just after a :30 run
aws events disable-rule --name prices-production-coarse-sweep --region eu-central-1
aws events describe-rule --name prices-production-coarse-sweep \
  --region eu-central-1 --query State --output text     # expect DISABLED

# ... ~3.5 h. SET A TIMER — nothing re-enables it automatically ...

# 2. read the evidence (an OK → ALARM transition IS the AC)
aws cloudwatch describe-alarm-history \
  --alarm-name prices-production-coarse-sweep-no-invocations \
  --history-item-type StateUpdate --max-records 10 --region eu-central-1 \
  --query 'AlarmHistoryItems[].[Timestamp,HistorySummary]' --output text

# 3. corroborate: no CoarseSweepRuns datapoints across the empty hours
aws cloudwatch get-metric-statistics --namespace Prices/Enrichment \
  --metric-name CoarseSweepRuns --dimensions Name=Environment,Value=production \
  --start-time <first empty hour>Z --end-time <last empty hour>Z \
  --period 3600 --statistics Sum --region eu-central-1 --output text

# 4. RE-ENABLE — do not skip
aws events enable-rule --name prices-production-coarse-sweep --region eu-central-1
aws events describe-rule --name prices-production-coarse-sweep \
  --region eu-central-1 --query State --output text     # expect ENABLED

# 5. after the next :30 run, all three alarms back to OK — the final test
aws cloudwatch describe-alarms --region eu-central-1 \
  --alarm-names prices-production-coarse-sweep-errors \
                prices-production-coarse-sweep-duration-near-timeout \
                prices-production-coarse-sweep-no-invocations \
  --query 'MetricAlarms[].[AlarmName,StateValue]' --output table
```

⚠️ If step 2 shows **no** `OK → ALARM` transition, the induction did not work —
do **not** tick AC 2. Check the rule really stayed disabled and that step 3
returned no datapoints.

⚠️ An ops page fires on the ALARM transition and again on recovery. Tell the team
it is a planned induction, or someone will chase it.

**Cost while disabled:** staleness only. `mv_ohlcv_1m_to_15m` still writes the
coarse tables every 60 s; what pauses is the USD backfill, ~1-2 k rows/hour
against a 10.1 M no-reference floor, and the 2-month lookback recovers a gap of
hours or days on the next run. ⚠️ A gap of *weeks* would permanently lose `_15m`
rows to its 30-day retention. An ops page fires on the ALARM transition and again
on recovery — expected, not an incident.

### Then close

1. Tick ACs 1, 2, 3 above; **restate AC 4** rather than ticking it as written (see
   the gaps below).
2. `/lore-framework-tasks` **before** any status change; status is `completed`,
   never `done`.
3. `/lore-framework-git` for the commit, then archive.
4. Drop the [[coarse-sweep-rule-disabled-must-reenable]] memory once the rule is
   back ENABLED.

### Two gaps to state, not tick

- `CoarseSweepFailedRuns=1` (a whole-run `Err`) requires the window query itself
  to fail, i.e. an unreachable ClickHouse — not safe to induce on a cluster BE
  owns 96% of. Covered by the unit test `a_failed_run_is_a_datapoint_not_silence`.
  **Test-covered, not prod-induced.**
- **AC 4's `EnrichmentPassDurationMs` clause assumed the sweep shared an
  invocation with the 1m pass.** The split dissolved that premise: the sweep has
  its own `CoarseSweepDurationMs` and its own `-duration-near-timeout` alarm, and
  a hard Lambda kill publishes nothing by construction in either function. The AC
  should be restated, not ticked as written.

## Design Decisions

### From Plan

1. **Option 1 — separate Lambda + EventBridge rule.** Decided before
   implementation; the deciding reason is the `?` after `pass.run().await`, not
   starvation. Option 3's up-front budget reservation also survives growth, but
   only a separate invocation removes the dependency on the 1m pass *succeeding*.

2. **A failed sweep fails its own invocation.** The old stage swallowed errors to
   protect the 1m pass. There is no pass to protect here, and a swallowed error
   was indistinguishable from a run that swept nothing — the defect itself.

### Emerged

3. **Deploy EventBridge before Observability.** Not specified anywhere. The two
   new alarms reference the function by name; deploying alarms first leaves them
   at `INSUFFICIENT_DATA` watching nothing — [[0204]]'s "10 of 13 alarms blind"
   failure. Order is now recorded in the deploy record above.

4. **Rebuilt `rollup-freshness-probe` alone to keep it out of the diff.** The
   wide build changed its binary through cargo feature unification, with no
   source change. Shipping it would have been harmless but would have made the
   deploy diff larger than the change it represents. See
   [[lambda-asset-diff-is-feature-unification]].

5. **AC 4 restated rather than ticked.** Its `EnrichmentPassDurationMs` clause
   assumed the sweep shared an invocation with the 1m pass. Ticking it as written
   would claim something not verified; the intent — a starved run is visible — is
   verified, under the split's own metric.

6. **`CoarseSweepFailedRuns=1` left test-covered, not prod-induced.** Forcing a
   whole-run `Err` needs an unreachable ClickHouse, which is not safe to induce
   on a cluster BE owns 96% of. Recorded as a gap rather than quietly skipped.

7. **The "never reached" induction was reverted mid-flight.** Started 14:42 UTC,
   then re-enabled the rule when the session had to end before the alarm could
   breach. An unattended production schedule left disabled is worse than an
   unproven AC — and would have reproduced this task's own defect.

## Issues Encountered

- **`cdk diff` showed an unexplained `RollupFreshnessProbeFunction` asset
  replacement.** Not a stale prod build — cargo feature unification from the
  build-set shape, which changed again when `coarse-sweep-worker` joined the
  list. Fixed by building that crate alone (12,039,704 B, byte-identical to
  prod). Known trap; cost three days under [[0111]], minutes here.

- **`cdk diff` hides 8 changes on this stack** as "likely mangled non-ASCII".
  Read once with `--strict`: they are rule and alarm descriptions containing `→`.
  Text only, recurs every diff. `CleanupRule` is *not* among them, so a genuine
  `State` change there would still be visible.

- **The full production diff carries other people's undeployed work** — portal
  Discord OAuth across Secrets / Compute / ApiGateway / PortalHosting, including
  a custom resource marked "may be replaced". Per-stack deploy targets avoided
  it; `make deploy-production` would have shipped it as a side effect.

- **`rows_remaining` rises between runs** (10,113,610 → 10,115,084 while 1,020
  rows were enriched). Not a regression: new candles outpace the priceable
  remainder because most of what is left has no USD reference. This is why
  `CoarseSweepRowsRemaining` is deliberately not published — do not alarm on it.

## Future Work

- The sweep iterates `cfg.tables` in fixed order with no rotation, so if any
  table ever consumes the budget the tail starves permanently. Raised as the
  structural half of a rejected PR #244 review finding; not yet a task. Worth
  spawning if `CoarseSweepDeadlineHit` is ever sustained in steady state.
- The residual zero counts per table are [[0114]]'s surface, not this task's.
