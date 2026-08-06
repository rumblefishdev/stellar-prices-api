---
id: "0145"
title: "All four pre-roll scripts carry the unguarded argMax(close_usd) — 121 sites that will bake zeros into the 0088 and 0136 pre-rolls"
type: BUG
status: active
related_adr: []
related_tasks: ["0144", "0146", "0088", "0136", "0114", "0131"]
tags:
  ["priority-high", "effort-small", "clickhouse", "data-correctness", "pre-roll", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/preroll.sql"
  - "../../../packages/prices-clickhouse/schema/preroll-incremental.sql"
  - "../../../packages/prices-clickhouse/schema/preroll-live-gap.sql"
  - "../../../packages/prices-clickhouse/schema/preroll-amm-reprice.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (scope correction C1). The defect BE's
      0199 report located in the six rollup MVs is also in every pre-roll
      script — 121 further sites. Time-critical: [[0088]] pass 2 and [[0136]]'s
      gap pre-roll both run this logic at span scale.
  - date: 2026-08-06
    status: active
    who: okarcz
    note: >
      Promoted on [[0144]]'s completion — it is phase 1 of that chain and the
      only item with an external deadline. Pre-flight confirms the task's three
      premises: **all 121 sites are the byte-identical string
      `argMax(close_usd, t.timestamp)`** (6 / 14 / 6 / 95 across the four
      scripts, matching C1 exactly), `argMaxIf` appears nowhere in the schema,
      and **`preroll-amm-reprice.sql` has no generator** — no build step emits
      it and it carries no generated-file marker — so the 95 blocks are edited
      directly. Deadline pressure eased slightly: the 2026-08-06 [[0088]] check
      measured pass 2 at 20.22% and ~138k ledgers/hr, moving its ETA from
      08-09/10 to **~08-12**, and that is a floor because the run is currently
      in empty pre-2016 partitions.
  - date: 2026-08-06
    status: active
    who: okarcz
    note: >
      Implementation complete; open only on the "merged before the pre-rolls
      run" criterion. All **121** sites guarded across the four scripts, header
      disclosure in each, two guard tests (pattern + exact 6/14/6/95 counts) and
      two integration cases on CH **26.3.10.60**. The regression test is
      **verified to fail without the fix** — reverting `preroll.sql`'s six sites
      turns `price_ohlcv_15m` back to `close_usd = 0`. 11 unit + 14 integration
      tests green. Two things worth carrying to [[0146]]: the guard tests must
      assert over comment-stripped statements (the header block quotes the guard
      expression verbatim and inflated the first count 6→7), and the fixture
      must prove it reproduces the defect before asserting the fix, or the test
      is vacuous. Also pinned as a test: when *every* sub-bucket is un-enriched
      the guard correctly still yields 0 — it does not make 0 readable as "worth
      nothing", which stays [[0151]]'s problem.
---

# Pre-roll scripts carry the same unguarded `argMax(close_usd, …)`

## Summary

[[0144]] reproduced that `argMax(close_usd, t.timestamp)` with no `> 0` guard
makes a coarse row inherit `0` whenever its newest sub-bucket is not yet
enriched — discarding the priced sub-buckets underneath it. That task names the
six MVs in `rollups.sql`. **The identical expression is in every pre-roll script
as well**, and those are the scripts about to be run over large historical spans.

| File | Unguarded sites |
|---|---|
| `preroll.sql` | 6 |
| `preroll-incremental.sql` | 14 |
| `preroll-live-gap.sql` | 6 |
| `preroll-amm-reprice.sql` | 95 |
| **total** | **121** |

`argMaxIf(close_usd` appears nowhere in the schema.

## Why this is urgent rather than merely large

Two pre-rolls are queued against exactly this code:

- **[[0088]] pass 2** finishes ~2026-08-09/10 and needs its output pre-rolled.
- **[[0136]]**'s 2026-07-21→08-03 freeze gap needs a bounded incremental
  pre-roll.

Run either against today's scripts and it manufactures a fresh estate of coarse
rows whose `close_usd` is 0 despite priced sub-buckets underneath — at backfill
scale, over spans where enrichment is by definition incomplete at pre-roll time.
Those rows then age out of the MV re-aggregation windows, at which point only
the [[0114]] sweep can reach them ([[0148]]).

Fixing the scripts is the cheapest link in the whole [[0144]] chain: they are
plain SQL scripts, not provisioned objects, so there is **no [[0142]] no-op
trap, no DROP window and no freshness exposure**. It merges and it is live.

## Implementation

- Replace `argMax(close_usd, t.timestamp)` with
  `argMaxIf(close_usd, t.timestamp, close_usd > 0)` at all 121 sites.
- **Check `preroll-amm-reprice.sql` for a generator first.** 95 near-identical
  unrolled blocks is not hand-written; if a generator exists, fix it and
  regenerate rather than editing the output.
- Note in each file header that `close` and `close_usd` may now come from
  different sub-buckets — the same disclosure [[0146]] owes `rollups.sql`.
  An approximately-right USD close beats a fabricated zero, but the two columns
  silently ceasing to be same-row will bite a future reader.
- Regression test on CH **26.3.10.60** (the prod pin): a span whose newest
  sub-bucket is unpriced must pre-roll to the latest *priced* close, not 0.
  [[0144]]'s `repro/03_tests.sql` TEST A is the shape to copy.

## Acceptance Criteria

- [x] No `argMax(close_usd` remains in any `preroll*.sql` — all **121** sites are
      now `argMaxIf(close_usd, t.timestamp, close_usd > 0)`. Two guard tests in
      `src/lib.rs`: `no_preroll_script_uses_an_unguarded_argmax_on_close_usd`
      (the pattern cannot be reintroduced) and
      `preroll_guarded_close_usd_site_counts_match_the_0144_audit` (6/14/6/95 —
      catches a projection added without the guard, or one silently dropped).
- [x] `preroll-amm-reprice.sql` is **not** generated — no build step emits it, no
      generated-file marker, no reference to it outside the schema dir and the
      0097 runbook. The 95 blocks were edited directly. Criterion resolved as
      not-applicable rather than skipped.
- [x] Regression test on 26.3.10.60, green —
      `tests/preroll_close_usd_guard_it.rs`, two cases through the real shipped
      `preroll.sql` chain. **Verified to fail without the fix** (reverting the
      six `preroll.sql` sites turns `price_ohlcv_15m` back to `close_usd = 0`).
- [ ] Merged **before** the [[0088]] pass-2 pre-roll and the [[0136]] gap
      pre-roll are run. → PR open; 0088 pass 2 now ETAs ~08-12.
- [x] Header disclosure of the `close` / `close_usd` decoupling in each of the
      four files.

## Implementation Notes

Four schema files, one Rust file, one new test file.

| File | Change |
|---|---|
| `preroll.sql` | 6 sites + header block |
| `preroll-incremental.sql` | 14 sites + header block |
| `preroll-live-gap.sql` | 6 sites + header block |
| `preroll-amm-reprice.sql` | 95 sites + header block |
| `src/lib.rs` | 3 new `include_str!` consts + `ALL_PREROLL_SQL`; 2 guard tests |
| `tests/preroll_close_usd_guard_it.rs` | new — 2 integration cases |

All 121 sites were the **byte-identical** string `argMax(close_usd, t.timestamp)`,
so the replacement was mechanical and total. The 121 `argMax(close, t.timestamp)`
sites were deliberately left alone: `close` is a real traded price with no
sentinel, so guarding it would be meaningless.

Only `preroll.sql` was reachable from Rust before this task. The other three are
operator-run scripts; they are now embedded via `include_str!` **solely so the
test suite can see them** — `apply_sql` is never pointed at the incremental
three, and the doc comment on `ALL_PREROLL_SQL` says so, because an embedded
const looks like an appliable one.

Tests: 11 unit + 14 integration green on CH **26.3.10.60** (confirmed
`SELECT version()`, not just the compose pin).

## Design Decisions

### From Plan

1. **`argMaxIf(close_usd, t.timestamp, close_usd > 0)`** — the expression 0144
   settled and measured; adopted unchanged so the pre-rolls, `current.sql`
   ([[0135]]) and the rollup MVs ([[0146]]) all read identically.

### Emerged

2. **Guard tests assert over `split_statements`, not raw file text.** The first
   version counted raw text and failed at 7-not-6: each header disclosure block
   quotes the guard expression verbatim, so comments inflated every count by
   one. Stripping comments makes the guard immune to its own documentation.
   Worth knowing before writing the equivalent guard in [[0146]].

3. **A second guard test pinning the exact per-file counts (6/14/6/95).** The
   "no unguarded argMax" test alone passes trivially if someone deletes a
   `close_usd` projection outright. The count test is what makes silence
   meaningful, and it re-asserts 0144's C1 audit as executable rather than prose.

4. **The regression test proves the fixture reproduces the defect first.** Before
   asserting the fix, it runs the OLD unguarded expression over the same input
   and asserts it returns 0. Without that, a fixture that would have produced
   7.0 anyway would make the whole test vacuous — and this is precisely the
   class of test that quietly stops testing anything.

5. **A second integration case pins what the guard does NOT fix.** When every
   sub-bucket is un-enriched, `argMaxIf` matches no rows and the `Decimal`
   default puts 0 back. That is correct, but it means a 0 in these tables still
   cannot be read as "worth nothing". Pinned as a test and stated in each file
   header so the guard is not mistaken for a stronger promise. → [[0151]]

6. **Header disclosure states the decoupling as a cost, not a footnote.**
   `close` and `close_usd` may now come from different sub-buckets. An
   approximately-right USD close beats a fabricated zero, but the two columns
   silently ceasing to be same-row is exactly the kind of thing a future reader
   assumes rather than checks.

## Issues Encountered

- **`cargo clippy -p prices-clickhouse --all-targets` fails on
  `tests/rollup_append_it.rs:10-11`** (`doc_lazy_continuation`, two errors).
  **Pre-existing** — reproduced on a clean stash of develop, in a file this task
  does not touch. Not fixed here: CI's clippy step lints only
  `extractors-core`, the three extractor crates and `ledger-processor`, so this
  package is unlinted and the failure is invisible to CI. Left alone rather than
  folded into an unrelated data-correctness PR. Worth its own chore task if the
  clippy step is ever widened.
- **`preroll.sql` alone used column-aligned `AS close_usd`;** the guarded
  expression is longer than the alignment column, so those six lines were
  normalised to single-space. The other three files already used single-space.
