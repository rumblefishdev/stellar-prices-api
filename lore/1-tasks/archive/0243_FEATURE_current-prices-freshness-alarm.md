---
id: "0243"
title: "No alarm watches current_prices freshness — a dead mv_current_prices serves a frozen price behind a healthy HTTP 200"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0178", "0137", "0204", "0218", "0283", "0284"]
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
  - date: 2026-09-15
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped and verified on production the same day. PR #315 (one review from
      okarcz, its medium finding folded in), 5 unit tests and 2 ClickHouse ITs,
      EventBridge deployed 09:06 UTC and Observability 09:14 UTC.
      prices-production-current-prices-freshness went INSUFFICIENT_DATA -> OK on
      a real 56 s reading, an action-less clone alarm proved the alarm reads the
      metric the probe publishes, and a set-alarm-state round trip delivered
      ALARM and OK to the ops Slack channel. 4 of 5 criteria met; the production
      induction (stopping mv_current_prices for real) is deferred to [[0283]]
      and recorded as "test-covered, not prod-induced". Spawned [[0283]] and
      [[0284]]; [[0181]] noted that Table=current_prices is taken.
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

- [x] An alarm exists that fires when `current_prices` stops advancing.
      `prices-production-current-prices-freshness`, live since 2026-09-15
      11:14:15 CEST. See the deploy record below.
- [x] Its bound is derived from the refresh interval, not guessed, and the
      derivation is written down. See the bound-derivation table.
- [ ] Verified by INDUCING the condition, not by reading the definition —
      the standard this repo has held since [[0204]]. **Test-covered, not
      prod-induced** ([[0218]]'s wording): an IT stops a real
      `mv_current_prices` and asserts the age grows, and an action-less clone
      alarm on the real production metric reached ALARM. Stopping the view on
      production is deferred to [[0283]].
- [x] Routed to the same Slack channel as the existing ops alarms. Three
      transitions on 2026-09-15 each executed the ops SNS action; see below.
- [x] Does not depend on any `system.*` table. The query reads
      `prices.current_prices` only; `system.view_refreshes` appears in the alarm
      description as human diagnosis, never on the alarm path.

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

## ✅ DEPLOYED — 2026-09-15, both stacks

Merged as PR #315 (`00e38a8` on `develop`, squashed) after Oskar's review; the
review's medium finding is folded in (`43bdf31`, see below). Deployed from
`develop` at `00e38a8` by stkrolikiewicz, EventBridge first, then Observability.

### Build — the macOS trap worth recording for [[0239]]

`tools/scripts/lambda-assets.sh` uses `mapfile`, which the system `bash 3.2` on
macOS does not have, so the canonical crate list cannot be produced the way CI
produces it. The list was derived with the script's own `grep` in zsh instead,
and checked to be exactly 11 crates before building:

```
grep -rhoE "'\.\./target/lambda/[^']*'" infra/src | tr -d "'" \
  | sed 's#\.\./target/lambda/##' | sed '/^$/d' | sort -u
```

Group build of all 11 crates, `--release --arm64 --features lambda`, 225 s,
rustc 1.97.1 / cargo-lambda 1.9.1 / zig 0.16.0 (the CI pins). The probe binary
carries the new query (`grep -a -c 'FROM current_prices'` → 1; `strings` on
macOS refuses the ELF, `grep -a` does not).

### EventBridge — 09:04:42 → 09:06:17 UTC, deploy 22.04 s

`cdk diff Prices-production-EventBridge --method=template --strict` showed, and
nothing else:

| change | count | reading |
|---|---|---|
| Lambda code asset | 9 | only `rollup-freshness-probe` is a source change |
| Rule + alarm descriptions | 8 | mangled `?` restored to `—`, `→`, `§` |
| CDK metadata | 1 | cosmetic |

The other eight binaries are **build churn, not new code**: no commit has
touched those crates or their shared dependencies since the 2026-09-14 15:06:51
UTC deploy, and a Rust binary embeds the building machine's paths. Same shape as
[[0277]]'s and [[0228]]'s deploys of this stack.

🔒 `prices-production-cleanup` **DISABLED before and after**, and the diff
touched no Rule `State`.

Probe invoked by hand straight after: `StatusCode 200`, no `FunctionError`, and
the new field present — `current_prices: {rows: 3448, age_seconds: 56}`. The
metric registered as `Prices/Rollup RollupLagSeconds` with
`{Environment=production, Table=current_prices}`. Probe errors since deploy: 0.

### Observability — 09:13:36 → 09:14:37 UTC, deploy 27.22 s

One alarm created, `DashboardAlarmCount` 51 → 52, dashboard body updated, and 32
alarm descriptions repaired (the same mangled-text class). No thresholds,
actions, metrics or IAM touched.

Deployed alarm, read back from production: threshold 900, `GreaterThanThreshold`,
period 900, `Maximum`, 2 evaluation periods, 1 datapoint to alarm,
`treatMissingData: missing`, dimensions `Environment=production` +
`Table=current_prices`, one alarm action and one OK action.

### The evidence, from the alarm's own history

| time (CEST) | event |
|---|---|
| 11:14:15 | alarm created |
| 11:15:19 | INSUFFICIENT_DATA → OK on a real datapoint (56 s), SNS action executed |
| 11:17:09 | **clone alarm** `tmp-0243-current-prices` (no actions, threshold 0) reached ALARM on the same series |
| 11:17:20 | OK → ALARM, set by hand for the routing test, SNS action executed |
| 11:19:03 | ALARM → OK, restored by CloudWatch's own next evaluation, SNS action executed |

All three actions report `Successfully executed action
arn:aws:sns:eu-central-1:750702271865:prices-production-ops-alarms`, which is the
ops → Chatbot → Slack path the other alarms use. The clone was deleted
immediately after; `describe-alarms --alarm-name-prefix tmp-0243` returns none.

✅ **Delivery confirmed in the channel, not only in the alarm history.** All
three messages arrived: the 11:15 OK, then the ALARM carrying the state reason
`task 0243 routing test — manual state set, NOT a real breach; current_prices is
healthy`, then the 11:19 OK. The alarm description renders correctly in Slack,
em dashes and all — the mangled-text repair this deploy carried.

The clone is what proves the alarm is not blind: it read the same namespace,
metric and dimensions the probe publishes and breached on a real value. A
dimension typo — [[0204]]'s "10 of 13 alarms blind" — would have left it in
INSUFFICIENT_DATA forever.

### Integration tests

Both `#[ignore]` ITs pass locally against ClickHouse 26.3.10.60 in 10.37 s. The
second one stops and restarts a real `mv_current_prices` with `SYSTEM STOP VIEW`
/ `START VIEW`, asserts the age grows one-for-one with the clock while stopped
and drops back after `REFRESH`. CI does not run them ([[0275]]).

### Incidental measurement

The same probe run read ClickHouse free disk at **21.18 %**, just above the 20 %
bound of `prices-production-ch-disk-free`. Not firing, little headroom, worth an
eye.
