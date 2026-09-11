---
id: "0101"
title: "Reprice the live-era AMM gap (Phoenix ~2% short + Soroswap 2026-07-06→07-15 hole)"
type: FEATURE
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0271", "0099", "0097", "0096", "0065", "0108", "0117", "0127", "0128", "0264", "0176", "0088"]
tags: [layer-indexing, priority-medium, effort-medium, milestone-M2, amm, phoenix, soroswap, backfill, clickhouse]
milestone: 2
links:
  - "../../../../docs/runbooks/events-sourced-amm-reprice.md"
  - "../../../../packages/prices-clickhouse/schema/preroll-amm-reprice.sql"
  - "notes/R-soroswap-five-day-gap-measured-on-prod.md"
history:
  - date: 2026-07-17
    status: backlog
    who: okarcz
    note: >
      Spawned from 0099 so the live-era gap is not lost. 0099 delivered the
      DEPLOY (Phoenix fix live 2026-07-17 11:57:52, so live is correct going
      FORWARD); this task is the backward-looking half — repricing what the buggy
      live processor wrote. Deliberately MILESTONE 2: not needed for M1, no
      urgency, do not pull focus. Both gaps are bounded, well-understood, and the
      tooling + runbook already exist from 0097.
  - date: 2026-07-20
    status: backlog
    who: okarcz
    note: >
      Absorbed the surviving residual of task 0065 (cross-chunk intra-minute
      candles) during the 0108 grooming sweep. 0065's main body is fixed and
      archived — the accumulator now keeps the boundary minute open across
      chunks — but the fix is per-process, so the cross-INVOCATION case lands
      here, on the task that actually chooses run boundaries. See
      §Cross-invocation minute boundary.
  - date: 2026-09-11
    status: active
    who: okarcz
    note: >
      Promoted to active and assigned. [[0271]] folded in and archived as
      superseded: it measured the same Soroswap darkness on prod from the other
      end, without knowing this task already carries a diagnosed cause. Its
      measurement is preserved verbatim at
      notes/R-soroswap-five-day-gap-measured-on-prod.md and its acceptance
      criteria are merged below. Converted file -> directory at the same time
      (the note pushed it past the ~150-line guidance). NOT started - the first
      move is the falsification in §Settle this before any run, not a reprice.
---

# Reprice the live-era AMM gap

## Summary

Two known holes in AMM history, both created by extractor bugs that ran in
**live** ingestion and are now fixed but only take effect **forward** from their
deploys. Task 0097 repriced everything up to the SDEX live floor
(`63352611`, 2026-07-06 09:35:16); this task covers the range **after** it.

| gap | range | what's wrong |
|---|---|---|
| **Phoenix ~2.1%** | `[63352612, deploy_ledger]` (deploy = 2026-07-17 11:57:52) | Live wrote candles with the `n >= 8` gate, silently dropping every 7-event swap. |
| **Soroswap ZERO** | 2026-07-06 → **2026-07-15** | Live ran with the 0096 `topic[0]` bug until the fix deployed on 07-15 — it emitted **no** soroswap candles at all. |

So Soroswap history is complete up to 07-06 (0097) and from 07-15 (live), with
**~9 days missing between**. Do not describe Soroswap history as continuous until
this lands.

## Context

- **0096** — soroswap extractor read the swap action from `topic[0]`; the real
  envelope is `[String("SoroswapPair"), Symbol("swap")]` (action in `topic[1]`).
  Fixed + deployed 2026-07-15.
- **0097** — CH-to-CH reprice for the historical range. Archived; verified.
- **0099** — Phoenix variable-length swap groups: `dispatch_phoenix` gated on
  `n >= 8` (the fully-populated shape) while Phoenix omits optional fields;
  5,175 real 7-event swaps (~2.1%) were discarded. Fixed + deployed
  2026-07-17 11:57:52.

The tool (`events-backfill`), the runbook
(`docs/runbooks/events-sourced-amm-reprice.md`) and the scoped pre-roll
(`schema/preroll-amm-reprice.sql`) all exist and are prod-proven. This is an
operational re-run, not new engineering.

