---
id: "0140"
title: "asset-discovery re-emits the whole asset registry every hour — 0132's defect in a second component"
type: PERF
status: backlog
related_adr: []
related_tasks: ["0210", "0132", "0133", "0067", "0256", "0226", "0241"]
tags: ["priority-medium", "effort-small", "cost", "write-amplification", "clickhouse"]
links: []
history:
  - date: 2026-08-03
    status: backlog
    who: okarcz
    note: >
      Found while verifying that the [[0072]] step-6 deploy had not reverted
      [[0132]]. `system.part_log` shows **~203,955 rows written to
      `prices.assets` every hour**, flat across the whole window and unchanged
      by the deploy — so not a regression, but the same full-registry re-emit
      0132 fixed in the ledger processor, present in `asset-discovery` and
      never addressed.
  - date: 2026-09-16
    status: backlog
    who: stkrolikiewicz
    note: >
      🔴 SOURCE CORRECTED — and this task survives [[0256]]'s fix rather than
      being closed by it. It locates the hourly re-emit at `discover_window`'s
      `write_assets`, but that branch has NEVER executed on production: the
      ledger scan has not run since the worker shipped on 2026-06-25. The hourly
      ~204k rows came from `ensure_seed` — the call site this task explicitly
      exempts as "a full write is correct there".
      ⚠️ Implemented as written, this task would NOT have fixed production: the
      guard would have landed in dead code while the re-emit continued. Fixed in
      [[0256]] instead (PR #319, deployed 2026-09-16 12:44 UTC). What remains
      here is recorded below — AC 3 and AC 5 untouched, `discover_window:255`
      still unguarded, and the AC 4 audit found a second live instance in
      `oracle-worker`.
  - date: 2026-09-21
    status: backlog
    who: stkrolikiewicz
    note: >
      Lands with [[0256]]'s removal PR #331. The scan decision is made —
      deleted, not enabled — so `discover_window:255` is gone, the
      "precondition" section is settled and AC 3 is moot. One item added from
      that PR's review: `ensure_seed` still READS the whole registry hourly to
      check ~20 identities. What remains here is that read and the
      `oracle-worker` instance.
---

# `asset-discovery` re-emits the full asset registry every hour

## Summary

`packages/asset-discovery/src/lib.rs:237-249`:

```rust
if scanned > 0 {
    writer.write_assets(&registry).await?;          // ← UNCONDITIONAL, full registry
    // Only re-write the registry when the scan changed it. The pre-seeded
    // registry is re-emitted verbatim on every zero-discovery run, and a full
    // RMT re-INSERT each hour would pile up parts and inflate the next FINAL
    // load. ...
    if final_rows != loaded_rows {
        writer.write_pool_registry(&pools).await?;  // ← what the guard protects
    }
    save_cursor(writer, last).await?;
}
```

`write_assets` writes **every row in the registry** on every run where
`scanned > 0`. The rule is `rate(1 hour)` (`infra/envs/production.json:17`).

## ⚠️ The comment is the trap

The comment describing the "only re-write when the scan changed it" guard sits
**directly above the unguarded `write_assets` call**, but the guard it describes
protects `write_pool_registry` below it. Read top-down it looks like
`write_assets` is the thing being guarded. It is not.

That misplacement is very likely why this survived 0132: the fix went into the
ledger processor, and anyone auditing `asset-discovery` afterwards would read
this comment and conclude it was already handled.

## Measured on ch-prod-01, 2026-08-03

```
hr                    parts   rows_written
2026-08-03 08:00:00      19        203953
2026-08-03 09:00:00       2        203954
2026-08-03 10:00:00       1        203953
2026-08-03 11:00:00       2        203955
2026-08-03 12:00:00       1        203954
2026-08-03 13:00:00       6        203966
```

One full-registry re-emit per hour: **~4.9M rows/day** of pure write
amplification into a `ReplacingMergeTree` that collapses essentially all of it on
the next merge. The row count tracks the registry size (~204k), not any real rate
of asset discovery — genuine new assets are a handful per hour at most.

**Not a correctness problem** — RMT dedup means no consumer sees a wrong value,
exactly as in 0132. The cost is egress to Hetzner, merge pressure on a cluster
shared with BE, and `FINAL` read cost against the accumulated parts.

## Still live on 2026-09-02, and it cost a deploy an hour

Confirmed unchanged a month later, from the worker's own logs while deploying
[[0210]]: every run still logs `wrote asset rows: 207754` — the whole registry.

The consequence this task predicts ("pile up parts and inflate the next FINAL
load") is now observable as a **reading hazard**, not just a cost. `existing_assets`
read at the start of three consecutive runs:

| run | rows read | ratio |
|---|---|---|
| 07:17 | 623,154 | 3× |
| 08:17 | 207,741 | 1× |
| 09:17 | 415,495 | 2× |

Distinct identities held at 207,754 throughout. A `count()` therefore lands
wherever the merge cycle happens to be, and this is not theoretical: during
0210's prod deploy a count taken mid-cycle read as *"the registry doubled in
five days, and the Soroban subset with it — 104 against 52"*. Both were exactly
2×, which reads as duplication rather than growth, and the deploy stopped until
`uniqExact` disproved it.

So the operational cost is larger than the write volume alone: **any count on
`prices.assets` is unreliable unless it is `uniqExact` or `FINAL`**, and nothing
says so at the point where someone would read one. Worth stating in whatever
fixes this, or in the schema comment if the fix is deferred again.

## Implementation

Apply the same shape the pool-registry write already uses: compare the row set
and skip the write when nothing changed, or write only newly-assigned assets as
[[0132]] did for the ledger processor.

- **Move or rewrite the misleading comment** so it sits with the guard it
  describes. This is half the value of the task.
- Check the other `write_assets` call site (`lib.rs:102`, the seed path) — a
  full write is correct there, so the fix must not break seeding.
- Audit the remaining writers found alongside this one for the same pattern:
  `oracle-worker`, `sdex-backfill`, `events-backfill`, `prices-ingest-core`.
- Measure before/after from `system.part_log` on the same query as above.

## 🔴 Re-scoped 2026-09-16 — the source was the seed path, not the scan

### Where this task's diagnosis was wrong

The Summary quotes `discover_window`'s unguarded `write_assets` and treats it as
the hourly re-emit. It is not, and could not have been: that branch sits behind
`if scanned > 0`, and **the ledger scan has never run on production**.
`INITIAL_DISCOVERY_LEDGER` was deliberately left unset when the worker shipped on
2026-06-25 and `prices.discovery_state` holds no cursor, so every run takes the
"seeding only" path — 83 days as of today. Full evidence in [[0256]].

The ~204k rows an hour therefore came from `ensure_seed`, the call site listed
under Implementation as one to leave alone.

⚠️ **Implemented as written, this task would not have fixed production.** The
guard would have gone into code that never executes.

### What was fixed, and where

[[0256]], PR #319, deployed 2026-09-16 12:44 UTC. `ensure_seed` captures
`AssetRegistry::watermark()` before interning the seed and writes through
`write_new_assets`, whose `since >= watermark()` short-circuit makes a
steady-state run issue no INSERT at all — the same remedy [[0132]] applied to
the ledger processor.

🔑 New information this task did not have: the staircase measured here
(623,154 / 207,741 / 415,495 on 2026-09-02) is what drives
`prices-production-oracle` into `Runtime.OutOfMemory` at its 256 MB ceiling. Both
OOMs sampled on 2026-09-12 landed immediately after a 4× read, and that alarm
accounts for ~84% of the ops channel's traffic. See [[0241]] and [[0226]].

### AC 4 audit — done 2026-09-16, five call sites

| caller | shape | verdict |
|---|---|---|
| `asset-discovery` `ensure_seed` (`lib.rs:119`) | `write_new_assets` + watermark | **fixed** in 0256 |
| `asset-discovery` `discover_window` (`lib.rs:255`) | unguarded full `write_assets` | **removed** with the ledger scan — [[0256]], PR #331 |
| `oracle-worker` (`lib.rs:569`) | full `write_assets` behind `count() > known_before` | **same defect, rarely triggered** |
| `sdex-backfill` (`sink.rs:109`, `run.rs:278`) | one-shot CLI | correct per `write_assets` docs |
| `events-backfill` (`run.rs:123`) | one-shot CLI | correct per `write_assets` docs |

🔑 `oracle-worker` is a **second live instance**: it writes the whole registry
to persist a handful of newly minted ids. Its guard keeps it quiet (no
occurrences in 24 h of logs), but the shape is wrong and `write_new_assets` with
a watermark is the fitting remedy, exactly as in the ledger processor.

### ✅ Settled by [[0256]]'s scan decision — the call site is gone

[[0256]] decided on 2026-09-21 to **delete** the ledger scan rather than switch
it on, and PR #331 removes `discover_window` with it. The unguarded
`write_assets` at `lib.rs:255` no longer exists, so there is nothing left to
guard in `asset-discovery`, and the precondition this section used to record
("enable the scan → guard first") has no scenario left to apply to.

Priority stays at `medium`: what remains is the oracle instance below.

### What this task still owns

- ~~**AC 3**~~ — moot. The misplaced comment at `lib.rs:256-262` sat inside
  `discover_window` and was deleted with it (PR #331).
- **AC 5** — `system.part_log` has not been re-measured on production. The
  13:17 UTC log check after today's deploy is a proxy, not this measurement, and
  needs ClickHouse access.
- **`oracle-worker`** — the second instance found by the AC 4 audit.
- **The read side of `ensure_seed`** — raised in PR #331's review (okarcz,
  2026-09-21). Since PR #319 a steady-state run *writes* nothing, but it still
  *reads* everything: `writer.load_assets()` pulls the whole registry (~209k
  rows, ~153 MB resident, no `FINAL`) over the AWS→Hetzner hop every hour to
  check ~20 seed identities. Remedy: a targeted
  `SELECT … WHERE (asset_code, issuer_address, contract_address) IN (<seed>)`
  plus `max(asset_id)` for the watermark — or cutting the seed stage, which
  [[0256]] left open. `eventbridge-stack.ts` sizes the worker's 512 MB on this
  load and points here. Same shape as [[0226]] (the oracle, every 5 minutes —
  twelve times as often, so that one first).

## Acceptance Criteria

- [ ] A zero-discovery hourly run writes **no** `prices.assets` rows.
- [ ] Newly-discovered assets are still persisted (seed path unaffected).
- [ ] The comment sits with the guard it actually describes.
- [ ] Other `write_assets` callers audited for the same defect.
- [ ] `part_log` shows the hourly ~204k rows gone, measured on prod.
