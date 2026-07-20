---
id: "0095"
title: "Rollup MVs → APPEND mode (stop them wiping pre-rolled history)"
type: FEATURE
status: completed
related_adr: ["0007"]
related_tasks: ["0090", "0064", "0094", "0059", "0104", "0102"]
tags: [layer-infra, priority-high, m1-blocker, effort-small, milestone-M1, clickhouse, rollup, materialized-view, data-loss]
milestone: 1
links:
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: 2026-07-17
    status: completed
    who: okarcz
    note: >
      DONE — merged (PR #121, squash fea922b) + deployed + verified on prod. All
      7 ACs met. rollups.sql six MVs → APPEND + sum(version) + aligned windows;
      preroll.sql / preroll-live-gap.sql → sum(version); new rollup_append_it.rs
      (3 tests green on CH 26.3.10.60) + negative control. Prod: six coarse
      tables backed up (verified faithful), gap pre-rolled, six MVs recreated in
      APPEND (first refresh OK), deep history byte-identical to backup, 15m tip
      advances autonomously. The last M1 blocker is cleared. Spawned 0104
      (cadence tuning). 0102/PR #118 (SCF package) un-gated and updated to
      describe the MVs as present. Housekeeping left: drop the six *_bak tables
      after a watch period.
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      FIX DEPLOYED + VERIFIED ON PROD. rollups.sql → APPEND + sum(version) +
      aligned windows; preroll.sql / preroll-live-gap.sql → sum(version); new
      rollup_append_it.rs (3 tests, all green on CH 26.3.10.60) + negative
      control (replace-mode wipes old bucket 1→0). Prod: backed up six coarse
      tables (verified faithful by FINAL fingerprint), bounded pre-roll to close
      a ~2.5 h gap, DROP + CREATE the six MVs in APPEND (first refresh of all six
      OK, empty exceptions). Verified 15m tip advances autonomously (15:30→15:45)
      and 1d/1M deep history byte-identical to backup. All ACs met. Emerged
      decisions: window alignment, sum(version) on preroll, cadence unchanged
      (spawned 0104). Follow-ups: revert 0102/PR #118 (MVs are back), drop *_bak
      after a watch period. PR pending; archive on merge.
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      Promoted backlog → active to start the fix on a fresh session, as the M1
      blocker banner directs. Coarse backup is step 1; no prod SQL until the
      backup is confirmed.
  - date: 2026-07-17
    status: backlog
    who: okarcz
    note: >
      ⛔ PROMOTED TO M1 BLOCKER by explicit decision (okarcz, 2026-07-17). The
      SCF Milestone 1 submission (0102) WAITS on this. Earlier call — "0095 is
      not an M1 blocker, do it after the backfill" — is REVERSED, on two facts
      found after it was made: (a) AC 2 requires the `prices.*` schema to match
      Section 3, and Section 3's Tranche 1 work list names the **MV chain** —
      `SHOW TABLES` does not show six of them, so a reviewer can point at the
      output and say AC 2 is unmet; (b) the rollup path is silently broken in
      prod and deleting live candles monthly, which is a far worse thing to
      surface AFTER an award than a short delay now. START THE NEXT SESSION
      HERE, fresh — not at the end of a long day. Everything needed is written
      down below; do not re-derive it.
  - date: 2026-07-17
    status: backlog
    who: okarcz
    note: >
      Re-scoped after the live-gap investigation. Three findings: (1) 0059
      ALREADY decided APPEND and it was descoped, not forgotten — see "Why this
      shipped"; (2) the fix is APPEND *plus* a strictly-increasing version, not
      one keyword; (3) the recovery window is CLOSING — coarse is now the only
      copy of history, so a mistake costs weeks of re-backfilling. The 0064
      dependency below is STALE (live is unfrozen since 2026-07-16), and the
      "not urgent" framing with it: the freeze is what was masking the loss.
      Live data has been accumulating in `1m` only and is deleted when its
      monthly partition ages out; `schema/preroll-live-gap.sql` (PR #120) is the
      recurring stopgap until this lands.
  - date: 2026-07-15
    status: backlog
    who: okarcz
    note: >
      Spawned from 0090 execution (Layer 2). Discovered during the pre-roll that
      the six mv_ohlcv_* rollup MVs are REPLACE-mode and were wiping the coarse
      tables every refresh. 0090 DROPs them as a stop-gap; this task recreates
      them correctly as APPEND.
---

# Rollup MVs → APPEND mode (stop them wiping pre-rolled history)

> ## ⛔ M1 BLOCKER — start here, on a fresh session
>
> The SCF Milestone 1 submission (**0102**) waits on this. Two reasons:
>
> 1. **AC 2 is arguably unmet.** It requires `prices.*` to match Section 3, whose
>    Tranche 1 work list names the **MV chain**. `SHOW TABLES` does not show six
>    of them. Landing this makes AC 2 literally, unarguably met — no asterisk, no
>    paragraph of explanation on camera.
> 2. **The live rollup path is broken in production and deletes candles.** Live
>    coarse only advances when an operator runs a pre-roll; anything not rolled
>    before its `1m` monthly partition ages out is gone. Surfacing that after an
>    award is a much worse conversation than a short delay now.
>
> **This is a SHORT task with a SHARP edge.** One wrong keyword deletes years of
> history, and as of 2026-07-18 **coarse is the only copy** (the `1m` feeder
> copy from the 0097 reprice is dropped by cleanup). Read "Why this shipped",
> "The fix is TWO changes", and "Why the blast radius is now maximal" **before**
> writing any SQL. The recipe is already worked out — do not re-derive it.
>
> Order of work: **back up coarse → pre-roll to a known-good current state →
> write the outside-the-window test on CH 26.3.10.60 → APPEND + version
> projection → apply → watch ≥1 refresh cycle.**
>
> Not urgent in the "do it tired" sense: `preroll-live-gap.sql` bought ~3 weeks.
> Urgent in the "it is the last thing standing between you and a clean M1" sense.

## Summary

The six `prices.mv_ohlcv_*` rollup MVs in `rollups.sql` are declared
`REFRESH EVERY <n> TO price_ohlcv_<coarse> AS SELECT … WHERE timestamp >= now()
- <window>` **with no `APPEND`**. A refreshable MV without `APPEND` *atomically
replaces* its target table on every refresh. So each MV overwrites its coarse
table with only the recent-window result — deleting all history (incl. any
pre-roll) every refresh. With live frozen the windows are empty, so they were
emptying the coarse tables outright.

Task 0090 **DROPped** the six MVs so the historical pre-roll can persist. This
task recreates them the right way.

## Context

Discovered in the 0090 pre-roll (2026-07-15): a successful pre-roll into the
coarse tables was wiped within a minute by `mv_ohlcv_1m_to_15m`
(`REFRESH EVERY 1 MINUTE`). Root cause of the long-standing "coarse tables empty"
symptom — deeper than 0090's original "pre-roll never run" diagnosis.

## Why this shipped — read before touching anything

**This was decided correctly and then lost.** Task 0059's design note
(`archive/0059_…/notes/G-rollup-version-propagation-decision.md`) states it
outright, with the ClickHouse docs cited:

> **Default (replace) mode** — *"each refresh atomically replaces the table's
> previous contents."* Combined with the bounded `WHERE timestamp >= now() -
> INTERVAL 2 HOUR`, the target then **only ever holds the last 2 h** … →
> **history loss.** Replace-mode is only safe with an *unbounded* recompute —
> too costly per grain.
>
> **Consequence:** durable + cost-bounded ⇒ **APPEND into a
> `ReplacingMergeTree(version)` target**

So the 2 h window is *right* — it bounds refresh cost. Pairing it with replace is
what is wrong. Only two designs are coherent: unbounded+replace (too expensive
every minute) or **bounded+APPEND** (what we want). We shipped bounded+replace.

**Why nobody caught it.** 0059's SCOPE was version propagation, not durability.
It closed accepting replace mode for its own question — *"the shipped chain is a
true refreshable MV in replace mode (atomic target swap), so `max(version)` is
sufficient"* — which is true, for version propagation.

**And its test could not see the bug.** `packages/prices-clickhouse/tests/rollup_chain_it.rs`
anchors every row *inside* the refresh window, deliberately (comment: rows
*"sit comfortably inside every rollup `WHERE timestamp >= now() - …`"*). With all
data inside the window, "replace the table with the last 2 h" and "replace the
table with everything" are the SAME OPERATION. The wipe was structurally
invisible. **A test that only holds fresh data can never catch this class of
bug** — which is why an outside-the-window test is an acceptance criterion below,
not a nicety.

## The fix is TWO changes, not one

1. **`APPEND`** — so a refresh inserts instead of swapping the table.
2. **A strictly-increasing `version` projection.** 0059 is explicit that this
   comes *with* APPEND:

   > **APPEND into a `ReplacingMergeTree(version)` target … reintroduces finding
   > #5 — so the version projection **must** be strictly-increasing
   > (`sum(version)` / refresh epoch) … The earlier "atomic replace sidesteps
   > finding #5" shortcut only holds for the unbounded-replace variant, which we
   > are not using.**

   `max(version)` is only sufficient under atomic replace. Appending into an RMT
   reintroduces the version-tie problem — the same one that forced a scoped
   `DELETE` before the phoenix re-roll in 0097. **Ship only the keyword and the
   table keeps its history but silently keeps the WRONG row on refresh** — worse
   than today, because it looks fine.

## Why the blast radius is now maximal

0059 also called this, in advance:

> **Retention corollary:** because the rollup is the *only* copy of its history,
> `_1m`'s retention/TTL must be **≥ the widest refresh window** of any rollup
> that reads it — otherwise a rollup bucket can never be rebuilt after `_1m`
> ages out.

`price_ohlcv_1m` retains **7 days** (monthly partitions; `cleanup-worker`). The
2024–2026 `1m` partitions written by the 0097 reprice are dropped on the first
cleanup pass after 2026-07-17. **After that, coarse is the only copy of years of
history.** A wipe would mean re-running the entire SDEX + AMM backfill (weeks)
plus the 0097 reprice. **Back the coarse tables up before touching the MVs.**

## Implementation

- Change each `mv_ohlcv_*` in `rollups.sql` to `REFRESH EVERY <n> APPEND TO …`
  so the refresh **inserts** the recent window (RMT collapses the re-inserted
  overlapping buckets by `version`) instead of replacing the whole table.
- Re-evaluate the refresh cadence vs window: `APPEND` re-inserting a 2h window
  every 1 minute = heavy write amplification (~120× duplicate buckets before
  merge). Consider a longer refresh interval or a tighter window so RMT merge
  load stays sane.
- Add the **strictly-increasing version projection** (see "The fix is TWO
  changes"). Do not ship `APPEND` without it.
- **Write the test that 0059's could not be**: seed a coarse table with rows
  OLDER than the refresh window, run a refresh, assert the old rows SURVIVE and
  the recent buckets updated. Run it against ClickHouse pinned to the prod
  version (26.3.10.60) — [[feedback-local-tests-match-prod-version]]. This test
  is the deliverable; the SQL change is the easy part.
- **Back up prod coarse first** (`CREATE TABLE price_ohlcv_1d_bak AS
  price_ohlcv_1d` + `INSERT … SELECT *`, per grain). Cheap insurance that turns
  a mistake from "re-run weeks of backfill" into "restore".
- Pre-roll immediately before the change (`schema/preroll-live-gap.sql`) so
  there is a known-good, current state to diff against.
- Recreate the six MVs in prod (`DROP` + `CREATE … APPEND`) — shared cluster,
  owner sign-off.
- Verify: after recreate, a coarse table retains its pre-rolled history AND gains
  new live buckets, and is NOT emptied on refresh. Watch at least one full
  refresh cycle before walking away.

## Dependencies

- ~~Coupled to the live-freeze fix (0064 / 0094)~~ — **STALE as of 2026-07-16**:
  live is unfrozen and writing (`behind_sec` ≈ 15 s). That dependency was also
  what made this look "not urgent" — the freeze was **masking the loss**. Live
  candles now accumulate in `1m` only, and are deleted when the monthly partition
  ages out. Measured 2026-07-17: every coarse tip was frozen at **2026-07-09**
  while `1m` was live — 8 days of candles that existed nowhere else.
- **Not blocked by the running SDEX backfill.** The MV window filters on
  `t.timestamp >= now() - 2 HOUR` — *candle* time, not insert time — so
  backfilled rows (2017-era timestamps) never enter the window. This fixes the
  **live** path only; backfilled history still needs explicit pre-rolls, which is
  what `preroll.sql` / `preroll-amm-reprice.sql` / `preroll-live-gap.sql` are for.
- **Stopgap in place:** `schema/preroll-live-gap.sql` (PR #120) rolls the live
  gap forward. It must be re-run before each month's `1m` partition ages out
  until this task lands. That recurring obligation is the cost of deferring.

## Acceptance Criteria

- [x] `rollups.sql` six `mv_ohlcv_*` use `REFRESH … APPEND …`; cadence/window
      re-evaluated for RMT merge load (kept as-is — see Design Decisions; tuning
      spawned as backlog 0104).
- [x] **Strictly-increasing version projection** shipped with it — `sum(version)`
      on the MVs *and* `preroll.sql` / `preroll-live-gap.sql`, so the coarse
      tables carry one monotonic scheme (0059 finding #5).
- [x] **Regression test with rows OLDER than the refresh window** proving they
      survive a refresh, on CH pinned to 26.3.10.60 — new `rollup_append_it.rs`
      (3 tests), plus a negative control confirming the OLD replace DDL wipes the
      old bucket (1 row → 0).
- [x] Prod coarse tables backed up before the change (six `*_bak` tables, ~18
      GiB, all six verified logical-identical by FINAL fingerprint).
- [x] MVs recreated in prod; a coarse table keeps pre-rolled history across a
      refresh AND picks up new live buckets (verified: deep history byte-identical
      to backup; `15m` tip advanced 15:30 → 15:45 autonomously).
- [x] No replace-mode refreshable MV remains on any `price_ohlcv_*` table.
- [x] Coarse tips track the live frontier without a manual pre-roll — the six
      APPEND MVs now roll live forward; the `preroll-live-gap.sql` obligation is
      retired for live data.

## Implementation Notes

Landed on branch `feat/0095_rollup-mvs-append-mode`, deployed to `ch-prod-01`
2026-07-17.

**Code (`packages/prices-clickhouse/`):**
- `schema/rollups.sql` — all six `mv_ohlcv_*`: `REFRESH EVERY <n> APPEND`;
  `max(version)` → `sum(version)`; window lower bounds aligned to the coarse
  bucket via `toStartOfInterval(now() - <window>, INTERVAL <grain>)`.
- `schema/preroll.sql`, `schema/preroll-live-gap.sql` — `max(version)` →
  `sum(version)` (both write coarse grains only, never `_1m`), so the whole
  coarse table shares one monotonic version scheme. Header rationale added.
- `tests/rollup_append_it.rs` (new) — 3 `#[ignore]` integration tests on CH
  26.3.10.60: (1) a 30-day-old bucket survives a refresh + a fresh live bucket
  is added; (2) the aligned oldest in-window bucket rebuilds complete, not
  partial; (3) `sum(version)` wins an early-minute correction that leaves
  `max(version)` tied. Negative control (ad-hoc) confirmed the replace-mode DDL
  wipes the old bucket 1 → 0.
- `tests/rollup_chain_it.rs` — updated version assertions for `sum` semantics
  (3 then 6, not 1 then 2) and the stale "replace mode / max sufficient" doc.

**Prod deploy sequence (all verified):**
1. Backed up six coarse tables (`*_bak`), verified faithful by per-grain FINAL
   `sipHash64(pk, version)` fingerprint — all six `src == bak`.
2. Bounded pre-roll `2026-07-17 13:00 → last closed 15m boundary` to close a
   ~2.5 h coarse gap (fine grains were frozen ~13:15 while `1m` was live at
   15:48) and convert the recent region to `sum(version)`.
3. `DROP … IF EXISTS` + `CREATE … APPEND` the six MVs (`allow_experimental_
   refreshable_materialized_view=1`). First refresh of all six succeeded, empty
   exceptions.
4. Verified: `15m` tip advanced 15:30 → 15:45 with no manual action; `1d` and
   `1M` deep history (`< 2025-06-01`, outside all windows) byte-identical to the
   backup.

## Design Decisions

### From Plan

1. **`APPEND` + `sum(version)`** — the two-change fix 0059 specified. `APPEND`
   stops the atomic table-replace that wiped history; `sum(version)` strictly
   increases under every real mutation (a correction raises one addend, a later
   row adds one), so the freshest/fullest aggregation wins the RMT dedup where
   `max(version)` would tie (finding #5).

### Emerged

2. **Window lower bounds aligned to the coarse bucket start.** Not in the
   original two-change framing. A raw `now() - <window>` bound falls mid coarse
   bucket, so the OLDEST in-window bucket would be re-aggregated from only its
   post-bound source slice — a partial row. With `sum(version) ≫ max(version)`
   for any multi-row bucket, that partial could outrank a complete pre-rolled
   bucket and delete history. Aligning the bound to the coarse-bucket start
   guarantees the oldest in-window bucket rebuilds complete. Proven by
   `aligned_window_rebuilds_oldest_bucket_complete`.

3. **`sum(version)` extended to `preroll.sql` and `preroll-live-gap.sql`**, so
   past and future pre-rolls share the MVs' monotonic scheme. Mixing schemes
   (preroll `max` vs MV `sum`) would let a partial MV bucket outrank a complete
   pre-rolled one, because `sum ≫ max`. Left `preroll-amm-reprice.sql`
   (archived 0097) and `preroll-incremental.sql` on `max` — some of their writes
   target `_1m`, where `max` is the canonical ledger version; window alignment
   protects their already-written rows.

4. **Cadence and windows kept unchanged** despite the AC asking to re-evaluate.
   Under APPEND each bucket is re-appended `window ÷ interval` times before
   ageing out (120× for `15m`, 400× for `1M`) — this is RMT *merge load*, not a
   correctness issue (alignment + `sum` hold at any window). Kept current values
   to minimise behaviour change on the M1-blocking deploy; tuning is better done
   against real prod merge metrics. Spawned backlog **0104**.

## Issues Encountered

- **Recipe assumed frozen coarse; prod was current.** The task/memory recorded
  coarse frozen at 2026-07-09, but the step-1 snapshot showed tips current — a
  pre-roll had been re-run since. The gap was still ~2.5 h (wider than the `15m`
  2 h window), so the pre-roll step was still needed, just for a different
  reason. Verified the gap against `now()` before sizing the pre-roll.
- **Raw `count()` on the `_bak` verify looked like data loss** (0.1–0.5% lower
  than the source snapshot). Expected: RMT background merges collapse duplicate
  -PK rows continuously, so raw counts drift. Confirmed faithful by FINAL
  (logical) fingerprint instead — all six `src == bak`.
- **`1m` holds full history right now** (`floor_1m` = 2016-03-21), not the
  assumed 7-day window — cleanup hasn't pruned the backfill/reprice partitions
  yet. Made the pre-roll trivially safe (source covers the whole gap).

## Future Work

- **0104** — re-evaluate rollup MV cadence vs window against prod merge metrics
  (the 120×/400× re-append amplification). Spawned.
- **Follow-up on the SCF package (0102 / PR #118):** it describes the six MVs as
  *dropped* in five places and adds a README step-0 pre-roll. Both are now wrong
  in the other direction — the MVs are back and roll live automatically. Update
  before the SCF submission (0102 was gated on this task).
- **Drop the `*_bak` tables** (~18 GiB) after a day or so of watching the live
  rollup hold. Not yet — they are the restore path.
