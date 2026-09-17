---
id: "0256"
title: "asset-discovery's ledger scan has never run on production — the worker re-seeds hourly and scans nothing"
type: BUG
status: active
related_adr: []
related_tasks: ["0210", "0054", "0218", "0223", "0226", "0241", "0140"]
tags: [layer-backend, priority-high, effort-small, milestone-M2, ingest, defect]
milestone: 2
links:
  - "../../../packages/asset-discovery/src/main.rs"
history:
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Found while deploying [[0210]]'s symbol stage. Reading the worker's
      CloudWatch logs to confirm the symbol stage showed that every run since
      at least 07:17 UTC ends in the same WARN and `scanned: 0`.
  - date: 2026-09-15
    status: backlog
    who: stkrolikiewicz
    note: >
      ⚠️ A liveness decision is now parked HERE, explicitly. [[0223]] measured
      that asset-discovery has no `-no-invocations` alarm — one of only two
      scheduled workers without one — and that the code comment exempting it
      (`observability-stack.ts:1515`, "its -errors alarm is the coverage today")
      is circular, since a -errors alarm reads OK when nothing runs at all. 0223
      gave `supply` its health alarms and deliberately did NOT do the same here,
      because this task says the worker's scan is a no-op and the stage may be
      removed: alarming the liveness of dead code is not coverage. Whoever
      closes this task must settle it one way or the other — add the worker to
      `workerHealth` (two alarms, an `impact` sentence, and 0222-style
      induction) or record why a worker that survives this task still needs no
      liveness alarm. Closing 0256 without deciding leaves the hole 0223 found.
  - date: 2026-09-16
    status: backlog
    who: stkrolikiewicz
    note: >
      🔴 This task is the CAUSE of [[0226]] and [[0241]], measured today. The
      seeding-only branch does not merely skip the scan — it re-inserts the
      WHOLE 209,196-row registry into `prices.assets` every hour via
      `write_assets()` (`asset-discovery/src/lib.rs:104`). `prices.assets` is
      `ReplacingMergeTree(updated_at)`, so each hourly copy lives as its own part
      until a merge, and `prices-production-oracle` reads that table WITHOUT
      `FINAL` — so it loads every un-merged copy. Its logged row count walks
      1×→2×→3×→4× on the :17 boundary and resets when ClickHouse merges; both
      `Runtime.OutOfMemory` kills sampled on 2026-09-12 landed immediately after
      the 4× read. The cheapest fix for the oracle OOM and for 84% of the ops
      channel's traffic is therefore a config decision HERE, not work in the
      oracle. The priority was already high; this is the reason.
  - date: 2026-09-16
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Sequencing decided with the operator: this goes BEFORE [[0226]]
      (raised to priority-high the same day), because it removes the hourly
      amplification that actually reaches the oracle's 256 MB ceiling, it is a
      config decision rather than a change to shared code, and it produces the
      one measurement 0226 is missing — `Max Memory Used` on a cold container
      reading a 1× registry. [[0241]]'s alarm damping waits on the outcome and
      may prove unnecessary.
      ⚠️ Two decisions must BOTH be settled before this closes: what happens
      to the ledger scan, and the liveness-alarm question parked here from
      [[0223]]. Closing one without the other leaves the hole 0223 found.
  - date: 2026-09-16
    status: active
    who: stkrolikiewicz
    note: >
      Fix written and verified locally; NOT deployed. `ensure_seed` captures
      `AssetRegistry::watermark()` before interning the seed and writes through
      `write_new_assets`, whose `since >= watermark()` short-circuit makes a
      steady-state run write nothing at all. Branch
      `fix/0256_asset-discovery-ledger-scan-never-runs`, commit `111a4fb`,
      pushed to origin; no PR opened, deliberately. Evidence under "Fix shipped
      to the branch" below — the load-bearing item is the NEGATIVE control,
      because every assertion `seed_it.rs` already had uses `FINAL` and so
      passes with the defect present.
      ⚠️ Scope unchanged: this stops the amplification only. The ledger-scan
      decision and the [[0223]] liveness question are both still open.
