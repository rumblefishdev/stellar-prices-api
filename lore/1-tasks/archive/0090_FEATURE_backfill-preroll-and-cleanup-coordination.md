---
id: "0090"
title: "Backfill loses history — wire preroll + cleanup-coordination into the backfill workflow"
type: FEATURE
status: done
related_adr: ["0007"]
related_tasks: ["0088", "0053", "0039", "0051", "0059", "0060"]
tags: [layer-infra, priority-high, effort-medium, milestone-M1, backfill, clickhouse, retention, rollup, data-loss, blocker]
milestone: 1
links:
  - "../../../docs/runbooks/continue-soroban-backfill.md"
  - "../../../packages/prices-clickhouse/schema/preroll.sql"
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../packages/cleanup-worker/src/lib.rs"
history:
  - date: 2026-07-09
    status: backlog
    who: okarcz
    note: >
      Discovered while running the 0088 backfill from a second machine. The
      backfill writes price_ohlcv_1m only; the live rollup MVs are refreshable
      LIVE-only (2h window) and ignore historical rows; the cleanup worker
      drops historical 1m partitions nightly (7d retention). Net: every
      backfilled candle is deleted un-rolled — the coarse forever-tables
      (1h/4h/1d/1w/1M) that the BE consumer reads are empty. BLOCKS 0088.
  - date: 2026-07-14
    status: backlog
    who: okarcz
    note: >
      Root-caused a SECOND, distinct failure: the Soroban-era backfill also
      STOPPED early (ledger 62,642,957) and was never resumed → unfilled gap
      62,642,957→63,352,611 (~710k ledgers). CH forensics: candles ahead of the
      completed-ledger batch = external mid-loop KILL (suspend/reboot/OOM), not a
      clean exit; proto27 (0091) and parse-error ruled out. No watchdog on the
      manual run → a momentary ~07-08 kill became a lasting gap. The re-run fixes
      both (gap + durability). Also: the "live ingestion stopped" side-note is a
      separate thread — Protocol-27, now task 0091; live processor is recovering.
  - date: 2026-07-14
    status: backlog
    who: okarcz
    note: >
      RE-SCOPED after measuring surviving data: Soroban-era 1m SURVIVED
      (contiguous 2024-02→2026-05; cleanup DISABLED), only June-2026 is a hole
      (the 62.6M→floor gap). Dropped the blanket TRUNCATE+re-download. New plan:
      Phase 1 pre-roll existing 1m (2.3yr durable history, no download); Phase 2
      gap-fill June only; Phase 3 deep 2015-2024 tail = DECISION → task 0092.
      Spawned 0092 (deep tail) + 0093 (backfill watchdog + live freshness alarm).
  - date: 2026-07-14
    status: active
    who: okarcz
    note: >
      Promoted backlog → active; starting the backfill fix. First deliverable
      is the non-prod-gated dev work: wire pre-roll + cleanup-coordination
      steps into docs/runbooks/continue-soroban-backfill.md (AC #4) and
      document the cleanup re-enable procedure (AC #5). Phase 1 pre-roll and
      Phase 2 gap-fill EXECUTION run on shared ch-prod-01 → owner sign-off +
      approval-gated, deferred to an explicit run. Branch
      feat/0090_backfill-preroll-cleanup-coordination.
  - date: 2026-07-14
    status: active
    who: okarcz
    note: >
      REVISED phase order: gap-fill FIRST (Phase A), then ONE full pre-roll last
      (Phase B), replacing the original pre-roll-first / re-roll-the-gap plan.
      Rationale: preroll.sql is full-range (no WHERE), so pre-roll-first = two
      full aggregations on the shared node + a ReplacingMergeTree reconciliation
      of the partial June bucket; fill-first = one aggregation over contiguous
      data, correct in a single pass. Safe because cleanup stays disabled until
      Phase B. Insurance pre-roll-first only wins if Phase A is slow/uncertain
      (home-line days), not a ~1h EC2 fill. Renamed Phases 1/2/3 to A/B/C and
      reordered the ACs to match.
  - date: 2026-07-14
    status: active
    who: okarcz
    note: >
      Phase A run procedure PREPARED (not executed — prod-gated). Added §6b to
      docs/runbooks/fix-backfill-history-loss-and-rerun.md: a bounded gap-fill
      that replaces the full-range Step 4 — live-progress preflight (resume from
      min tracker + 1 ≈ 62,592,000, not a hard-coded start, since the kill left
      trackers behind the last candle), the exact `sdex-backfill --mode combined
      --start … --end 63352611 --transport hetzner` command in tmux with the
      writer certs, and a close-the-hole verify. Also reordered the runbook's
      superseding banner to gap-fill-first. Execution still needs owner sign-off.
  - date: 2026-07-15
    status: active
    who: okarcz
    note: >
      EXECUTED Phase A+B on prod. Phase A already satisfied by an unnoticed
      run-full-backfill.sh (running since 07-08, Phase 1 complete → gap filled;
      verified 1m contiguous 2024-02→2026-07, 532M rows). Killed its deep tail
      (Phase 2) — BE confirmed Soroban-era-only, resolving 0092 — and deleted the
      28-row pre-Soroban fragment. Phase B pre-roll needed 3 unplanned fixes
      (see Execution findings): (1) chunk preroll.sql by year to fit the 5.59 GiB
      quota; (2) ★ DROP the six replace-mode mv_ohlcv_* MVs that were WIPING the
      coarse tables every refresh — the real root cause of the empty coarse tables,
      deeper than the original diagnosis; (3) re-run the chunked pre-roll. Spawned
      Layer-2 follow-up (MVs → APPEND, coupled to the live-freeze fix). Cleanup
      re-enable (AC #5 execution) still pending pre-roll completion + verify.
  - date: 2026-07-15
    status: done
    who: okarcz
    note: >
      DONE. Phase B pre-roll landed durably: chunked by year + dropped FINAL on the
      dup-free intermediate stages to fit the 5.59 GiB quota, after DROPping the six
      replace-mode mv_ohlcv_* MVs (the real wiper). All six coarse tables populated
      (15m 151.7M → 1M 1.3M, monotonic-decreasing = no double-count), 1h/1d hold
      sdex+aquarius+phoenix 2024→2026-07-09. Cleanup rule prices-production-cleanup
      re-ENABLED after verify. BE has its durable 1h/1d history. Spawned follow-ups:
      0095 (rollup MVs → APPEND, Layer 2) and 0096 (backfill preload pool_registry —
      soroswap has 0 candles, invisible to the backfill, a separate coverage gap).
      Unblocks 0088.
---

# Backfill loses history — wire preroll + cleanup-coordination into the backfill workflow

## Summary

The historical backfill (`sdex-backfill`, task 0088) writes candles **only** to
`prices.price_ohlcv_1m`. But `price_ohlcv_1m` is a **transient 7-day feeder**, not
a store of record. The durable history is supposed to live in the forever-retained
coarse tables (`price_ohlcv_1h/4h/1d/1w/1M`), populated by the rollup chain. Two
facts make the backfill's output vanish:

1. The rollup MVs (`schema/rollups.sql`) are **refreshable, LIVE-only, `now() - INTERVAL 2 HOUR`** windowed — they deliberately ignore historical/backfilled rows (their own header says so). Historical data must be rolled up by **`schema/preroll.sql`** instead.
2. The **cleanup worker** (`prices-{env}-cleanup`, EventBridge `cron(0 2 * * *)`) drops every monthly `1m` partition older than 7 days — i.e. **all** backfilled history — nightly.

The backfill runbook has no preroll step and no cleanup coordination, so backfilled
`1m` data is written, never pre-rolled, and partition-dropped (often within hours,
at the next 02:00–03:00 UTC cleanup). **The coarse forever-tables are empty; the BE
consumer's 1h/1d surface has no history.**

This is a **workflow gap, not a code bug** — the extractor and rollup SQL are
correct (proven below). It **blocks 0088** (the backfill produces nothing durable
until fixed).

## Backfill also STOPPED early — a SECOND, distinct failure (root-caused 2026-07-14)

Beyond losing durability (above), the Soroban-era backfill **stopped short and was
never resumed**, leaving an unfilled ledger gap **62,642,957 → 63,352,611** (~710k
ledgers, the death point up to the floor). Forensics on ch-prod-01:

- Stopped **mid-partition**: candles reach ledger **62,642,956**, but the per-partition
  `backfill_sdex_ledgers` completed-batch only reaches **62,617,423**, and
  `backfill_progress.soroban_amm` sits at **62,591,999**. Candles *ahead of* the
  completed-ledger batch (which is written once, at end-of-partition) is the signature
  of a **mid-loop kill**, not a clean error exit or completion.
- **Ruled out:** proto27 (its parse boundary 63,401,875 is *above* the backfill floor,
  never reached — see task 0091); XDR parse error (aborts *before* the completed batch,
  yet completed rows exist); transient CH/AWS errors (the sink retries all of these; a
  persistent one exits cleanly with a logged error).
- **Cause = external host-level kill** of the operator backfill box (suspend/sleep,
  reboot, or OOM). Inferred from the CH evidence; *which* kill is not yet confirmed —
  pending `~/backfill.log` + `journalctl`/`last -x` on the backfill machine.
- **Why it became a lasting gap:** the manual `continue-soroban-backfill` process has
  **no watchdog / auto-restart / alarm**, so a single kill ended it permanently and it
  was never re-run. A momentary kill (~2026-07-08) became a 6-day+ gap.

The task's re-run (below) fixes **both** failures at once — it re-processes the
62.6M→floor gap (and the 0%-done pre-Soroban tail) *and* pre-rolls for durability.
Consider a follow-up to supervise/alarm the backfill so an interruption can't silently
persist (the live processor has a doorbell-lag alarm; the backfill has nothing).

## Context / Evidence (measured on ch-prod-01, 2026-07-09)

- `price_ohlcv_1m`: 40.4M sdex rows, oldest `2024-07-11`, newest `2026-07-08 00:37`.
- `price_ohlcv_15m` / `_1h` / `_1d`: **empty**. `_1M`: only `2026-07-01`, ~5k rows.
- `SHOW CREATE TABLE price_ohlcv_1m` → **no TTL** (retention is the cleanup worker, not DDL).
- `system.query_log`: nightly `ALTER TABLE prices.price_ohlcv_1m DROP PARTITION 2024xx`
  at ~03:00 UTC (rows: 202403–202407 dropped 2026-07-09 03:00).
- Rollup MVs present in prod: `mv_ohlcv_1m_to_15m … _1w_to_1M` (all 6 + `mv_current_prices`).
- `rollups.sql` MV body: `FROM price_ohlcv_1m FINAL WHERE t.timestamp >= now() - INTERVAL 2 HOUR`.
- Extractor proven correct: local decode of archive ledger `51050000` (pre-floor,
  no candles in DB) → 133 SDEX trades → **114 candles** through the real pipeline
  (`decode_object` → `extract_trades` → `raw_trade_to_tick` → `CandleAccumulator`).
  Archive files below the floor are full-size (data is present). So candles are
  generated correctly; they are lost at the destination, not at extraction.
- Cleanup retention (`cleanup-worker/src/lib.rs`): `1m`=7d, `15m`=30d, `oracle`=13mo;
  `1h/4h/1d/1w/1M` retained forever.

Also observed (separate thread, now tracked as **task 0091**): live ingestion went
stale (~2026-07-08) in the **Protocol-27** window. Investigated 2026-07-14 — the
`prices-production-ledger-processor` Lambda is actually **healthy and catching up**
(proto27 decode impact TBD at the ledger-63,401,875 crossing); this is **NOT** the same
as the backfill's stop above. See task 0091 / memory `proto27-xdr26-live-freeze`.

## Recommended plan (RE-SCOPED 2026-07-14 after measuring the surviving data)

Measurement changed the plan. The Soroban-era `1m` data **survived** — contiguous
`2024-02 → 2026-05` (verified by per-month counts), and the cleanup rule is currently
**DISABLED**, so nothing is dropping it. Only **June 2026 is a hole** (the 62.6M→floor
gap). So the original "TRUNCATE `backfill_sdex_ledgers` + re-download the whole chain"
is **wasteful** — it would re-fetch ~94% of data that already exists (weeks). Instead:

> **Ordering (revised 2026-07-14): gap-fill FIRST, then a single full pre-roll.**
> `preroll.sql` has no `WHERE` — every run re-aggregates the *entire* `1m` range.
> So pre-rolling before the gap is filled means **two** full-range aggregations on
> the shared node (one now + one after the fill) plus a `ReplacingMergeTree`
> reconciliation of the partial June bucket. Filling the gap first and pre-rolling
> **once** over contiguous data is cheaper (one aggregation), correct in a single
> pass (no straddle-bucket replace), and simpler to verify. Safe because cleanup
> stays **disabled** until the final pre-roll, so the surviving 2.3 yr `1m` is not
> at risk during the fill. *(The old "pre-roll-first for insurance" order only
> wins if Phase A will be slow/uncertain — e.g. run on a home line over days —
> where locking the 2.3 yr into the forever-tables up front hedges the wait. On a
> ~1 h us-east-2 EC2 fill, it doesn't.)*

**Phase A — Gap-fill the one hole (`62,642,957 → 63,352,611`, ~710k ledgers = the
missing June-2026 range).** Bounded backfill of just this range — all **below** the
proto27 boundary (63.4M), so `stellar-xdr 26` is fine (no 0091 dependency). Pool
registry already seeded in prod → AMM resolves. ~1h on a us-east-2 EC2 / ~1 day
local. **Do NOT** TRUNCATE `backfill_sdex_ledgers`; **do NOT** re-run the full chain.

**Phase B — One full pre-roll over the now-contiguous `1m` (do LAST).**
Apply `preroll.sql` to the completed `price_ohlcv_1m` → populates the coarse
`1h/4h/1d/1w/1M` tables in a single pass across `2024-02 → tip-of-backfill`
(~2.3 yr durable SDEX history + the filled June gap). Idempotent (optionally
`TRUNCATE` coarse first for a clean rebuild). Runs heavy aggregation on the
**shared** ch-prod-01 → owner sign-off + low-traffic window + spill-to-disk flags.

**Phase C — Deep pre-Soroban tail (2015 → 2024-02): a DECISION, not automatic.**
The only expensive piece (~50M ledgers, multi-week). Split to **task 0092** — decide
with the BE consumer (0199) whether pre-2024 history is needed before downloading.

**Cleanup:** already disabled (safe — protects the 1m meanwhile). Re-enable
(`aws events enable-rule --name prices-production-cleanup`) only **after** the
Phase B pre-roll is verified, so the coarse tables have captured the history before
the redundant `1m` partitions drop.

**Monitoring:** the backfill had no watchdog (a host kill became a 6-week gap silently);
the live doorbell-lag alarm is blind to a drains-but-doesn't-write processor. Both split
to **task 0093**.

## Acceptance Criteria

- [x] **Phase A (first):** June-2026 gap (`62,642,957 → 63,352,611`) backfilled to
      `price_ohlcv_1m`. **Done 2026-07-15** — but NOT via the bounded §6b gap-fill:
      an already-running `run-full-backfill.sh` (started 2026-07-08, unnoticed) had
      completed its Phase 1 (`combined activation→floor`, `PHASE 1 COMPLETE 07-14
      17:54Z`) which covers the gap. Verified: `1m` contiguous `2024-02-20 → 2026-07-08`,
      532M rows, gap range fully populated for sdex/aquarius/phoenix.
- [x] **Phase B (last):** pre-roll over the completed `1m` into the coarse tables.
      **DONE 2026-07-15** after THREE fixes the original plan missed (see Execution
      findings): (1) deleted the pre-Soroban fragment; (2) chunked the rollup by
      year + dropped `FINAL` on the dup-free intermediate stages to fit the 5.59 GiB
      quota; (3) DROPped the replace-mode rollup MVs that were wiping the coarse
      tables. Verified: all six coarse tables populated (15m 151.7M → 1M 1.3M rows,
      monotonic-decreasing), `1h`/`1d` hold sdex+aquarius+phoenix `2024 → 2026-07-09`.
      NOTE: `soroswap` absent — separate backfill-coverage gap, task **0096** (not
      a pre-roll issue). Owner sign-off held.
- [ ] `docs/runbooks/fix-backfill-history-loss-and-rerun.md` re-scoped (superseding
      banner: pre-roll-first / gap-fill, no blanket TRUNCATE+re-download). ✅ done 2026-07-14.
- [x] `docs/runbooks/continue-soroban-backfill.md` gains the pre-roll + cleanup-
      coordination steps (currently stops at writing `1m`). ✅ 2026-07-14 — added
      durability ⚠ callout + §9 pre-roll (`preroll.sql` via Route-A, spill flags,
      owner sign-off) + §10 cleanup-coordination; renumbered stop/re-run → §11.
- [x] Cleanup re-enable procedure documented; re-enabled only AFTER pre-roll.
      **Documented** ✅ (§10: `describe-rule` → pre-roll → `enable-rule` order).
      **Executed** ✅ 2026-07-15 — `prices-production-cleanup` re-ENABLED after the
      coarse tables were verified populated.
- [x] Deep pre-Soroban tail (2015→2024) split to **task 0092** (decision w/ BE 0199).
      ✅ `0092_FEATURE_pre-soroban-tail-backfill-decision` (backlog, related to 0090).
      **DECISION MADE 2026-07-15: BE needs Soroban-era ledgers only — pre-2024 NOT
      needed.** The full-backfill's deep tail (Phase 2, `sdex-only 1→50457423`) was
      KILLED and the ~28-row 2015-2016 fragment it wrote was deleted from `1m`.
      → 0092 resolves as "not needed" (archive).
- [x] Backfill watchdog + live candle-freshness alarm split to **task 0093**.
      ✅ `0093_FEATURE_freshness-alarms-backfill-and-live` (backlog, related to 0090).

## Execution findings (2026-07-15 — the actual run)

Executing Phase A/B surfaced things the plan didn't anticipate. In order:

1. **The backfill was never actually dead — a full re-download had been running
   since 2026-07-08.** `pgrep` on the run box found `run-full-backfill.sh` (the
   old full-range §6.1 script, NOT the bounded §6b gap-fill) alive in tmux,
   already past Phase 1 (gap included) and grinding Phase 2 (the deep tail).
   So the bounded gap-fill was moot; Phase A was already satisfied.

2. **BE needs Soroban-era data only → deep tail killed.** Confirmed with the
   owner: the BE consumer (0199) does not need pre-2024 history. The running
   Phase 2 (2015→2024, ~2 more weeks on the shared cluster) was therefore
   pure waste — stopped it. Resolves 0092 (see AC).

3. **Trimmed the pre-Soroban fragment.** Phase 2 had written 28 candles
   (2015-11-18 → 2016-02-17) before the kill. `preroll.sql` is full-range, so
   left in place they'd roll up into the coarse tables and make `min(timestamp)`
   read a misleading 2015. `ALTER TABLE prices.price_ohlcv_1m DELETE WHERE
   toUInt64(version) DIV 1000 < 50457424` → clean Soroban-only surface.

4. **`preroll.sql` OOMs the 5.59 GiB `prices` memory quota — must be chunked.**
   The full-range `1m→15m` (473M rows, `FINAL`) exceeds the per-query quota;
   `do_not_merge_across_partitions_select_final=1` + external group-by spill were
   not enough. Fix: run each **sub-day** rollup (`15m,1h,4h,1d`) **chunked by
   year** (year-aligned, so no bucket is split); keep `1w`/`1M` full-range (a WEEK
   can straddle a year boundary so it must NOT be year-chunked, and its input is
   tiny). Script: `scratchpad/chunked_preroll_v2.sh`. **This belongs in the
   runbook** (see Future Work).

5. **★ ROOT CAUSE the task under-diagnosed: the rollup MVs are REPLACE-mode and
   were WIPING the coarse tables.** `rollups.sql`'s six `mv_ohlcv_*` are
   `REFRESH EVERY <n> TO price_ohlcv_<coarse>` with **no `APPEND`** and a
   `WHERE timestamp >= now() - <window>`. Refreshable MVs without `APPEND`
   *atomically replace* their target each refresh. With live frozen at 2026-07-08
   every window is empty, so the MVs were **emptying the coarse tables every
   minute** — the real reason they've been empty (deeper than "pre-roll never
   run"), and they wiped our first successful pre-roll within a minute. Fix
   (Layer 1, this task): **DROP the six `mv_ohlcv_*`** so nothing wipes the
   pre-roll; leave `mv_current_prices` (unrelated). Durable fix (Layer 2, follow-up):
   recreate them as `REFRESH … APPEND …` so live adds onto history without wiping
   → spawned task, coupled to the live-freeze fix (the MVs read live `1m`). The
   original "MVs deliberately ignore historical rows" note was right about the
   `WHERE` window but MISSED that replace-mode actively deletes history.

## Risks / Notes

- **Disk**: holding the entire `1m` history (2015→now) on the shared ch-prod-01
  defeats the point of the 7d retention temporarily; must confirm headroom before
  disabling cleanup, or use per-chunk preroll.
- **Shared cluster**: cleanup rule + any prod schema/DML changes affect the shared
  BE ClickHouse — coordinate + get owner sign-off (see [[flag-container-restarts]],
  [[feedback-prepare-not-deploy]]).
- **Idempotency**: `preroll.sql` and the backfill are ReplacingMergeTree-idempotent;
  re-runs collapse duplicates. Safe to re-run.

## Future Work

Spawned as tracked tasks (not left as prose):

- **Layer 2 — rollup MVs → `APPEND`** (new task): recreate the six `mv_ohlcv_*`
  as `REFRESH … APPEND …` so live rollups add onto the pre-rolled history instead
  of replacing it. Code change to `rollups.sql` + prod re-create. Coupled to the
  live-freeze fix ([[proto27-xdr26-live-freeze]] / 0064 / 0094) since the MVs read
  live `1m` — no point until live writes fresh candles. **Until then the coarse
  tables are pre-roll-only and static.**
- **Runbook update**: fold the real Phase-B procedure into
  `docs/runbooks/fix-backfill-history-loss-and-rerun.md` — the chunked pre-roll
  (`chunked_preroll_v2.sh` shape: year-chunk sub-day, full-range week/month) for
  the memory quota, and the "DROP replace-mode MVs before pre-rolling" step. The
  current §7 (single full-range `preroll.sql`) OOMs and gets silently wiped.

## Investigation artifact

`decode_probe.rs` (proves the extractor produces candles for pre-floor ledgers)
was **relocated to the task-0091 PR**, migrated to stellar-xdr 27. Its
`stellar_xdr` import paths depend on the crate version, so it can only compile on
the xdr-27 branch — keeping it here (xdr 26) would break once 0091 lands. Moved to
`.trash/` on this branch; the live, migrated copy lives with 0091.
