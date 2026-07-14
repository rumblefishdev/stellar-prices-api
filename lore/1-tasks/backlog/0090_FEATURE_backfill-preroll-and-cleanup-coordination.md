---
id: "0090"
title: "Backfill loses history — wire preroll + cleanup-coordination into the backfill workflow"
type: FEATURE
status: backlog
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

**Phase 1 — Pre-roll the existing 1m (do FIRST; cheap, no download).**
Apply `preroll.sql` to the current `price_ohlcv_1m` → populates the coarse
`1h/4h/1d/1w/1M` tables. Delivers ~2.3 years of durable SDEX history (2024-02 →
2026-05) immediately. Idempotent (optionally `TRUNCATE` coarse first for a clean run).
Runs heavy aggregation on the **shared** ch-prod-01 → owner sign-off + low-traffic
window + spill-to-disk flags. **Do NOT** TRUNCATE `backfill_sdex_ledgers`; **do NOT**
re-run the full chain.

**Phase 2 — Gap-fill the one hole (`62,642,957 → 63,352,611`, ~710k ledgers = the
missing June-2026 range).** Bounded backfill of just this range — all **below** the
proto27 boundary (63.4M), so `stellar-xdr 26` is fine (no 0091 dependency). Pool
registry already seeded in prod → AMM resolves. Then pre-roll that range. ~1h on a
us-east-2 EC2 / ~1 day local.

**Phase 3 — Deep pre-Soroban tail (2015 → 2024-02): a DECISION, not automatic.**
The only expensive piece (~50M ledgers, multi-week). Split to **task 0092** — decide
with the BE consumer (0199) whether pre-2024 history is needed before downloading.

**Cleanup:** already disabled (safe — protects the 1m meanwhile). Re-enable
(`aws events enable-rule --name prices-production-cleanup`) only **after** pre-roll, so
the coarse tables have captured the history before the redundant `1m` partitions drop.

**Monitoring:** the backfill had no watchdog (a host kill became a 6-week gap silently);
the live doorbell-lag alarm is blind to a drains-but-doesn't-write processor. Both split
to **task 0093**.

## Acceptance Criteria

- [ ] **Phase 1:** `preroll.sql` applied to the existing `1m`; coarse `1h`/`1d` hold
      `2024-02 → 2026-05` (verified per-source query). Owner sign-off obtained first.
- [ ] **Phase 2:** June-2026 gap (`62.6M → floor`) backfilled + pre-rolled; the coarse
      hole is closed (verified query).
- [ ] `docs/runbooks/fix-backfill-history-loss-and-rerun.md` re-scoped (superseding
      banner: pre-roll-first / gap-fill, no blanket TRUNCATE+re-download). ✅ done 2026-07-14.
- [ ] `docs/runbooks/continue-soroban-backfill.md` gains the pre-roll + cleanup-
      coordination steps (currently stops at writing `1m`).
- [ ] Cleanup re-enable procedure documented; re-enabled only AFTER pre-roll.
- [ ] Deep pre-Soroban tail (2015→2024) split to **task 0092** (decision w/ BE 0199).
- [ ] Backfill watchdog + live candle-freshness alarm split to **task 0093**.

## Risks / Notes

- **Disk**: holding the entire `1m` history (2015→now) on the shared ch-prod-01
  defeats the point of the 7d retention temporarily; must confirm headroom before
  disabling cleanup, or use per-chunk preroll.
- **Shared cluster**: cleanup rule + any prod schema/DML changes affect the shared
  BE ClickHouse — coordinate + get owner sign-off (see [[flag-container-restarts]],
  [[feedback-prepare-not-deploy]]).
- **Idempotency**: `preroll.sql` and the backfill are ReplacingMergeTree-idempotent;
  re-runs collapse duplicates. Safe to re-run.

## Investigation artifact

`packages/prices-ingest-core/examples/decode_probe.rs` was added to prove the
extractor produces candles for pre-floor ledgers. Keep as a diagnostic or move to
`.trash/` — it is not part of the fix.
