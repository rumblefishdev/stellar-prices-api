---
id: "0263"
title: "backfill_progress.current_ledger asserts a floor, not contiguous coverage — a genesis-anchored chunk makes /backfill/status claim a complete archive"
type: BUG
status: completed
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
  - date: 2026-09-08
    status: active
    who: okarcz
    note: >
      Promoted to active for the Milestone 2 pre-submission pass ([[0128]]).
      Operator decision 2026-09-08: **option 2 — gate the writer.** Gate
      `SetBackward(start)` on the same `reached_genesis` condition that
      already gates `status`, rather than only documenting the limitation or
      deriving coverage from `backfill_sdex_ledgers`. Chosen because it is the
      genuine fix and is surgical: the production row stays correct because
      that run really did reach genesis. Ships in one `sdex-backfill` release
      with [[0264]].
  - date: 2026-09-08
    status: completed
    who: okarcz
    note: >
      **CLOSED — merged (PR #294) and deployed to production 2026-09-08.** All
      five criteria met. One arm of `progress.rs` changed; the floor is now
      gated on the same `reached_genesis` condition as `status`, so the two
      cannot disagree. 27 tests pass, no new clippy findings. ⚠️ Two existing
      tests were asserting the defect and were changed deliberately. Production
      re-read after deploy: `sdex_archive` still `completed` with
      `current_ledger = 1`, and the endpoint now returns `progress_pct: 100.0`
      with `ledgers_remaining: 0`. `PCT_RUNNING_CEILING` kept and documented as
      a second line of defence rather than removed — a durable table can still
      hold pre-fix rows.
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

- [x] A decision is recorded between documenting the limitation, gating the
      writer, and deriving true coverage — with the reason, not just the choice.
- [x] A genesis-anchored partial run (`--start 1 --end N`, N below activation)
      no longer produces a `/backfill/status` payload that overstates coverage,
      by whichever mechanism was chosen.
- [x] The `covered + remaining <= span` invariant still holds across every
      reachable row shape, with the [[0176]] assertion kept green.
- [x] The production row is re-read after any writer change and still reports
      the archive as complete — it is genuinely complete, per [[0127]].
- [x] `PCT_RUNNING_CEILING` in `backfill/handlers.rs` is either removed as
      redundant or documented as a deliberate second line of defence.


## Implementation Notes

Shipped as PR #294, merged 2026-09-08, deployed the same day.

**One arm changed**, `packages/sdex-backfill/src/progress.rs`, `ExtractMode::SdexOnly`:

```rust
current_ledger: match phase {
    Phase::Running => Current::Keep,
    Phase::Completed if reached_genesis => Current::SetBackward(start as u64),
    Phase::Completed => Current::Keep,
},
```

`reached_genesis` already existed two lines below, gating `status`. The floor and
the status now move together by construction rather than by a downstream guard.

27 tests pass, no new clippy findings.

## Issues Encountered

- **Two existing tests asserted the defect.**
  `sdex_only_genesis_chunk_that_stops_short_does_not_complete` pinned
  `SetBackward(1)` — the exact overclaim this task exists to remove — as expected
  behaviour, and `sdex_only_partial_tail_does_not_complete` pinned a floor for a
  run that never started at genesis. Both changed to `Current::Keep`, with
  in-place comments saying what changed and why. Intentional, not regressions.
- **The `Combined` arm was left alone deliberately.** Its `SetBackward(start)` is
  honest — the floor is the run's own start, as its comment argues — and this
  task's reproduction is sdex-only. Changing it would have been scope creep on a
  correct code path.

## Design Decisions

### From Plan

1. **Option 2, gate the writer** — chosen over documenting the limitation or
   deriving true coverage from `backfill_sdex_ledgers`. It is the genuine fix and
   it is surgical, and the production row stays correct because that run really
   did reach genesis. Deriving coverage would have cost the O(1) read the
   endpoint is designed around.

### Emerged

2. **Under-claiming accepted as the trade-off.** A chunked run now never advances
   the floor at all, because `reached_genesis` requires one run to both start at
   genesis and reach the activation boundary. That under-claims, which is the
   safe direction; over-claiming was the defect. The alternative — carry the
   floor when adjacent to the stored one — is recorded in this task's
   Implementation section as the follow-up if chunked runs become the normal path.
3. **`PCT_RUNNING_CEILING` kept, not removed.** The criterion allowed either.
   Kept because the writer fix only protects rows written by a binary carrying
   it, and `backfill_progress` is a durable table, not a queue — a pre-fix row
   could still publish 100% beside `running` on a reviewer-facing endpoint.
   Documented in `handlers.rs` rather than left implicit. That commit went on the
   0176 branch so two branches would not both edit the same file.

## Notes

- ⚠️ **[[0176]] already owned the reader-arithmetic half of this and was not
  read first.** It states the same diagnosis — *"the stored value is CORRECT,
  the endpoint's arithmetic is wrong, do not repair the data"* — and PR #283
  independently arrived at the same fix. The duplication cost little because
  both reached the same answer, but it is a reminder to grep the backlog for
  the surface, not only for the symptom. What is left in 0176 is its Defect 2
  (a dead run still advertising `running`, and `completed_at` predating
  `last_push_at`), which is unrelated to this task.

- 🔴 **Do not "fix" this by loosening the status guard.** The 99.9 ceiling is
  what stops `progress_pct: 100.0` appearing beside `status: "running"`. It is
  crude, and it is the only thing currently preventing the optimistic reading
  on a reviewer-facing endpoint.
- The same review pass found that [[0128]]'s predecessor,
  `docs/scf/milestone-1-evidence.md`, captured a `/backfill/status` response
  under the **old forward formula** (`progress_pct: 79.47`,
  `ledgers_remaining: 13032807`) and describes the stream as *"~79 % through the
  chain"*. Under the corrected arithmetic the same row is **20.53%** — the
  submitted prose overstates coverage by roughly 4x. 🔒 **Decided 2026-09-04:
  that file is left exactly as submitted — no correction, no annotation.** The
  reasoning is in [[0127]]'s Design Decisions; do not reopen it here. The place
  to be accurate is [[0128]].
