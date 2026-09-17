---
id: "0291"
title: "pool_registry has not learned a pool since 2026-07-06 — live forgets newer pools on every cold start and drops their trades"
type: BUG
status: active
related_adr: []
related_tasks: ["0285", "0286", "0282", "0078", "0101", "0256", "0069", "0080", "0290"]
tags: [layer-indexing, priority-high, effort-small, amm, aquarius, soroswap, ingestion, data-correctness, clickhouse]
links:
  - "../active/0285_RESEARCH_pool-registry-does-not-match-what-is-trading/notes/S-classification-2026-09-17.md"
  - "../../../packages/prices-ledger-processor/src/main.rs"
  - "../../../docs/runbooks/seed-pool-registry.md"
  - "https://github.com/rumblefishdev/stellar-prices-api/pull/322"
history:
  - date: 2026-09-17
    status: backlog
    who: okarcz
    note: >
      Spawned from 0285. The table's only writer is the history backfill, whose
      data ends 2026-07-06; the live processor only reads it. 22 Aquarius pools
      (34,684 trades) and 10 Soroswap pools created since are missing. Blocks
      0286 phase 3 for the live-era AMM months (its precondition 9).
  - date: 2026-09-17
    status: active
    who: okarcz
    note: >
      Activated. Blocks 0286 phase 3 for AMM months from 2026-07; start with
      the one-off seed, then live persistence, then observability.
  - date: 2026-09-17
    status: active
    who: okarcz
    note: >
      Implemented, PR #322 (branch
      fix/0291_pool-registry-is-stale-since-the-backfill-ended), NOT merged or
      deployed. Live persists learned pools (new rows only, before the cursor
      passes the factory event); events-backfill --discover-pools does the
      one-off; UnregisteredPoolEvents metric + alarm. Real gap is 42 pools
      (27 aquarius, 14 soroswap, 1 phoenix), all after ledger 63M. Local
      end-to-end against a copy of the prod registry: 42 written, re-run 0.
      Workspace 1,115 tests green. Operator runbook below.
---

## 📊 STATUS — 2026-09-17 · code done, PR #322 open, prod steps owed

- ✅ Live persistence, one-off tool, counter + alarm — written, tested, **PR #322**.
- ⏳ **Operator:** discover-pools on prod, then deploy (runbook below).
- ⏳ AC 4 needs a full day after the deploy — check with [[0282]]'s 2026-09-19
  measurement.
- 🔑 Root cause is [[0256]]: the registry's designed maintainer (the
  asset-discovery scan, task 0069) has never run on production. 0291 makes the
  live processor the maintainer, so 0256 can drop the scan without losing it.

# `pool_registry` is stale since the backfill ended

## Summary

`prices.pool_registry` was last written **2026-08-11**, by the history backfill,
whose data ends **2026-07-06**. The live processor loads it at cold start and
never writes it (`prices-ledger-processor/src/main.rs:80-94`). A pool created
later is known to live only while a container that saw its factory event is
warm; after any cold start its trades are silently dropped — an unknown
contract's `trade` is not even counted as unresolved.

