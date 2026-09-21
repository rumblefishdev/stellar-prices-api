---
id: "0291"
title: "pool_registry has not learned a pool since 2026-07-06 — live forgets newer pools on every cold start and drops their trades"
type: BUG
status: blocked
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
  - date: 2026-09-18
    status: blocked
    who: okarcz
    by: ["0286"]
    note: >
      AC 1 DONE on production: #322 merged (af1f910), --discover-pools wrote the
      42 missing pools at 08:00 UTC — prices.pool_registry 728 → 770 (aquarius
      515, phoenix 20, soroswap 235), dry run to_write=42 all change="new",
      re-run to_write=0, no duplicate contract_id. The alarm
      prices-production-ledger-processor-unregistered-pool is deployed.
      BLOCKED on 0286: the Compute deploy at 08:11:33 UTC stalled live
      ingestion for ~6 min (every reconcile failed with ClickHouse Code 16
      NO_SUCH_COLUMN_IN_TABLE) because a build from develop carries 0286's
      pf_trade_count / pf_volume / pf_price_volume candle columns, which prod's
      price_ohlcv_1m does not have — 0286 (#320) merged 2026-09-17 12:59 and was
      never deployed, and its runbook orders schema BEFORE ingest. Rolled back
      08:18:04, recovered by 08:22, DLQ 0, nothing lost, no alarm fired.
      Cannot be worked around: this task's branch also contains 0286 and its
      reconcile.rs calls 0286's API (cherry-pick onto 2cb5b2b conflicts in 5
      hunks). AC 2 + AC 3 close when 0286 phase 1 runs its ingest step; its own
      precondition 1 is 0282's full-day measurement, owed 2026-09-19 — the same
      measurement this task needs for AC 4. Six runbook defects recorded.
  - date: 2026-09-21
    status: blocked
    who: okarcz
    note: >
      AC 4 met. Measured 2026-09-18 → 09-20 as dev_read: 09-19 and 09-20 are
      raw = stored exactly (26,422 and 20,497), so no residue in either
      direction and the old stored > raw impossibility is gone. 09-18's 6.6
      percent is this task's own seed seen from the other side — hour by hour,
      lost equals the newly-seeded pools' raw count EXACTLY in all eight hours
      before the 08:00 UTC write, then zero for fifteen hours after. Also
      measured: the 08:11:33 deploy stall that blocked this task cost ZERO
      trades, because 0064's durable cursor caught it up. Still blocked on
      0286 for AC 2 and AC 3 — the deploy, not the data.
  - date: 2026-09-21
    status: blocked
    who: okarcz
    note: >
      Two additions from a teammate's review, both accepted. (1) 0256 recorded
      the drop-the-scan decision on 2026-09-21; its removal PR is gated on this
      task's AC 2 + AC 3 on production, and saying so in 0256 is the trigger.
      (2) The unattended window: since the 09-18 08:00 UTC seed the registry has
      neither a writer nor a working sensor — the alarm cannot fire because its
      counter is in the rolled-back processor — so any pool created since then
      is exactly where the 42 were, silently. 0282's same-day measurement makes
      the cost precise: such a pool is INVISIBLE, not under-counted. Noted that
      0290's 09-21 dry run used Friday's tip as --end, so its to_write=133 says
      nothing about the window; a dry run to the current tip is owed before
      0286 phase 3.
  - date: 2026-09-21
    status: blocked
    who: okarcz
    note: >
      The unattended window is MEASURED and currently EMPTY. Re-running 0290's
      --discover-pools dry run to the true tip 64,539,364 returns identically
      candidates=650, to_write=133, per_venue={"sushiswap": 133} — not one new
      pool of any venue in the 47,418 ledgers since Friday. to_write=133 is
      therefore a current number and 0286 phase 3 can be planned against it.
      One sample only, and it expires: the window grows until the deploy and
      the alarm is still blind, so the pre-write re-run stays mandatory. The
      fill rate implied (~0.7 pools/day, from the original 42 over ~2 months)
      argues for waiting on 0286 phase 1 rather than deploying Compute from a
      divergent branch.
---

## 📊 STATUS — 2026-09-18 · ⛔ BLOCKED on [[0286]] · AC 1 DONE on prod