---

# The ledger scan is dead code in production

## Summary

`prices.discovery_state` is **empty** on production, and every `asset-discovery`
run logs:

```
WARN  no discovery_state cursor and INITIAL_DISCOVERY_LEDGER unset —
      seeding only, skipping ledger scan
```

The worker's own stats confirm it: `scanned: 0`, `to_ledger: 0`,
`pools_total: 0`, on every run. The scan half of this Lambda has never executed.

## Context

The scan starts from `load_cursor()`, falling back to the operator-set
`INITIAL_DISCOVERY_LEDGER` when there is no cursor. Neither exists, so the
branch is skipped — by design, loudly, but nobody was reading the log.

The 52 soroban assets and the ~207k registry evidently arrive by another path
(most likely `prices-ledger-processor`), which is why the gap went unnoticed:
the registry looks healthy.

## Never switched on — since 2026-06-25, not since 2026-09-02

The first history entry bounds this at "every run since at least 07:17 UTC" on
2026-09-02. The true answer is the worker's own birthday. `asset-discovery`
shipped on **2026-06-25** in three commits (`feat(lore-0054)`: crate foundation,
then the increment that introduced `INITIAL_DISCOVERY_LEDGER`, then the CDK
wiring) — and the CDK wiring deliberately left the variable unset. The comment
is still there, `eventbridge-stack.ts:290`:

> NB: `INITIAL_DISCOVERY_LEDGER` is intentionally NOT set here — the binary
> seeds gracefully without it and only scans once a `prices.discovery_state`
> cursor exists. **Operator activates the ledger scan as a deploy-prep step**
> (seed the cursor or set the env), so synth is not gated on an operator value.

🔑 **The scan was never broken. It was designed off, behind a manual
activation step nobody performed.** That is a different defect from the one this
task's title implies, and it changes who has to act: there is no bug to find in
the scan path, only a decision deferred at deploy time and then forgotten.
**83 days** as of 2026-09-16.

`git log -S "INITIAL_DISCOVERY_LEDGER" -- infra/` returns exactly one commit —
the one that wrote that comment. The variable has never been set in any
environment.

⚠️ Production can only corroborate as far as retention allows: the earliest
`seeding only` WARN in `/aws/lambda/prices-production-asset-discovery` is
**2026-08-18 10:17 UTC**, which is the 30-day log boundary, not an onset. The
2026-06-25 date comes from the repository, not from CloudWatch.

### The symbol stage does NOT depend on the scan

Relevant because this task's first decision is whether to delete the stage.
`main.rs` runs the symbol stage **first and unconditionally** — before the seed
and before the scan branch — so it is not downstream of the dead code.

Measured over 7 days, all pages summed: **168 runs, 0 with
`symbols_considered > 0`, 0 `symbol stage failed`.** Zero failures matters,
because on `Err` the run-complete log reports all four `symbols_*` fields as `0`
via `unwrap_or(0)` — so without that check the zeros would be ambiguous.

`load_unresolved_contracts` (`symbols.rs:356`) selects contract rows not already
in `prices.asset_symbol`, so `considered: 0` means **nothing is left to
resolve** — [[0210]] finished its 52 rows. And new soroban contracts keep
arriving via `ledger-processor`, so deleting the scan would not starve the
symbol stage.

#### Settled: there are no give-ups, and no ClickHouse query is needed

That `NOT IN` excludes both "has a symbol" and "gave up at
`MAX_SYMBOL_ATTEMPTS`", so `considered: 0` looked ambiguous. The run logs
separate them. Across the **full 29-day retention** exactly five runs carried
symbol work:

| run (UTC) | considered | resolved | absent | skipped |
|---|---|---|---|---|
| 2026-09-02 09:54:40 | 25 | 25 | 0 | 0 |
| 2026-09-02 10:17:23 | 25 | 25 | 0 | 0 |
| 2026-09-02 10:19:14 | 2 | 2 | 0 | 0 |
| 2026-09-02 17:17:21 | 1 | 1 | 0 | 0 |
| 2026-09-07 16:17:20 | 1 | 1 | 0 | 0 |

🔑 **`absent` is 0 in every run.** `attempts` increments only on a negative
answer, so no contract has ever accumulated one, and none can be parked at
`MAX_SYMBOL_ATTEMPTS` (= 3). `considered: 0` therefore means, unambiguously,
**every contract has a symbol**. The `prices.asset_symbol` query this section
previously called for is not needed.

25 is `MAX_CONTRACTS_PER_RUN` (`symbols.rs:47`), so the first two runs are full
batches: 25 + 25 + 2 = **52**, exactly the population [[0210]] recorded. The
remaining two runs are **54** in total.

🔑 **Those last two are live evidence for this task's open decision.** Both
contracts arrived *after* the ledger scan was already dead — so they reached
`prices.assets` through `ledger-processor` — and the symbol stage resolved each
within the hour. **Deleting the scan would not starve the symbol stage**, and
that is now observed twice on production rather than inferred from the call
order in `main.rs`. Nothing new has arrived since 2026-09-07, which is why all
168 runs in the last 7 days report zero.

⚠️ Retention bounds this at 2026-08-18, but `prices.asset_symbol` was created
by [[0210]]'s deploy on 2026-09-01/02, so the window covers the table's entire
history.

## 🔴 Measured consequence — this is why the oracle OOMs (2026-09-16)

The seeding half is not harmlessly idle. It is the most expensive writer in the
system, and it breaks a different worker.

| link | evidence |
|---|---|
| asset-discovery re-inserts the **whole** registry hourly | 07:17 UTC run: `wrote asset rows` `assets=209196`, `seeded=209196`, `scanned=0` |
| it is the only such writer | over 24 h only asset-discovery and ledger-processor emit `wrote asset rows`; enrichment, supply and coarse-sweep emit none. ledger-processor writes **deltas** (1–21 rows/run) via `write_new_assets` |
| each re-seed is a real INSERT | `write_asset_rows` skips only when the iterator is empty; the `written` counter reaches 209,196 |
| the copies survive until merge | `prices.assets` is `ENGINE = ReplacingMergeTree(updated_at)` |
| the oracle loads all of them | `SELECT … FROM prices.assets` with no `FINAL`, `prices-ingest-core/src/writer.rs:77` |
| the arithmetic closes | 3 × 209,196 = **627,588** — exactly what the oracle logged in the same hour |

🔑 `write_assets()` is the very call `ledger-processor` **deliberately avoids**
because of Hetzner egress ([[0132]]) — see the doc comment on `write_assets` in
`writer.rs`. asset-discovery does hourly what that task taught the ledger
processor not to do, and it does it to re-write 209k rows of unchanged identity
data.

⚠️ **Not proven:** that fixing this alone ends the OOM. `Max Memory Used` is a
per-container high-water mark that does not reset between invocations, so the
249 MB seen on a 1× read is probably left over from an earlier 4× read in the
same container. A clean measurement needs a cold container reading 1×. What IS
established is that both sampled OOMs fired immediately after a 4× read, two for
two.

