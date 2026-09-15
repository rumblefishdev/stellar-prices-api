---
id: "0243"
title: "No alarm watches current_prices freshness — a dead mv_current_prices serves a frozen price behind a healthy HTTP 200"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0178", "0137", "0204", "0218"]
tags:
  [
    "priority-high",
    "effort-small",
    "observability",
    "clickhouse",
    "refreshable-mv",
    "read-surface",
    "milestone-M2",
  ]
milestone: 2
links:
  - "../../../packages/rollup-freshness-probe/src/main.rs"
  - "../../../packages/prices-clickhouse/schema/current.sql"
history:
  - date: 2026-08-31
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0178]]'s deploy-runbook work. Writing the rollback plan for
      that task's DROP + recreate required knowing what would page if the
      recreate failed, and the answer measured on 2026-08-31 is nothing:
      rollup-freshness-probe covers the price_ohlcv_* tiers only, and no
      Observability construct references current_prices. Kept out of 0178
      because that task is a data-correctness fix and this is an ops gap that
      predates it and outlives it.
  - date: 2026-09-14
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Item 11 of Oskar's backlog summary shared on 2026-09-14, beside
      0214 + 0223 (item 10), and started first because it is the smallest of the
      three and the only one whose failure a consumer sees: a stopped
      mv_current_prices serves a frozen price behind HTTP 200. Two facts read
      from the code that shape the work. rollup-freshness-probe builds its query
      around `max(timestamp)`, and current_prices has no `timestamp` column
      (init.sql:155), so the probe needs a per-table age column — `updated_at`
      here. And `updated_at` is `now()` at every refresh, so this alarm catches
      a dead refresh, not stale input; stale input stays the rollup alarms' job,
      as the 2026-09-14 Galexie stall showed when rollup-freshness-1m fired.
---

# `current_prices` can freeze and nothing notices

## Summary

`prices.current_prices` is written by exactly one writer, the refreshable MV
`mv_current_prices`, on a 1-minute schedule. **Nothing monitors whether that
writer is still running.** If it stops, the table keeps its last-written rows
and `GET /price` keeps returning HTTP 200 with a plausible price that has
silently stopped moving.

## Why this is worse than an outage

Every consumer-side health signal stays green. The endpoint responds, the
status code is 200, the payload validates, the price is a normal number. Only
`updated_at` betrays it — and `updated_at` is the MV's refresh time, so it stops
advancing exactly when the writer dies.

There is prior art for the failure mode in this repo: [[0215]]'s enrichment pass
failed on **every invocation for 26 days** while ClickHouse logged `QueryFinish`
and every data signal read normal. The lesson recorded there — that a healthy
exit status is not evidence of work done — applies unchanged here.

## Measured 2026-08-31

- `rollup-freshness-probe` watches the `price_ohlcv_*` tiers. `grep` for
  `current_prices` in its source: no match.
- `grep` for `current_prices` / `CurrentPrices` across `infra/`: no match.
- So the gap is total, not partial.

## Implementation sketch

The probe already has the right shape — reuse it rather than writing a second
one, per the working agreement on reusing tested code.

- Metric: `now() - max(updated_at)` on `prices.current_prices FINAL`, in seconds.
- Bound: the refresh interval (1 min) plus refresh duration, with headroom.
  Follow [[0137]]'s rule — the bound is bucket width **plus** the feeding
  refresh, not the width alone.
- ⚠️ The probe reads as `prices_writer`, for which `system.*` is denied and
  cannot be granted. Do not reach for `system.view_refreshes` without checking
  that wall first — see [[0204]]'s and [[0182]]'s experience.
- ⚠️ Never run the alarm at `1/1`. Cf. the FILL/sliding-window notes from 0218.

## Acceptance Criteria

- [ ] An alarm exists that fires when `current_prices` stops advancing.
- [ ] Its bound is derived from the refresh interval, not guessed, and the
      derivation is written down.
- [ ] Verified by INDUCING the condition, not by reading the definition —
      the standard this repo has held since [[0204]].
- [ ] Routed to the same Slack channel as the existing ops alarms.
- [ ] Does not depend on any `system.*` table.

## Notes

- [[0178]] is the task that uncovered this. Its runbook compensates for the gap
  manually (verify `updated_at` advances across two refresh cycles before
  declaring the deploy good); once this alarm exists that manual step can be
  dropped from future MV recreates.
- Worth checking at the same time whether `prices.assets` and
  `prices.asset_supply` have the same blind spot — both are single-writer
  tables feeding the same read surface.