- ✅ **AC 1 is met and permanent.** The 42 missing pools were seeded on
  production 2026-09-18 08:00 UTC — `prices.pool_registry` went **728 → 770**
  rows (aquarius 515, phoenix 20, soroswap 235), no duplicate `contract_id`.
  This was written by the one-off tool straight to ClickHouse, so it does not
  depend on any Lambda and cannot regress.
- ✅ PR #322 merged into `develop` (`af1f910`). Alarm
  `prices-production-ledger-processor-unregistered-pool` is deployed and live.
- ⛔ **The live-persistence half is NOT deployed, and cannot be** until
  [[0286]]'s schema lands. Deploying it on 2026-09-18 08:11 UTC **stalled
  production ingestion for ~6 minutes**; rolled back at 08:18. See
  *Issues Encountered* → "The 2026-09-18 deploy incident".
- ⏳ AC 4 (2026-09-19) is unaffected — it measures [[0282]]'s fix, not this one.
- 🔑 Root cause is [[0256]]: the registry's designed maintainer (the
  asset-discovery scan, task 0069) has never run on production. 0291 makes the
  live processor the maintainer, so 0256 can drop the scan without losing it.
  **[[0256]] recorded that decision on 2026-09-21.** Its removal PR is gated on
  **this task's AC 2 + AC 3 being met on production** — when they are, say so in
  0256; that is the trigger.

### 🔴 The unattended window — open since 2026-09-18 08:00 UTC, and growing

Raised by a teammate 2026-09-21. Between the seed and the live-persistence
deploy the registry has **neither a writer nor a working sensor**:

- the seed was a one-off; nothing has written `pool_registry` since 08:00 UTC
  on 2026-09-18;
- the live processor still does not persist what it learns (the code is
  merged, not deployed);
- `UnregisteredPoolEvents` **cannot fire** — its counter is in the rolled-back
  processor, so the alarm sits `OK` on no data (`notBreaching`).

**Any pool created in this window is exactly where the 42 were**, and nothing
will say so. ⚠️ [[0282]]'s 2026-09-21 measurement makes the cost precise: an
unregistered pool is **invisible, not under-counted** — live stores *none* of
its trades. So the window's damage is the full raw trade count of every pool
created in it, not a fraction.

The runbook only catches this at its Final test (`to_write=0`) — i.e. at deploy
time, as a surprise. Two changes follow:

1. **Step 3 (`--discover-pools`) must be re-run at deploy time** over the range
   from 2026-09-18 to the then-current tip. A dry run from before the deploy
   does not license the write.
