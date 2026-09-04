---
id: "0176"
title: "GET /backfill/status publishes an impossible state — 'completed, 0%, 63.8M remaining' for SDEX and a 28-day-dead soroban_amm still reading 'running'"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0127", "0263", "0088", "0072", "0136"]
tags: [layer-api, priority-medium, effort-small, backfill, observability, consumer-facing]
links: []
history:
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      Spawned from 0088's last open AC ("GET /backfill/status monotonic"). The
      AC could not be closed: measured on prod 2026-08-11, the endpoint reports
      sdex_archive as completed AND 0.0% AND 63,795,748 ledgers remaining, and
      soroban_amm as running with a last push 28 days old. Two independent root
      causes, one surface. 0088's own notes had already predicted the second.
  - date: 2026-09-04
    status: backlog
    who: okarcz
    note: >
      ⚠️ **Defect 1 is FIXED — but by [[0127]], not by this task, which nobody
      read first.** PR #283 (`d9de725`) corrected `progress_pct` and
      `ledgers_remaining` for the backward stream exactly as this task
      prescribes, including the instruction not to repair the data: the reader
      was changed, the stored `current_ledger = 1` was left alone. Found
      independently while closing Tranche 2's backfill gate, so the two were
      solved twice up to the point of discovery. **Defect 2 is untouched.**
      Re-scope before starting: what remains here is the dead-run status and
      the timestamp inconsistency, not the arithmetic.
---

# `/backfill/status` publishes a self-contradictory state

> ## 🗄️ Defect 1 is FIXED — 2026-09-04, PR #283 (`d9de725`)
>
> Corrected in `packages/prices-api/src/backfill/handlers.rs` exactly as this
> task prescribes. `progress_pct` is now `(target − current) / (target − start)`
> and `ledgers_remaining` is `current − start`; the completed archive reads
> **100% with 0 remaining** instead of 0% with 63,795,748. 🔑 This task's
> instruction was followed to the letter: **the reader was fixed and the stored
> `current_ledger = 1` was left untouched.** Five unit tests plus the amended
> integration test cover it.
>
> Two things this task did not anticipate, both now guarded:
> a non-`completed` stream is held under 100% (a genesis-anchored chunk writes
> `current_ledger = 1` while still `running`, which the corrected arithmetic
> would otherwise read as finished — that residue is [[0263]]), and
> `ledgers_remaining` is clamped to the span because `current > target` is
> reachable.
>
> ⚠️ **Defect 2 below is untouched and is the whole remaining scope**: a dead
> run that still advertises `running`, and `completed_at` predating
> `last_push_at`. AC 1 and AC 4 are met; AC 2, 3 and 5 are not.


## Summary

Two independent defects make the public `GET /v1/backfill/status` endpoint
report things that cannot all be true at once. Measured on prod 2026-08-11:

```
task_name     status     current_ledger  start_ledger  target_ledger  last_push_at         completed_at
sdex_archive  completed  1               1             63795749       2026-08-11 02:43:32  2026-07-27 21:24:26
soroban_amm   running    63352611        50457424      63475475       2026-07-14 17:54:24  NULL
```

## Defect 1 — writer and reader disagree on what `current_ledger` means

🔴 **The stored value is CORRECT. The endpoint's arithmetic is wrong.** Do not
"repair" the data; that would corrupt a right answer to hide a reader bug.

`resolve_current` (`packages/sdex-backfill/src/sink.rs:365-373`) has two modes:

```rust
Current::SetForward(v)  => existing.map_or(v, |e| e.max(v)),   // furthest FORWARD
Current::SetBackward(v) => …e.min(v)…                          // furthest BACK
```

The pre-Soroban tail walks **downward** toward genesis, so for that stream
`current_ledger` means *"furthest back reached"*. Pass 2 reached ledger 1, so
`current_ledger = 1` is exactly right and monotonic **in its own direction**.

But `progress_pct` (`packages/prices-api/src/backfill/handlers.rs:68-75`) and
`ledgers_remaining` (`handlers.rs:41`) both assume a forward position:

```
done              = current_ledger − start_ledger = 1 − 1          = 0  -> 0.0%
ledgers_remaining = target_ledger  − current_ledger = 63,795,749−1  = 63,795,748
```

So a fully completed backfill publishes **`completed`, 0.0%, 63,795,748
remaining**. A consumer cannot reconcile those, and `progress_pct` is the field
most likely to be rendered on a status page.

**This is a semantics bug, not a data bug.** The fix belongs in the read model —
either the DTO distinguishes forward from backward streams, or the row carries
its direction so the reader can compute the right thing. Note that
`sdex_archive` is a *single* stream that has been walked in **both** directions
(forward to the tip, backward to genesis), so "which direction is this stream"
may not be answerable from one field — that is the design question this task has
to settle.

## Defect 2 — nothing writes a terminal state when a run dies

`soroban_amm` has read `status: running` since **2026-07-14** with
`completed_at: NULL`. The run is long dead. `resolve_status`
(`sink.rs:381-388`) only transitions on a push from a live run, so a crashed or
killed run leaves its row asserting it is still working — **indefinitely**.

0088 already flagged this class:

> Same class of lying health signal as the sweep that reported nothing and the
> [[0136]] freeze.

Consumer impact: the API advertises an in-flight AMM backfill that stopped 28
days ago. Anything gating on `status != 'running'` waits forever.

Also inconsistent on `sdex_archive`: `completed_at` (2026-07-27, pass 1) is
**earlier** than `last_push_at` (2026-08-11, pass 2). A completion timestamp
that predates the last write is another signal a consumer cannot interpret.

## Implementation

- Decide the read-model contract for a stream walked in both directions, then
  fix `progress_pct` / `ledgers_remaining` to match. Consider publishing the
  covered *range* rather than a scalar position — `earliest_data_available` is
  already on the row and already correct (`2015-11-18` for `sdex_archive`).
- Give a stalled stream a way to stop claiming it is running. Options: a
  freshness rule derived from `last_push_at` (the freshness probe in
  `packages/backfill-freshness-probe/` already reads this table), an explicit
  terminal write on shutdown, or a reaper. ⚠️ A shutdown hook alone does not
  cover a hard kill or a power cut — 0088 saw both.
- Reconcile `completed_at` vs `last_push_at` so completion cannot predate the
  last write.

## Acceptance Criteria

- [x] `/backfill/status` cannot publish `completed` together with a non-100%
      `progress_pct`, in either walk direction. Assert it as a test, not by
      inspection.
- [ ] A stream whose last push is far in the past does not report `running`.
- [ ] `completed_at >= last_push_at` holds, or the two fields are given
      meanings that make the ordering irrelevant and that is documented.
- [x] `sdex_archive`'s stored `current_ledger = 1` is **preserved**, not
      overwritten — the fix is in the read path. A test should pin this so a
      later "cleanup" does not silently reintroduce the bug by mutating data.
- [ ] Prod re-checked after the fix and the actual output recorded.

## Notes

- Found closing 0088's last AC. `backfill_status_maps_both_streams`
  (`packages/prices-api/tests/endpoints_it.rs`) passes and is not wrong — it
  covers stream *mapping*, not the arithmetic or staleness, so this defect sits
  in its blind spot.
- ⚠️ **Do not resolve this with a manual `UPDATE` on `prices.backfill_progress`.**
  Defect 1's data is correct, and hand-patching defect 2 fixes one row while
  leaving the mechanism that produced it intact — the next crash reproduces it.
- Related: [[0072]] shipped the `/price` surface and found a similar
  "measure on the right table" trap; [[0136]] is the canonical lying-health-
  signal precedent.