## Design decisions — 2026-09-14

Planned before any code; the implementation lands on a branch.

1. **A separate check, not an eighth tier.** `ROLLUP_TIERS`' empty-tier sentinel
   rule is positional (fine → coarse) and 11 unit tests pin the list;
   `current_prices` has no `timestamp` column. New module `current_prices.rs`
   beside `disk.rs` / `mv_drift.rs`, with its own query and its own block in the
   handler, so a failed read cannot suppress another check.
2. **An empty table breaches.** The MV emits one row per asset with a 1-minute
   candle in the last 24 h (`current.sql:486-495`) and runs in REPLACE mode, so an
   empty table means the API serves nothing. No `HAVING` gate: `count() = 0`
   publishes `EMPTY_TIER_SENTINEL_SECONDS`.
   ⚠️ **Empty has two causes** (review of PR #315): a broken writer, or an input
   gone empty — no 1-minute candle for 24 h, i.e. ingestion down for a day, with
   the USDC rate stale too. The alarm description names both and sends the
   on-call to `rollup-freshness-1m` first, so a healthy view is not restarted
   for an ingestion outage.
3. **No `FINAL` — this corrects the sketch above.** The version column is
   `updated_at` itself (`ReplacingMergeTree(updated_at)`), so the newest row
   survives any merge and `max(updated_at)` cannot differ. An IT pins it.
4. **Same metric, new dimension value:** `Prices/Rollup` `RollupLagSeconds` with
   `Table=current_prices`. No new publish code, no IAM change (the grant is
   conditioned on the namespace only).
5. **Own config key and own alarm.** `validateConfig` requires `rollupLagSeconds`
   to cover exactly the seven tiers, and the rollup loop would name this
   `rollup-freshness-current_prices` with a bucket-shaped description. New key
   `opsAlarms.currentPricesFreshnessSeconds`, alarm
   `prices-{env}-current-prices-freshness`.
6. **`treatMissingData: MISSING`.** The probe always publishes this datum, so its
   absence means a dead probe — already covered by the probe's worker-health
   alarms — and must not resolve a latched ALARM (the rule at
   `observability-stack.ts:1194-1207`).
7. **Threshold 900 s**, derived below. No dashboard widget: the alarm joins the
   derived alarm strip on its own.

### Bound derivation

| quantity | value | source |
|---|---|---|
| refresh interval | 60 s | `current.sql:156` |
| refresh duration, measured | 85–267 ms | [[0072]], [[0135]] |
| worst accepted refresh | 40 s — the runbook's stop line | `docs/runbooks/0072-current-prices-mv-rollout.md:160-167` |
| healthy peak (interval + worst refresh) | **100 s**, enforced at synth | — |
| bound | **900 s** = 15 missed refreshes | the `1m` tier's bound at the same probe cadence |

`updated_at` is the refresh START, so a healthy age sawtooths from 0 to 60 s. A
stall under 15 min never pages; one of 30 min or more always does (bound plus one
15-min probe period). A tighter bound buys nothing: the probe cadence dominates
detection.

### What it catches, and what it does not

It watches the WRITER, not its input. During the 2026-09-14 Galexie stall
`updated_at` kept advancing by construction — the MV reruns on stale candles — and
`rollup-freshness-1m` fired. That is the intended division of labour.

### Verification — decided 2026-09-14

A real `SYSTEM STOP VIEW` on production freezes `GET /price` for ~16 min on the
shared cluster while the Milestone 2 reviewer key is live, so the production
induction is **deferred to [[0283]]**. This task proves the chain without it and
records the induction criterion as **"Test-covered, not prod-induced"** ([[0218]]'s
standard):

- an IT stops a real `mv_current_prices` locally and asserts the age grows;
- a temporary action-less clone alarm on the real metric must reach ALARM, proving
  the alarm reads exactly what the probe publishes;
- `set-alarm-state` ALARM → OK on the real alarm proves the Slack routing.

### Findings for the Notes above

- `prices.asset_supply` has one hourly writer and the same blind spot — spawned as
  [[0284]].
- `prices.assets` is **not** single-writer: every upsert stamps `updated_at`, and
  asset-discovery re-emits ~204k rows an hour, so its `max(updated_at)` measures
  asset-discovery activity rather than freshness. No alarm.
- The [[0178]] manual check (`updated_at` advancing across two refresh cycles)
  stays for attended MV recreates: it answers in ~2 min, this alarm in 15–30 min.
