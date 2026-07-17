---
id: "0095"
title: "Rollup MVs → APPEND mode (stop them wiping pre-rolled history)"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0090", "0064", "0094"]
tags: [layer-infra, priority-high, effort-small, milestone-M1, clickhouse, rollup, materialized-view, data-loss]
milestone: 1
links:
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
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

- [ ] `rollups.sql` six `mv_ohlcv_*` use `REFRESH … APPEND …`; cadence/window
      re-evaluated for RMT merge load.
- [ ] **Strictly-increasing version projection** shipped with it — `max(version)`
      is only sufficient under atomic replace (0059 finding #5).
- [ ] **Regression test with rows OLDER than the refresh window** proving they
      survive a refresh, on CH pinned to 26.3.10.60. The existing
      `rollup_chain_it.rs` keeps all rows inside the window and therefore cannot
      catch this — extend or replace it.
- [ ] Prod coarse tables backed up before the change.
- [ ] MVs recreated in prod; a coarse table keeps pre-rolled history across a
      refresh AND picks up new live buckets (verified over ≥1 full cycle).
- [ ] No replace-mode refreshable MV remains on any `price_ohlcv_*` table.
- [ ] Coarse tips track the live frontier without a manual pre-roll — i.e. the
      `preroll-live-gap.sql` obligation is retired for live data.