## Implementation

Same sequence as 0097 §1–4 — but note the differences below, which are the whole
reason this isn't a trivial repeat:

1. **Pick `--end` deliberately.** Live is actively writing. The end ledger must
   sit safely behind the live frontier **and be minute-aligned**, or the
   boundary minute is contested between this reprice and live: RMT keeps
   `max(version)`, live's ledgers are higher, so our partial loses. Observed in
   0097 exactly this way at `09:35`. `--start` = `63352612`.
2. **Disable `prices-production-cleanup` first.** `price_ohlcv_1m` is a 7-day
   transient and the 07-06→~07-10 partitions are **already gone** — this reprice
   rewrites historical `1m`, so cleanup must stay off until the pre-roll
   verifies. Re-enable after. (The 0090 incident is exactly this.)
3. **Phoenix needs DELETE-first in `1m` AND coarse.** This is the sharp edge: the
   recovered 7-event swaps sit **mid-bucket**, so they raise volume/trade_count
   **without** raising the bucket's `max(ledger*1000 + op_index)`. The corrected
   row therefore **ties** the stale one on `version`, and RMT's tie-break is not
   contractual — the fix can silently fail to land while the data looks fine.
   0097 solved this in coarse with a scoped `ALTER TABLE … DELETE … SETTINGS
   mutations_sync = 2`; here it applies to `1m` too, since live already wrote
   those minutes. Soroswap needs no delete (no rows to contest).
4. **Pre-roll** with `preroll-amm-reprice.sql`, params adjusted to this window.
   Keep **FINAL** (the target levels are not TRUNCATEd → non-FINAL
   double-counts) and keep the **month-chunking** (a year-bounded FINAL exceeds
   the 5.59 GiB quota; see the script header).
5. **Verify** per script §5 — per-source conservation, one granularity at a time
   (a multi-way `UNION ALL` of FINAL scans runs concurrently and blows the quota).

## Cross-invocation minute boundary (absorbed from 0065)

The `CandleAccumulator` keeps a boundary minute open **within one process run**
(`events-backfill/src/run.rs:198`, flushing via `flush_older_than`; regression
test `minute_split_across_calls_is_summed_once_not_undercounted`, `run.rs:544`).
That guard does **not** span separate invocations: two runs whose ranges split a
minute each emit a partial candle for it, and because `price_ohlcv_1m` is
`ReplacingMergeTree(version)` the higher-version partial simply **replaces** the
other — a silent undercount, never a duplicate.

This is exactly the hazard §1 already guards for `--end`, so treat it as one
rule rather than two: **every run boundary must be minute-aligned**, at the
start as well as the end. `--start = 63352612` inherits its alignment from
0097's end, so it is safe as written; re-verify if either bound moves.

## Settle this before any run (absorbed from 0271)

[[0271]] measured the Soroswap hole on production on 2026-09-08, from
`price_ohlcv_1d`, the forever table. Full measurement:
[notes/R-soroswap-five-day-gap-measured-on-prod.md](notes/R-soroswap-five-day-gap-measured-on-prod.md).

| boundary | ledger |
| --- | --- |
| Soroswap's last candle before the gap | **63,352,574** |
| documented backfill handoff floor | **63,352,611** |
| Soroswap's first candle after the gap | **63,433,850** |

🔴 **One fact does not fit this task's story.** 0271 has Soroswap resuming on
**2026-07-11** (10 candles that day, ledger 63,433,850); this task says the 0096
`topic[0]` bug suppressed every Soroswap candle until the fix deployed
**2026-07-15**. Both cannot be true. Settle it first — it decides whether the
range to refill is 07-06→07-11 or 07-06→07-15, and whether one mechanism is at
work or two:

```sql
SELECT toDate(timestamp) d, count() FROM prices.price_ohlcv_1d
WHERE source = 'soroswap' AND timestamp >= '2026-07-05' AND timestamp < '2026-07-20'
GROUP BY d ORDER BY d
```