2. **Size the hole now, not at the deploy.** ✅ **MEASURED 2026-09-21 09:24
   UTC — the window is EMPTY.** [[0290]]'s first dry run that day used
   `--end 64491946` (Friday's tip) and so said nothing about the window; re-run
   to the current tip **64,539,364** it returns *identically*
   `candidates=650 to_write=133 per_venue={"sushiswap": 133}`. Not one new
   pool of any venue was created in the 47,418 ledgers (~2.7 days) since. So
   `to_write=133` is now a **current** number, and [[0286]] phase 3 can be
   planned against it.

   ⚠️ **This is one sample, and it expires.** The window keeps growing until
   the deploy, and nothing will announce the pool that fills it — the alarm is
   still blind. Re-run before the write regardless; an empty window today is
   not a licence to skip the check later.

   📈 It does say the window fills **slowly**: the original gap was 42 pools
   accumulated over roughly two months, so ~0.7/day, and zero in 2.7 days is
   consistent with that. That is an argument for **waiting** for 0286 phase 1
   rather than deploying Compute from a divergent branch to close it sooner.

### What unblocks this

[[0286]] phase 1 must roll out in its documented order
(`docs/runbooks/0286-candle-definitions-rollout.md` §2:
`schema → enrichment + coarse sweep + prices-api → MV re-CREATE → ingest LAST`).
Its **precondition 1 is [[0282]]'s full-day measurement**, which does not exist
until **2026-09-19** — the same measurement this task owes for AC 4. Once the
ingest step of that rollout runs, it carries 0291's code with it and AC 2 + AC 3
close with no further work here.

⚠️ **Do not deploy `Prices-production-Compute` from `develop` before then** —
any build from `develop` (or from this task's branch) breaks live ingestion.

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

- [x] The 22 Aquarius and 10 Soroswap pools are in `pool_registry`.
      → the full gap was **42** (27 aquarius, 14 soroswap, 1 phoenix; 0285 only
      counted pools that had traded). **Written on prod 2026-09-18 08:00 UTC**:
      dry run `to_write=42`, all `change="new"`, zero `changed`; write
      `written=42`; re-run `to_write=0`. Verified as `dev_read`: **728 → 770**
      rows (aquarius 515, phoenix 20, soroswap 235), 0 duplicate `contract_id`,
      token convention preserved (soroswap populated, aquarius/phoenix empty —
      matching all 728 pre-existing rows). Independent of any Lambda.
- [ ] A pool created after deploy is in the table within one reconcile run, and
      survives a forced cold start (test + prod check).
      → ✅ test (`tests/pool_registry_persist.rs`); ⛔ **blocked** — the code is
      merged but cannot be deployed until [[0286]]'s schema lands (see the
      incident below). Not deferred, not abandoned: it ships with 0286's ingest
      step.
- [ ] Unclassifiable pool events are counted and alarmed on the live path.
      → ✅ alarm `prices-production-ledger-processor-unregistered-pool` is
      **deployed and live** (Observability was not rolled back). ⛔ the counter
      that feeds it is in the rolled-back processor, so the alarm currently
      reads `OK` on *no data* (`notBreaching`) and cannot fire. Same blocker.
- [x] Aquarius raw (registry-joined, all pool wasms) vs stored for a full day
      after the fix shows no stored > raw residue. → **MET, measured
      2026-09-21** over 2026-09-18 → 09-20. No residue in either direction:
      09-19 and 09-20 are `raw = stored` on the integer (26,422 and 20,497).
      The prediction held — raw no longer omits the 42 pools, so the old
      *stored > raw* impossibility is gone.
      🔑 **The seed's effect is visible as a clean boundary.** On 09-18, split
      by hour and by `pool_registry.updated_at`, `lost` equals the newly-seeded
      pools' raw count **exactly** in all eight hours before the 08:00 UTC
      write, then is zero for the fifteen hours after it. That is this task's
      fix being observed directly, not inferred: 1,563 + 12 = the day's entire
      1,575 gap. Full table in [[0282]]'s runbook step 8.
      ⚠️ **Generalises to every future measurement:** `raw` resolves
      `pool_registry` as of *now*, so any raw-vs-stored window spanning a seed
      shows a phantom loss for the pre-seed hours. Measure after the seed, or
      pin the registry to the measured day.

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
8. **Roll back, do not roll forward, on 2026-09-18.** With ingestion stalled,
   the alternative was applying [[0286]]'s schema ad hoc to make the running
   code valid. Rejected: the schema is step 1 of a four-step order, and
   schema + ingest without the middle steps is *worse* than the stall — a
   pre-0286 MV reads a dust-only minute's `low = 0` and writes a zero `low`
   across every coarse tier. A 6-minute stall is recoverable; corrupted
   published prices are not. Rollback restored service in 31 seconds.
9. **Only the processor was rolled back; Observability was left deployed.** The
   api-handler showed 0 errors and no 5xx (the read path does not touch `pf_*`),
   and the new alarm is inert rather than harmful. Reverting either would have
   added change during an incident for no benefit.
10. **Do not hand-merge 0291 without 0286.** See the incident notes for the
    evidence (branch contains 0286; cherry-pick conflicts in 5 hunks; our code
    calls his API). Waiting costs only the exposure of a *new* pool created
    before 0286 deploys; AC 1 is already permanent. Reviewed and rejected twice
    on 2026-09-18, including the "deploy from the feature branch" variant.
11. **The seeded rows are kept despite the rollback.** They are valid under
    both code versions — nothing in the 42 rows depends on the new processor —
    and they are what makes [[0286]] phase 3 safe for live-era AMM months.

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

### 🔴 The 2026-09-18 deploy incident — ~6 min of stalled ingestion

**What happened.** `deploy-production-compute` at **08:11:33 UTC** shipped a
ledger-processor built from `develop`. From **08:12:09** every reconcile failed:

```
ERROR reconcile failed — will redeliver doorbell
      error="sink error: sink write failed: clickhouse: Code: 16"
```

Code 16 is `NO_SUCH_COLUMN_IN_TABLE`. Rolled back with
`lambda update-function-code` to the previous asset at **08:18:04**; the last
failure is **08:18:11** (one in-flight invocation). Recovery was complete by
08:22: 0 failures, queue drained to 0, **DLQ 0 — nothing was lost**, candle lag
back to ~90 s from a peak of 429 s. No CloudWatch alarm fired at any point
(the doorbell-age threshold is 120 s and the backlog drained first).

**Root cause — not this task's code.** [[0286]] (PR #320) merged into `develop`
on 2026-09-17 12:59, six hours before #322, and was never deployed. Its
`OhlcvCandle` row carries three new columns — `pf_trade_count`, `pf_volume`,
`pf_price_volume` — which **production's `price_ohlcv_1m` does not have**
(verified: 15 columns, zero `pf_*`). Building the processor from `develop`
swept his ingest code in. `write_candles`'s own doc comment states the rule we
broke: *"DEPLOY ORDER (task 0286 §4.9): schema first … and this — the ingest —
LAST."*

**0291's own code was never implicated.** The pool write uses the same
`write_pool_rows` path the discover tool had just run 42 times successfully
against the same table.

**Why no guard caught it.** The runbook's step-5 check
(`strings … | grep -c 'dropped trades from pools missing'`) proves the WANTED
code is in the binary. It cannot prove nothing ELSE came with it. The `cdk diff`
showed nine unrelated Lambdas with changed code — the same signal in another
form — and was read as routine rebuild noise.

**Cannot be worked around by building from the branch.** Verified:
`git merge-base --is-ancestor d11d20a b2541ef` → true; the branch tip's
`bucket.rs` has 17 `pf_trade_count` occurrences; 11 of 0286's commits are
reachable from it. A cherry-pick of `ebe1faf` onto the deployed commit
(`2cb5b2b`) conflicts in 5 hunks because **0291's `reconcile.rs` calls 0286's
API** (`OfferLookupCounts`, `extract_trades_with_counts`). No commit exists with
0291 and without 0286; creating one means ~60 lines of unreviewed rewrite.

### Runbook defects found by executing it (all fixed in the runbook below)

1. **Step 1 has no anti-stale guard.** The first build failed, yet `file` still
   reported `static-pie linked` — on a **17 July** binary left in `target/`,
   with no `--discover-pools` at all. The checkpoint passed on a stale artifact.
   Step 5 already greps the binary; step 1 now does too.
2. **Step 1 needs a musl C toolchain.** `zstd-sys` (via `xdr-parser`) is C, so
   the musl target needs `x86_64-linux-musl-gcc` — `sudo apt install musl-tools`
   (which pulls `musl-dev`). 0142's musl binary had no C dependency, which is
   why this never came up before.
3. **Step 3's AWS profile was dead.** `--profile soroban-explorer` returns
   `UnrecognizedClientException`; use the SSO profiles (`soroban-admin`).
4. **Step 3 needs an explicit `--region eu-central-1`** — that profile has no
   `[profile]` block in `~/.aws/config`.
5. **The secret is shell-`export` format**, not dotenv and not JSON (39 lines,
   2333 bytes), so the documented `sed 's/^CLICKHOUSE_PASSWORD=//p'` matched
   nothing and returned an empty password; `jq` fails at `column 7` (the space
   after `export`). Working extraction is in the runbook.
6. **🔑 The missing check — step 5 must verify nothing ELSE is in the build.**
   Before building, diff the deployed commit against `develop` over the ingest
   path and stop if any commit belongs to another task:
   `git log --oneline <deployed-sha>..develop -- packages/prices-ingest-core/src packages/prices-ledger-processor/src`.
   This is the check that would have prevented the incident, and it applies to
   every Compute deploy, not just this task's.

# 📕 DEPLOY RUNBOOK

> ## ▶️ Runbook state as of 2026-09-18
>
> **Steps 0–4 are DONE on production.** Step 5 was executed, step 6 was
> executed **and rolled back**. Steps 5–9 must NOT be re-run until [[0286]]
> phase 1 has rolled out — see the incident in *Issues Encountered*. When it
> has, 0286's own ingest step deploys this code; nothing here needs repeating.
>
> | step | state |
> | --- | --- |
> | 0 merge #322 | ✅ `af1f910`, 2026-09-18 07:11 UTC |
> | 1 musl build | ✅ after fixing two defects (below) |
> | 2 scp to host | ✅ byte-identical, runs on the host |
> | 3 dry run | ✅ `to_write=42`, all `change="new"` |
> | 4 write | ✅ `written=42`, re-run `to_write=0`, **728 → 770** |
> | 5 build Lambda assets | ⚠️ ran clean, but its guard was insufficient |
> | 6 deploy | ⛔ **rolled back** — broke ingestion, see incident |
> | 7 health check | ✅ ran; caught the failure |
> | 8 AC 2 prod check | ⛔ blocked |
> | 9 AC 4 | ⏳ 2026-09-19, unaffected |

Order matters: the registry is seeded (steps 3–4) **before** the deploy
(step 6), so the deploy's cold start loads the new rows. Steps 3–4 run from the
local machine over `ssh`; the password travels on stdin, never in `argv`, and
is never typed.

0. **[GitHub]** PR #322 CI green (Rust + TypeScript), then merge it into
   `develop`. ✅ done — `af1f910`.
1. **[local machine, repo root]** Sync and build the host binary. Run all
   together:
   ⚠️ **Prerequisite:** `sudo apt install musl-tools` (pulls `musl-dev`, which
   provides `x86_64-linux-musl-gcc`). `zstd-sys` — reached via `xdr-parser` —
   is C, and the musl target cannot build it without a musl C compiler. Without
   it the build dies on `failed to find tool "x86_64-linux-musl-gcc"`.
   ```bash
   git switch develop && git pull --ff-only
   mv target/x86_64-unknown-linux-musl/release/events-backfill .trash/ 2>/dev/null
   cargo build --release -p events-backfill --target x86_64-unknown-linux-musl
   file target/x86_64-unknown-linux-musl/release/events-backfill
   strings target/x86_64-unknown-linux-musl/release/events-backfill | grep -c 'discover-pools'
   ```
   ✅ Checkpoint: `file` says `static-pie linked` (a `gnu` build does not run
   on the host) **and the `strings` count is ≥ 1**.
   ⛔ `file` ALONE IS NOT ENOUGH. On 2026-09-18 the build failed and `file`
   still passed — on a 17 July binary left in `target/`, which had no
   `--discover-pools` at all. Moving it aside first makes a failed build
   impossible to mistake for a good one.
2. **[local machine, repo root]** Copy it up. Run this command:
   ```bash
   scp -i ~/.ssh/sorban-prod_ed25519 target/x86_64-unknown-linux-musl/release/events-backfill deploy@168.119.73.161:~/events-backfill
   ```
3. **[local machine → prod CH host]** Dry run. Run the first command on its
   own, check the length, then run the second:
   ⛔ Corrected 2026-09-18 — the original command failed three ways:
   `soroban-explorer`'s static keys are DEAD (`UnrecognizedClientException`),
   that profile has no region, and the secret is **shell-`export` format**
   (39 lines), not the dotenv the `…/env` name suggests — so the old `sed`
   matched nothing and returned an empty password (and `jq` dies at
   `column 7`, the space after `export`). Working version:
   ```bash
   CH_PW=$(aws --profile soroban-admin --region eu-central-1 \
     secretsmanager get-secret-value \
     --secret-id soroban/production/operator/env --query SecretString --output text \
     | sed -n 's/^[[:space:]]*\(export[[:space:]]\+\)\?CLICKHOUSE_PASSWORD=[[:space:]]*//p' \
     | tr -d "\"'" | tr -d '\r' | head -1); echo "${#CH_PW}"
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
5. **[local machine, repo root, on `develop` at the merge]**
   🔑 **FIRST — prove the build carries ONLY this task's changes.** Compare the
   DEPLOYED commit against `develop` over the ingest path; every commit listed
   must belong to this task:
   ```bash
   DEPLOYED=2cb5b2b   # the sha the running processor was built from
   git log --oneline $DEPLOYED..develop -- \
     packages/prices-ingest-core/src packages/prices-ledger-processor/src
   ```
   ⛔ **Any commit from another task → STOP.** On 2026-09-18 this listed 11
   [[0286]] commits alongside 0291's two; deploying anyway stalled production
   ingestion (see the incident). `strings` on the bootstrap proves the wanted
   code is present — it can never prove nothing else came with it, and a
   `cdk diff` full of unrelated Lambda code changes reads as routine rebuild
   noise. This git check is the only guard that catches it.

   Then build **every** Lambda asset — the Compute deploy ships whatever is in
   `target/lambda/`, not just the processor. Run all together:
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
