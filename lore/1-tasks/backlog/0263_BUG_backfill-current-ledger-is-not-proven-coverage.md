---
id: "0263"
title: "backfill_progress.current_ledger asserts a floor, not contiguous coverage — a genesis-anchored chunk makes /backfill/status claim a complete archive"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0127", "0088", "0128", "0176"]
tags: [layer-backend, layer-api, priority-medium, effort-medium, milestone-M2, backfill, api, verification]
milestone: 2
links:
  - "../../../packages/prices-api/src/backfill/handlers.rs"
  - "../../../packages/sdex-backfill/src/progress.rs"
history:
  - date: 2026-09-04
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0127]], found by code review on PR #283. That PR fixed the
      *direction* of `progress_pct` / `ledgers_remaining` and shipped a status
      guard so a non-`completed` stream can never publish 100%. The guard is a
      containment, not the fix: the underlying column still cannot distinguish a
      finished archive from a genesis-anchored chunk, and the endpoint it feeds
      is the one Tranche 2 AC 5 sends a reviewer to.
---

# `current_ledger` is a floor claim, not a coverage proof

## Summary

`prices.backfill_progress.current_ledger` for the `sdex_archive` stream is
**the lowest run `start` that has ever completed** — not the oldest ledger
whose data is provably present, and not evidence that everything between that
floor and `target_ledger` was ingested.

`GET /v1/backfill/status` derives both `progress_pct` and `ledgers_remaining`
from it, so both inherit the weakness.

## Context

`sdex-backfill`'s `progress.rs` writes the terminal update as:

```rust
current_ledger: match phase {
    Phase::Running   => Current::Keep,
    Phase::Completed => Current::SetBackward(start as u64),
},
```

`Phase::Completed` is reached by **any** run that finishes successfully, and
`SetBackward(start)` is written **unconditionally** — the `reached_genesis`
check on the line below gates only `status`, not `current_ledger`.

So the chunking pattern the module's own doc comment recommends —
`--mode sdex-only --start 1 --end 20_000_000` — writes `current_ledger = 1`
while `[20_000_000, activation)` has never been touched. `status` correctly
stays `running`; `current_ledger` says genesis.

**The two disagree, and only `status` is telling the truth.**

⚠️ This is pre-existing and is **not** a regression from PR #283. It matters
more after that PR only because the corrected backward arithmetic turns the
inconsistency from a pessimistic reading into an optimistic one: the old
forward formula reported such a row as `0.0%`, the corrected one as `100%`.
PR #283 holds a non-`completed` stream under `PCT_RUNNING_CEILING` (99.9) so it
cannot assert completion the status does not support — which stops the
self-contradiction without making the number true.

## Implementation

- Decide what the endpoint should publish. Three shapes, in rising cost:
  1. **Document the limitation and stop there.** `current_ledger` is a floor
     claim; say so in the OpenAPI text (PR #283 already added a caveat
     sentence) and let the reviewer read `status` for completion. Cheapest,
     and arguably correct — the field's contract simply is not "coverage".
  2. **Make the writer honest.** Only carry `current_ledger` down when the run
     actually extends the covered span contiguously — i.e. gate
     `SetBackward(start)` on the same `reached_genesis` condition that gates
     `status`, or on the stored floor being adjacent to this run's `end`.
     Changes the writer, so it needs a backfill-side release.
  3. **Derive coverage from the inventory.** `prices.backfill_sdex_ledgers`
     records completed sequences; a true floor is the bottom of the contiguous
     run below `target_ledger`. Correct, but it costs the **O(1) read** this
     endpoint was designed around (§4.5) — measure before choosing it, and
     consider a periodically-materialised value rather than a live scan.
- Whichever is chosen, keep the `covered + remaining <= span` invariant
  [[0176]] asked for. PR #283 added a test asserting it across the reachable
  states.
- If (2) or (3): re-check the production row afterwards. It currently reads
  `current_ledger = 1`, `status = completed`, and that combination **is**
  believed truthful — [[0127]] corroborated it against
  `min(timestamp) = 2015-11-18` in `price_ohlcv_1d` and `201511` as the oldest
  active partition. A stricter writer must not regress a correct row.

## Acceptance Criteria

- [ ] A decision is recorded between documenting the limitation, gating the
      writer, and deriving true coverage — with the reason, not just the choice.
- [ ] A genesis-anchored partial run (`--start 1 --end N`, N below activation)
      no longer produces a `/backfill/status` payload that overstates coverage,
      by whichever mechanism was chosen.
- [ ] The `covered + remaining <= span` invariant still holds across every
      reachable row shape, with the [[0176]] assertion kept green.
- [ ] The production row is re-read after any writer change and still reports
      the archive as complete — it is genuinely complete, per [[0127]].
- [ ] `PCT_RUNNING_CEILING` in `backfill/handlers.rs` is either removed as
      redundant or documented as a deliberate second line of defence.

## Notes

- 🔴 **Do not "fix" this by loosening the status guard.** The 99.9 ceiling is
  what stops `progress_pct: 100.0` appearing beside `status: "running"`. It is
  crude, and it is the only thing currently preventing the optimistic reading
  on a reviewer-facing endpoint.
- The same review pass found that [[0128]]'s predecessor,
  `docs/scf/milestone-1-evidence.md`, captured a `/backfill/status` response
  under the **old forward formula** (`progress_pct: 79.47`,
  `ledgers_remaining: 13032807`) and describes the stream as *"~79 % through the
  chain"*. Under the corrected arithmetic the same row is **20.53%** — the
  submitted prose overstates coverage by roughly 4x. That is submitted evidence
  and its handling is a call for the operator and the team, tracked in
  [[0127]], not here.
