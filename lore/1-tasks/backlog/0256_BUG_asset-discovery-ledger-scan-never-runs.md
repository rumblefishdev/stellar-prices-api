---
id: "0256"
title: "asset-discovery's ledger scan has never run on production — the worker re-seeds hourly and scans nothing"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0210", "0054", "0218", "0223", "0226", "0241"]
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

- ⚠️ Whichever way the scan decision goes, the hourly full re-seed must stop —
  it is load-bearing for [[0226]] and [[0241]]. Deleting the stage settles it;
  keeping the scan does not, unless the seed switches to `write_new_assets`.
- Decide whether the scan is still wanted at all. If `ledger-processor` already
  covers asset discovery, this stage may be redundant and the honest fix is to
  delete it rather than start it.
- If wanted: set `INITIAL_DISCOVERY_LEDGER` in `infra/envs/production.json`,
  pick a sensible starting ledger, and confirm `discovery_state` advances.
- Either way the WARN must stop being a permanent steady state — a worker whose
  main stage is skipped every hour should alarm, not log.

## Acceptance Criteria

- [ ] A recorded decision on whether the ledger scan is still needed
- [ ] If kept: `discovery_state` has a cursor and it advances between runs
- [ ] If dropped: the scan path and its config are removed, not left dormant
- [ ] The permanent WARN is gone — either the scan runs, or the code does not
      pretend it might