🔑 **This reframes the task.** It is written as a missing feature ("the scan
never runs"). The measured defect points the other way: the *seeding* half runs
too much.

### The registry is written by `ledger-processor` — Context's guess, now confirmed

Context calls it "most likely `prices-ledger-processor`". It is, on two
independent lines of evidence.

**Measured.** Over 24 h, `wrote asset rows` appears in exactly two log groups:

| worker | rows per run | call |
|---|---|---|
| `ledger-processor` | **1–21** (newly-interned assets only) | `write_new_assets` |
| `asset-discovery` | **209,196** (the entire registry) | `write_assets` |
| `enrichment`, `supply`, `coarse-sweep` | none | — |

**Documented.** `prices-ingest-core/src/writer.rs` already says so on
`write_asset_metadata`: identity columns in `prices.assets` are *"written by the
ledger processor — a delta of newly-interned assets per run via
`write_new_assets`"*. The delta path is the designed one; `write_assets` is the
exception, and `write_new_assets`' own doc note records that `ledger-processor`
avoids the full form deliberately because of Hetzner egress ([[0132]]).

🔑 **So asset-discovery contributes no new assets at all.** It reads the
registry, de-duplicates it in memory (which is why the number it *writes* stays
209,196 even when the read returned 627,588 — three un-merged copies), and
writes that clean copy back as a **new part**. It does not replace the
duplicates it just read; it adds to them. Pure churn on a table another worker
already maintains correctly.

⚠️ This settles the *seeding* half of the first Implementation bullet
independently of the scan decision: the seed is redundant whether or not the
ledger scan is ever switched on. Deleting it costs nothing that
`ledger-processor` is not already doing.

## Implementation

- ✅ The hourly full re-seed is **fixed** as of 2026-09-16 (PR #319): `ensure_seed`
  now writes only newly interned assets.
- ⛔ But [[0140]] documented this defect on 2026-08-03 and located it at
  `discover_window`'s `write_assets`, which is **still unguarded**
  (`lib.rs:255`). It is dormant only because the scan never runs. **Enabling the
  scan would reintroduce the hourly full re-emit**, so 0140's guard is a
  precondition for that option, not a follow-up. 0140 also found a second live
  instance in `oracle-worker`.
- Decide whether the scan is still wanted at all. If `ledger-processor` already
  covers asset discovery, this stage may be redundant and the honest fix is to
  delete it rather than start it.
- If wanted: set `INITIAL_DISCOVERY_LEDGER` in `infra/envs/production.json`,
  pick a sensible starting ledger, and confirm `discovery_state` advances.
- Either way the WARN must stop being a permanent steady state — a worker whose
  main stage is skipped every hour should alarm, not log.

## Fix shipped to the branch — verification (2026-09-16)

Branch `fix/0256_asset-discovery-ledger-scan-never-runs`, two commits: `111a4fb`
(the fix) and `0893445` (test determinism, from the review below). Opened as
PR #319 against `develop`.

### The change

`asset-discovery/src/lib.rs`, `ensure_seed` — three lines:

```rust
 let mut registry = AssetRegistry::from_existing(existing);
+let durable = registry.watermark();
 for identity in identities {
     registry.get_or_assign(identity);
 }
-writer.write_assets(&registry).await?;
+writer.write_new_assets(&registry, durable).await?;
```

Capturing the watermark BEFORE interning the seed is the whole trick: in steady
state every seed identity already exists, `get_or_assign` returns its existing
id, nothing lands at or above the watermark, and `write_new_assets` takes its
`since >= registry.watermark()` fast path and issues no INSERT at all. An empty
table still seeds correctly, because the watermark is then 1.

🔑 The guard already existed in this file — in the other half.
`discover_window` writes only when the scan changed the registry, with a comment
naming this exact failure: "a full RMT re-INSERT each hour would pile up parts
and inflate the next FINAL load". It sits in the scan path, which has never run
on production. [[0132]] removed the same amplification from `ledger-processor`
and left this caller behind; `write_assets`' own doc blesses "the
discovery/oracle workers" on the rationale that "a single full write per run is
cheap" — false for a worker scheduled hourly rather than one-shot.

### Evidence

| check | result |
|---|---|
| `cargo test --workspace` (what CI runs) | rc=0, 96 passed, 0 failed |
| `cargo clippy -p asset-discovery --all-targets` | rc=0, zero warnings from this crate |
| IT `seed_it` vs local ClickHouse | passed |
| **IT negative control** | fails on the new assertion, `left: 40, right: 20`, repeatably over two consecutive runs |
| restore after the negative control | byte-identical to backup, IT green again |
| formatting | `cargo fmt --check` rc=0; committed bytes identical to the bytes tested |

⚠️ **The negative control is the load-bearing evidence, not the green run.**
Every assertion `seed_it.rs` already had uses `FINAL`, which collapses a full
re-emit either way — so the existing test passes with the defect present and
could never have caught it. The new assertion reads the raw, un-merged
`count()`. With `write_assets` restored it fails at **left: 40, right: 20**:
two copies of the 20-asset seed, the production 1×–4× shape at test scale.

Per [[0275]] CI does not run these, which is why the results are recorded here.

⚠️ The repo's pre-push hook runs `nx affected` and covers only the TypeScript
projects — it did NOT exercise this crate. CI also lints an explicit crate
allow-list that does not include `asset-discovery` (`ci.yml:170-182`), so this
crate has no clippy coverage in CI at all.

### Accepted cost — existing rows lose their hourly `sac_address` refresh

Found while reviewing the branch, and taken deliberately rather than missed.

`write_asset_rows` recomputes `sac_address = registry.sac_address_of(identity)`
for every row it writes. The old full re-emit therefore rewrote all ~209k rows
every hour with a freshly derived SAC address and a new `updated_at`, so any
correction to that derivation propagated across the whole table within the hour.
With `write_new_assets` only newly interned assets are written, so a later fix to
SAC derivation — or to the mainnet-only network scoping flagged on
`AssetRegistry::from_existing` — will no longer self-heal through this worker.
Pre-existing rows would keep a stale `sac_address` indefinitely and nothing else
backfills them.

Why it is accepted: `AssetRegistry::assets_since` already states the assumption
this rests on — an asset's `sac_address` is a deterministic function of its
identity, so a newly-written row needs no later correction — and
`ledger-processor` has behaved exactly this way since [[0132]]. Keeping an hourly
209k-row re-emit purely as an accidental repair mechanism is the defect this task
exists to remove.

⚠️ The consequence to remember: if SAC derivation ever changes, it now needs a
deliberate one-shot backfill. No hourly process is quietly fixing it any more.

### Still unproven → proven 2026-09-17

~~That this alone ends the oracle's OOMs.~~ Read on cold containers over the
full day after the deploy, registry at 1× throughout:

| | before the fix (2026-09-15 00:00 → 09-16 12:42 UTC) | after (24 h, → 2026-09-17 12:47 UTC) |
|---|---|---|
| `wrote asset rows` by asset-discovery | 37 of 37 runs, ~209 k rows each | **0 of 24 runs** |
| `existing_assets` seen by the oracle | 209 k → 418 k → 627 k → **836 k**, merging back every ~4 h | **209,208 → 209,292**, one copy |
| oracle `Max Memory Used`, hourly peak | 210–**256 MB** (the limit) | **148–191 MB** on every new container |
| OOM / `signal: killed` lines | 5 in ~37 h | **0** |
| `AWS/Lambda Errors` | 2–5 a day, every day since at least 09-10 (1.0–1.7 %) | **0 on 289 invocations** |
| `prices-production-oracle-errors` | 18 firings 09-12 → 09-16, each 3–5 min | none since 09-16 07:23 UTC (29 h) |

Every one of the five OOMs fell in an hour where the oracle read ~836 k rows —
four un-merged copies. Since the fix it has never read more than one. The one
post-deploy 250 MB reading is a container born before the deploy carrying its
old high-water mark; it was gone by 12:47 UTC. Ten cold containers since all
start at **148 MB** and peak at 191 MB — 58–75 % of the 256 MB limit.

[[0241]] is closed on this. [[0226]] is an efficiency task, not an outage fix.

## Acceptance Criteria

- [ ] A recorded decision on whether the ledger scan is still needed
- [ ] If kept: `discovery_state` has a cursor and it advances between runs
- [ ] If dropped: the scan path and its config are removed, not left dormant
- [ ] The permanent WARN is gone — either the scan runs, or the code does not
      pretend it might