Missing today (live era, see [[0285]]'s note):

| venue | pools | events |
| --- | --- | --- |
| aquarius | 22 | 34,684 `trade` (~5% of Aquarius) |
| soroswap | 10 | 267 `swap` |

## Why it matters now

- **[[0286]] phase 3** reprices live-era AMM months from the registry after
  `DROP PARTITION` — with a stale registry it would delete candles for these
  pools and not put them back. **Fix before phase 3 reaches 2026-07.**
- [[0282]] and [[0101]] use the registry as their denominator.

## Implementation

- **One-off:** seed the missing pools from the factory events since
  2026-07-06 (`add_pool`, `new_pair`, Phoenix `create`). ⛔ Corrected
  2026-09-17: `events-backfill` does NOT learn pools — it reads only the events
  of contracts already registered (`run.rs:159`), so a registry-only run can
  never find a missing one. Built `--discover-pools` instead (below).
- **Durable:** have the live processor persist pools it learns from factory
  events (it already does the same for assets, task 0132), so a cold start
  never forgets one.
- **Observable:** count and alarm on `trade`/`swap` events from contracts the
  live path cannot classify, including `trade` (today only `swap` is counted,
  and live never writes `unresolved_pools`) — this also closes [[0282]]'s open
  observability criterion.

## Acceptance Criteria

- [ ] The 22 Aquarius and 10 Soroswap pools are in `pool_registry`.
      → the full gap is **42** (27 aquarius, 14 soroswap, 1 phoenix; 0285 only
      counted pools that had traded). Tool ready and verified locally; ⏳ the
      prod run is the operator's (runbook step 2).
- [ ] A pool created after deploy is in the table within one reconcile run, and
      survives a forced cold start (test + prod check).
      → ✅ test (`tests/pool_registry_persist.rs`); ⏳ prod check after deploy
      (runbook step 5).
- [ ] Unclassifiable pool events are counted and alarmed on the live path.
      → ✅ in code (`UnregisteredPoolEvents` +
      `prices-production-ledger-processor-unregistered-pool`); ⏳ deploy.
- [ ] Aquarius raw (registry-joined, all pool wasms) vs stored for a full day
      after the fix shows no stored > raw residue. → ⏳ first full day after
      the deploy.

## Findings — 2026-09-17 (production, `dev_read`)

### The gap is 42 pools, all created after ledger 63,000,000

Factory events matched against `pool_registry`, whole Soroban era:

| venue | created (all time) | missing | missing before 63M |
| --- | --- | --- | --- |
| aquarius | 515 (`add_pool`) | **27** (16 concentrated, 7 constant, 4 stable) | 0 |
| soroswap | 214 (`new_pair`) | **14** | 0 |
| phoenix | 20 (`create`/`liquidity_pool`) | **1** (created ≥ 64M) | 0 |

The first missing pool is at ledger 63,370,873; the backfill's last day
(2026-07-06) ends at 63,361,521.

### Only 33 of the 42 have traded, and nothing else looks like a pool

Contracts outside the registry whose events match the counter's shapes, ledgers
63.36M → 64.64M: **23 aquarius (47,586 `trade`) + 10 soroswap (227 `swap`)**,
and **every one is among the 42 with the same venue**. So once they are seeded
the new alarm reads silent. Routers and the v3-style venue ([[0290]]) do not
match.

### ⚠️ Phoenix and Soroswap factory events have NULL `signature`

Both use **String** topics (`["create","liquidity_pool"]`,
`["SoroswapFactory","new_pair"]`), so BE's `signature` column is NULL — the
same gotcha 0285 hit for swaps. A `signature IN (…)` filter finds **0** of the
20 Phoenix pools. `learn_factory` reads the topic value whatever its type, so
live was never affected; the discover query matches topic values.

### Aquarius `concentrated` pools decode correctly ([[0080]])

The registry already held 31 concentrated pools (learned by the backfill), so
persisting new ones changes no policy. Their `trade` event has the exact shape
`AquariusPoolExtractor` reads: `[trade, sold, bought, trader]`, data
`[sold_amt, bought_amt, fee]`. A sample's 12,107,191 / 1,264,886 = **9.57**
against the same pool's `pool_state` tick −22,580 → 1.0001^22,580 ≈ **9.56**.

### ⚠️ The `dev_read` hourly quota is 2 TiB — this task's scans used it up

Parsing `topics_xdr` across the era exhausted it at ~14:45 UTC (`Code: 201
QUOTA_EXCEEDED`, reset 15:00). If other people read as `dev_read`, they were
blocked for ~15 min. The discover query costs 2–4 s per 320k-ledger chunk;
keep its range to `63000000` → tip.

## Implementation Notes

PR #322, two commits:

- `prices-ingest-core`: `Registries::pool_rows_unpersisted` (rows that differ
  from a `contract_id`-keyed snapshot); `OhlcvWriter::write_pool_rows`
  (`write_pool_registry` now delegates); public `learn_factory_event`;
  `LedgerSoroban::unregistered_pool_events` fed by `unregistered_pool_venue`
  (Aquarius `trade` with address topics, `SoroswapPair`/`swap`, Phoenix
  `swap`/`sell_token` — one match per trade).
- `prices-ledger-processor`: `CandleSink::write_pool_rows`;
  `ProcessingState::persisted_pools`; the write sits right after the asset
  write, before the hold-back return and the cursor advance;
  `RunStats::{pools_persisted, unregistered_pool_events}` (the count follows
  the WRITTEN minutes, like `offer_lookups`); metric `UnregisteredPoolEvents`,
  emitted only when non-zero.
- `events-backfill`: `--discover-pools` (`src/discover.rs`), runs before the
  empty-registry guard.
- `infra`: alarm `prices-<env>-ledger-processor-unregistered-pool`
  (`Sum >= 1` / 5 min, NOT_BREACHING, description 645 chars).
- Docs: `seed-pool-registry.md` (new section), schema overview §3.6 + App. A.
- Review follow-up (`b2541ef`): the decoder's per-contract line is now
  `debug!` — live re-reads held-back ledgers, and `sdex-backfill` /
  `asset-discovery` share the decoder, so a WARN there floods. The contract IDs
  travel in `LedgerSoroban::unregistered_pool_contracts`; the reconcile WARN
  `dropped trades from pools missing from prices.pool_registry` lists them
  (written minutes only), and the alarm description names that WARN.
- `4c13123`: prettier on `close-usd-zero-guardrails.md` (0151's doc reached
  `develop` unformatted and failed CI's format check on every branch).

Verification: workspace **1,115 passed, 0 failed**; negative controls — all 3
persistence tests fail with the write removed, the counting test fails when the
tally counts every decoded ledger; local `cdk synth` of Observability shows the
alarm; local end-to-end on a scratch CH loaded with the prod registry (728
rows) and the prod factory events 63M→tip (185 candidates): dry run
`to_write=42 {aquarius 27, phoenix 1, soroswap 14}`, all `change="new"`; write
→ 515 / 20 / 235; re-run `to_write=0`; no new soroswap row without tokens.

**Modified tests:** `reconcile_e2e.rs` and `reconcile_drain.rs` test sinks gain
a no-op `write_pool_rows` — the trait grew a method. No assertion changed.

## Design Decisions

### From Plan

1. **Live writes only the delta, like 0132 does for assets.** Snapshot of
   durable rows in `ProcessingState`, advanced only after a successful write.
2. **The write precedes the cursor advance.** A held-back ledger is re-read and
   re-learned; a passed one never is.

### Emerged

3. **A new `--discover-pools` mode, not the Soroswap-API seeder.** The seeder
   rewrites all ~730 rows from a different source and holds back concentrated
   pools; the new mode uses live's own `learn_factory` and writes only missing
   rows, and its dry run doubles as the precondition check before a 0286
   `DROP PARTITION`.
4. **A separate `unregistered_pool_events` counter, not `unresolved`.**
   `unresolved` counts any pool-level `swap` (routers, the v3 venue) and is the
   backfills' fatal guard, so it can never read zero on live. The shape-based
   count can, which is what makes `>= 1` a usable alarm. `unresolved`
   semantics are untouched.
5. **Snapshot from the registry's own rows, not the table's.**
   `load_pool_rows` normalises some rows (bad Phoenix `wasm_hash` → none); a
   raw snapshot would rewrite them every run.
6. **Discover before deploy.** The tool does not need the new processor, and
   the deploy's cold start then loads the 42 rows.
7. **Synthetic ledgers from `Default` for the new tests** instead of the
   gitignored fixtures, so they run in CI.

## Issues Encountered

- **Known, not fixed:** re-learning a Phoenix `create` replaces the pool with
  `wasm_hash: None` (`PhoenixPoolRegistry::register`), and live would now
  persist that. No writer sets a Phoenix `wasm_hash` today (all 19 prod rows are
  empty), so nothing can be lost yet; the discover dry run flags any
  `change="changed"` row.
- The first discover query used `signature` only and would have missed every
  Phoenix pool (see Findings). Caught against prod before shipping.
- A whole-era read in one query exceeds `dev_read`'s 30 s limit; chunked reads
  do not.

# 📕 DEPLOY RUNBOOK

Order matters: the registry is seeded (steps 3–4) **before** the deploy
(step 6), so the deploy's cold start loads the new rows. Steps 3–4 run from the
local machine over `ssh`; the password travels on stdin, never in `argv`, and
is never typed.

0. **[GitHub]** PR #322 CI green (Rust + TypeScript), then merge it into
   `develop`.
1. **[local machine, repo root]** Sync and build the host binary. Run all
   together:
   ```bash
   git switch develop && git pull --ff-only
   cargo build --release -p events-backfill --target x86_64-unknown-linux-musl
   file target/x86_64-unknown-linux-musl/release/events-backfill
   ```
   ✅ Checkpoint: `file` says `static-pie linked` (a `gnu` build does not run
   on the host).
2. **[local machine, repo root]** Copy it up. Run this command:
   ```bash
   scp -i ~/.ssh/sorban-prod_ed25519 target/x86_64-unknown-linux-musl/release/events-backfill deploy@168.119.73.161:~/events-backfill
   ```
3. **[local machine → prod CH host]** Dry run. Run the first command on its
   own, check the length, then run the second:
   ```bash
   CH_PW=$(aws secretsmanager get-secret-value --profile soroban-explorer \
     --secret-id soroban/production/operator/env --query SecretString --output text \
     | sed -n 's/^CLICKHOUSE_PASSWORD=//p' | tr -d "\"'"); echo "${#CH_PW}"
   ```
   ✅ Checkpoint: prints `44`. Then:
   ```bash
   printf '%s\n' "$CH_PW" | ssh -T -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 '
     read -r CH_PW
     TIP=$(docker exec -i app-clickhouse-1 clickhouse-client -q "SELECT max(ledger_sequence) FROM default.soroban_events")
     echo "tip=$TIP"
     CLICKHOUSE_PASSWORD="$CH_PW" ~/events-backfill --discover-pools \
       --start 63000000 --end "$TIP" --clickhouse-url http://localhost:8123 --dry-run'
   ```
   ✅ Checkpoint: `to_write=42 per_venue={"aquarius": 27, "phoenix": 1,
   "soroswap": 14}` (more if pools were created since 2026-09-17), every pool
   line `change="new"`, then `DRY RUN — nothing written`. ⛔ Any
   `change="changed"` → stop.
4. **[local machine → prod CH host]** Write, then prove it. Same command as
   step 3 **without `--dry-run`** → `discover-pools: done written=42`. Then
   run step 3 again unchanged → ✅ `to_write=0`. Then run this command:
   ```bash
   ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
     "docker exec -i app-clickhouse-1 clickhouse-client -q 'SELECT venue, count() FROM prices.pool_registry FINAL GROUP BY venue ORDER BY venue'"
   ```
   ✅ Checkpoint: aquarius 515, phoenix 20, soroswap 235 (plus any pools
   created since). Then `unset CH_PW`.
5. **[local machine, repo root, on `develop` at the merge]** Build **every**
   Lambda asset — the Compute deploy ships whatever is in `target/lambda/`,
   not just the processor. Run all together:
   ```bash
   for c in $(tools/scripts/lambda-assets.sh); do
     cargo lambda build --release --arm64 --features lambda -p "$c" || break
   done
   strings target/lambda/prices-ledger-processor/bootstrap | grep -c 'dropped trades from pools missing'
   ```
   ✅ Checkpoint: the loop finishes without error and the count is **≥ 1**
   (0 = stale processor binary, stop).
6. **[local machine, repo root]** Deploy. Run separately, answer the CDK
   prompts, and read each diff: Compute should change only Lambda code,
   Observability should add only the `…-unregistered-pool` alarm.
   ```bash
   export AWS_PROFILE=soroban-admin AWS_REGION=eu-central-1
   make -C infra diff-production
   ```
   then
   ```bash
   make -C infra deploy-production-compute
   ```
   then
   ```bash
   make -C infra deploy-production-observability
   ```
7. **[local machine, ~15 min after step 6]** Check the processor is healthy and
   silent. Run all together:
   ```bash
   aws logs filter-log-events --log-group-name /aws/lambda/prices-production-ledger-processor \
     --start-time $(( ($(date +%s) - 900) * 1000 )) \
     --filter-pattern '"dropped trades from pools missing"' --query 'length(events)'
   aws cloudwatch describe-alarms --alarm-names prices-production-ledger-processor-unregistered-pool \
     --query 'MetricAlarms[0].StateValue'
   ```
   ✅ Checkpoint: `0` and `"OK"` (or `"INSUFFICIENT_DATA"` — the metric is
   only emitted when non-zero). If the WARN appears, its `contracts` field names
   the pools; re-run step 3 over the range holding their factory events.
8. **[whenever the next pool is created]** AC 2 prod check: the log shows
   `persisted AMM pools learned from factory events` and the pool's
   `contract_id` appears in `prices.pool_registry`.
9. **[2026-09-19, first full day after deploy]** AC 4: Aquarius raw vs stored,
   alongside [[0282]]'s measurement. Then close the task.

**Final test** (local machine → prod CH host): run step 3 unchanged. The
registry covers the live era when it prints `to_write=0`; together with step 7's
`0` / `"OK"`, the durable fix is live.