Then falsify the trading lull from **raw swap events**, not candles: Soroswap is
low-volume enough that a five-day quiet spell is not absurd on its own. If swaps
exist in `soroban_events` for the range and candles do not, ingestion dropped
them. If no swaps exist, the venue was genuinely quiet and half this task
evaporates.

⚠️ **Do not investigate any of this in `price_ohlcv_1m`.** Non-SDEX rows only
start 2026-07-01 there — AMM history lives in `_1d`/`_1h`. A `_1m` query
returning zero proves nothing. See [[amm-history-is-not-in-price-ohlcv-1m]].

🔑 **Method worth reusing:** candles carry no ledger column, but
`version = ledger_seq * 1000 + op_index`, so `intDiv(version, 1000)` locates any
candle in ledger space. Comparing that against the backfill's documented floor is
what turned a curiosity into the 37-ledger alignment above.

### Competing causes, both on the table

| cause | predicts |
| --- | --- |
| **0096 `topic[0]` bug** (this task's premise) | Soroswap dark from the handoff until the 07-15 deploy, exactly |
| [[amm-live-pool-registry-preload-gap]] | one venue dark across a restart, others unaffected, no error anywhere — the live processor builds `Registries::new()` empty and drops unregistered-pool swaps silently |

Phoenix and Aquarius ran normally throughout, which fits either. Name the
mechanism before refilling — a refill without the fix recurs at the next restart.

### Sweep for other instances

The five-day hole was found by accident in one five-week window. Run the same
`intDiv(version, 1000)` shape over the full AMM range and say whether it has
happened before.

## Acceptance Criteria

- [ ] Phoenix candles in `[63352612, end]` reflect the variable-length fix
      (compare against `soroban_events`: 8-event **and** 7-event groups both
      priced), in `1m` **and** coarse.
- [ ] Soroswap candles exist for 2026-07-06 → 07-15 in `1m` and coarse; Soroswap
      history is continuous from activation to the live tip.
- [ ] Conservation holds per source at every granularity; no level below `1m`.
- [ ] SDEX untouched (row count + `1d` tip unchanged — **capture the baseline
      BEFORE the run this time**; 0097 skipped it and could only sanity-check).
- [ ] `prices-production-cleanup` re-enabled after verification.
- [ ] Both run bounds minute-aligned (not just `--end`) — see
      §Cross-invocation minute boundary.

Merged from [[0271]]:

- [ ] The 07-11 vs 07-15 contradiction is resolved and the true Soroswap range
      is stated — see §Settle this before any run.
- [ ] Trading-lull vs dropped-ingestion is settled from raw swap events, said
      plainly either way.
- [ ] If ingestion: the mechanism is identified and fixed, and it is stated why
      Phoenix and Aquarius were unaffected.
- [ ] A sweep over the full AMM range reports whether other instances exist.
- [ ] `milestone-2-evidence.md` §8's row is updated to the outcome — it currently
      reads "cause under investigation" and promises resolution before
      Tranche 3.

## Notes

- Milestone 2 by explicit decision (2026-07-17): not required for M1, and the
  data is only ~2% off for Phoenix plus a 9-day Soroswap window. Don't let it
  pull focus from M1.
- Everything learned in 0097 — RMT version ties, FINAL-is-mandatory,
  month-chunking, the readonly=1 `SETTINGS` trap, minute-alignment — is captured
  in the pre-roll script header and the runbook. Read those first.
- ⚠️ **The repair is live-owned range**, so it carries the
  [[backfill-live-no-code-coordination]] overlap hazard. 0271 raised it against
  the sdex/combined backfill CLI, which has no per-venue filter (`--mode` is
  `combined` or `sdex-only`) — a Soroswap refill there would rewrite Phoenix and
  Aquarius rows live already wrote correctly. **This task's path is the
  events-sourced CH-to-CH reprice instead**, which rewrites only AMM sources in
  the ledger range, and rewriting Phoenix is the point here. The surviving edge
  is minute-alignment at **both** bounds, per §Cross-invocation minute boundary.
