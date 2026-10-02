---
id: "0273"
title: "ch-demo-queries.sql has two queries numbered (10) and two numbered (11) — the Milestone 2 block restarts the sequence its own header says it continues"
type: DOCS
status: completed
related_adr: []
related_tasks: ["0128"]
tags: [layer-docs, priority-low, effort-small, scf, evidence]
links:
  - "../../../docs/scf/ch-demo-queries.sql"
history:
  - date: 2026-09-09
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0128]] future work. Found while refreshing the Milestone 2
      evidence package for submission; offered three times during that task and
      deferred each time because it blocks nothing.
  - date: 2026-09-10
    status: active
    who: okarcz
    note: >
      Activated. Counting the M2 block before editing shows the target range in
      Implementation is off by one: the block holds **14** queries
      (10, 11, 12-22, 26), so continuing from M1's (11) lands on **(12)-(25)**,
      not (12)-(24). The (12)-(24) figure came from reading the range as
      (10)-(22) = 13 queries and missing the stray (26). Renumbering to
      (12)-(25) is what actually satisfies the acceptance criteria as written
      (no duplicates, no gaps, strictly increasing).
      Also falsified: "Nothing cross-references these numbers" holds for the
      other documents but **not inside this file** — four in-file references
      point at numbers that move.
  - date: 2026-09-10
    status: active
    who: okarcz
    note: >
      Implemented on `docs/0273_ch-demo-queries-renumber-milestone-2`,
      **PR #302** open against develop. M2 block renumbered **(12)-(25)**; the
      file now runs 1-25 with no duplicates and no gaps, and the banner's
      "numbering continues from the Milestone 1 set" is true for the first time.
      18 comment lines changed, nothing else — verified mechanically that every
      changed line begins with `--`. Milestone 1's query numbering is unmoved.
      Stays `active` until #302 merges.
  - date: 2026-09-10
    status: completed
    who: okarcz
    note: >
      ✅ **Merged in PR #302** (merge commit `2c58b47`) and verified on
      `develop`: headers run 1-25, contiguous, zero duplicates. All 5 acceptance
      criteria met, one of them corrected first — the (12)-(24) target was off
      by one against a 14-query block, so the range shipped as **(12)-(25)**.
      Two premises in the task file were falsified before any edit: that target
      range, and "nothing cross-references these numbers" (true of the other
      documents, false inside this file — four references moved with it).
      No follow-up work spawned. Nothing ran against production.
---

# `ch-demo-queries.sql` numbering collides between the two milestones

## Summary

`docs/scf/ch-demo-queries.sql` is the reviewer-facing query set behind both SCF
evidence packages. The Milestone 2 block, appended under its own banner, **starts
again at (10)** — a number Milestone 1 already uses — and then **jumps from (22)
to (26)**. Its own header claims the opposite:

> `-- Query numbering continues from the Milestone 1 set.`

So a reviewer told to "see query (10)" finds two different queries, and (23),
(24) and (25) do not exist.

## Context

Milestone 1 runs (1) through (11). The Milestone 2 banner sits at line 202 and
its first query is (10) again — "The published row for one asset, as the API
serves it" — colliding with M1's (10), "Oracle reference prices are ingested as a
cross-reference". (11) collides the same way. The final query is (26), four
numbers past its predecessor.

[[0128]]'s own acceptance criteria recorded the M2 additions as "queries
(10)-(22)", so the collision was written down as if it were the intended range
rather than caught.

**Nothing cross-references these numbers** — `milestone-2-evidence.md` does not
cite query numbers, and neither does the M1 package — so renumbering is safe and
touches one file.

⚠️ **The Milestone 1 package is frozen.** Its numbering must not move. Only the
M2 block below the banner is in scope.

## Implementation

- Renumber the Milestone 2 block **(12) through (24)**, continuing M1's (11) and
  closing the (23)-(25) hole in one pass.
- Leave every query body untouched; this is a comment-header change.
- Re-read the banner's "numbering continues from the Milestone 1 set" line and
  confirm it is finally true.

## Acceptance Criteria

- [x] No query number appears twice in the file. **25 headers, zero duplicates.**
- [x] ~~The Milestone 2 block runs (12)-(24) with no gaps.~~ **Runs (12)-(25)** —
      the (12)-(24) target was off by one, see Design Decisions #1. No gaps.
- [x] Milestone 1's (1)-(11) are unchanged. **No M1 query renumbered**; the two
      changed lines in the M1 region are a range description and a forward
      reference that pointed at (26), which no longer exists.
- [x] No query body, table name or column name is edited. **Every changed line
      begins with `--`**, checked against the diff rather than by eye.
- [x] `grep -c '^-- ([0-9]' docs/scf/ch-demo-queries.sql` equals the number of
      queries, and the sequence is strictly increasing. **26** (25 numbered +
      the unnumbered `(2b)` sub-query); strictly increasing, contiguous 1-25.

## Implementation Notes

One file, `docs/scf/ch-demo-queries.sql`, 18 comment lines changed.

Mapping applied below the `MILESTONE 2` banner (line 203): old 10-22 shift +2,
and the stray **(26) becomes (25)**.

| old | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 26 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| new | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | **25** |

Four in-file cross-references moved with the numbers they point at:

| where | was | now |
|---|---|---|
| file header range summary | `(1)-(9)` … `(10)-(22)` | `(1)-(11)` … `(12)-(25)` |
| M1 query (10) forward ref | `Query (26) below counts them` | `Query (25) …` |
| M2 (12) body | `against query (11)` | `against query (13)` |
| M2 (16) body | `matching (13)` | `matching (15)` |

## Issues Encountered

- **The banner anchor matched a wrapped header sentence.** Selecting the
  Milestone 2 banner with `startswith('-- MILESTONE 2')` also matched line 5,
  a mid-sentence wrap of the file header (`-- MILESTONE 2 banner onward, back
  milestone-2-evidence.md.`). The first run aborted on an assertion rather than
  renumbering Milestone 1 — the guard caught it because every header number
  outside the expected 10-22/26 set is a hard error, not a silent skip.
  Anchored on `'task 0128' in l` instead. **No partial write occurred.**

## Design Decisions

### From Plan

1. **Renumber the M2 block only, continuing from M1's (11)**, and close the
   (23)-(25) hole in the same pass rather than leaving it for later.

### Emerged

2. **Target range corrected from (12)-(24) to (12)-(25).** The task's own
   Implementation section and one acceptance criterion both said (12)-(24),
   derived from reading the block as "(10)-(22) = 13 queries". The block holds
   **14** — the trailing (26) was not counted. (12)-(24) cannot satisfy the
   other criteria, which require no duplicates and no gaps across 14 queries,
   so the arithmetic decided this rather than a preference.

3. **Corrected the file header's Milestone 1 range from `(1)-(9)` to
   `(1)-(11)`.** Only the M2 half of that sentence had to change. Leaving
   `(1)-(9)` beside a freshly corrected M2 range would have left a known-false
   statement in a line being edited anyway, and it is a *description* of the M1
   range, not M1's numbering — no M1 query moved. Flagged for review in #302
   because [[milestone-1-evidence-stays-as-submitted]] freezes the M1 package;
   the frozen artefact is `milestone-1-evidence.md`, and this shared query file
   has been edited since M1 (the whole M2 block was appended to it 2026-09-07).

4. **Verified mechanically, not by reading.** "No query body edited" is checked
   by asserting every changed diff line starts with `--`, and the sequence by
   extracting all headers and comparing to `range(1, 26)`. A comment-only change
   is exactly the kind that looks right when skimmed.
