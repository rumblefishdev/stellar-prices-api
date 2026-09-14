---
id: "0101"
title: "Reprice the live-era AMM gap (Phoenix ~2% short + Soroswap 2026-07-06→07-11 hole)"
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
  - "notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md"
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
  - date: 2026-09-14
    status: active
    who: okarcz
    note: >
      The falsification is DONE and the premise survives, narrowed. Measured on
      prod as dev_read; full working at
      notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md. (1) Not
      a trading lull - Soroswap swaps exist in default.soroban_events on every
      dark day (268-3,919/day) while candles are zero. (2) ONE mechanism, the
      0096 topic[0] bug; the pool-registry preload gap is not implicated. (3) The
      07-11 vs 07-15 contradiction dissolves - neither is a bug boundary.
      07-15 15:57Z is the extractor-fix deploy; 07-11 21:00 is where the
      post-proto27 catch-up replay was standing at that moment, so every ledger
      after it was repriced by the fixed binary in flight. Dark range is
      therefore 07-06 09:35 -> 07-11 21:00, ~5.4 days not 9, and the Soroswap
      refill shrinks to [63352612, 63434025] snapped to a minute edge. Two
      method corrections recorded: intDiv(version,1000) is only a ledger in
      single-member buckets (the coarse rollups sum version), and
      prices.unresolved_pools holds backfill rows only, so it cannot witness a
      live drop. Still NOT started on the reprice itself.
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
| **Soroswap ZERO** | `[63352612, 63434025]` = 2026-07-06 09:35 → **2026-07-11 21:00** | Live ran with the 0096 `topic[0]` bug and emitted **no** soroswap candles. It stops at 07-11 not because anything was fixed then, but because that is where the post-proto27 catch-up replay stood when the 07-15 fix deployed — from that ledger on, the replay repriced with the fixed binary. |

So Soroswap history is complete up to 07-06 (0097) and from 07-11 21:00 (the
replay), with **~5.4 days missing between**. Do not describe Soroswap history as
continuous until this lands.

> ✅ **Measured 2026-09-14**, not assumed — see
> [notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md](notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md).
> The range was **07-06 → 07-15 (~9 days)** in every earlier revision of this
> file. It is shorter. Repricing past `63434025` rewrites rows the fixed
> extractor already wrote correctly.

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

## Settled 2026-09-14 — was "Settle this before any run" (absorbed from 0271)

✅ **Done. The premise survives, narrowed.** Full working:
[notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md](notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md).
[[0271]]'s original measurement is preserved at
[notes/R-soroswap-five-day-gap-measured-on-prod.md](notes/R-soroswap-five-day-gap-measured-on-prod.md).

| boundary | ledger |
| --- | --- |
| Soroswap's last candle before the gap | **63,352,574** |
| documented backfill handoff floor | **63,352,611** |
| Soroswap's first candle after the gap | **63,434,026** (2026-07-11 21:00) |

**Three answers:**

1. **Not a trading lull.** Soroswap swaps exist in `default.soroban_events` on
   every dark day — 387 / 336 / 3,919 / 268 on 07-07 → 07-10 — against **zero**
   candles. Input present, output absent: ingestion dropped them.
2. **One mechanism, the 0096 `topic[0]` bug.**
   [[amm-live-pool-registry-preload-gap]] is **not** implicated and no live-path
   fix is owed before refilling.
3. **07-11 vs 07-15 dissolves — neither is a bug boundary.** The envelope is
   `[String("SoroswapPair"), Symbol("swap")]` on every single day 07-04 → 07-16,
   so the broken extractor would have dropped 07-11 → 07-14 exactly as it
   dropped 07-07 → 07-10. And no deploy happened between 07-08 15:48 and
   07-14 17:06. What actually happened: live froze on proto27 at ledger
   ~63,384,067, the xdr-27 build deployed 07-14 ~21:00 and began a catch-up
   replay, and the 0096 fix deployed **07-15 15:57Z** — at which moment the
   replay was standing at ledger ~63,434,000. Every ledger after that went
   through the fixed extractor. **63,433,850 is a cursor position, not a second
   bug.** The arithmetic checks: 63,384,068 → ~63,434,000 in ~19 h is
   ~2,630 ledgers/hour, ≈3.6× real time — a catch-up, not live tailing.

**Why Phoenix and Aquarius were unaffected:** their extractors read the action
from the slot their venues use. Soroswap is the only venue putting a `String`
constant in `topic[0]`, which is also why its events carry a `NULL` `signature`
([[soroban-events-gotchas]] #3). Treat it as a permanent special case.

⚠️ **Do not investigate any of this in `price_ohlcv_1m`.** Non-SDEX rows only
start 2026-07-01 there — AMM history lives in `_1d`/`_1h`. A `_1m` query
returning zero proves nothing. See [[amm-history-is-not-in-price-ohlcv-1m]].

### ⚠️ Correction — `intDiv(version, 1000)` is NOT a general method

Earlier revisions of this file called it "a method worth reusing". It holds in
`price_ohlcv_1m`, where `version = ledger_seq * 1000 + op_index`. It does **not**
hold in the coarse tables: the rollup MVs aggregate with `sum(version)`
([[rollup-mvs-replace-mode-wipe]]), so a multi-minute bucket carries a sum, not a
ledger. Visible in the data — `aquarius` on 2026-07-10 reports a "ledger" of
**380,487,849**, which has never existed. It gave a true answer for the July
Soroswap boundaries only because those buckets have one contributing candle each.
**Sanity-check the magnitude against real ledger height before believing it.**

### ⚠️ `prices.unresolved_pools` cannot witness a live drop

Every row has `source = 'backfill'` (138 pools, 7,887 swaps, `last_ledger`
topping out at 63,352,576 — the handoff floor). The live path never writes there.
The table that exists to record "a swap we could not classify" is blind to the
process that produced this gap.

### Sweep for other instances

The hole was found by accident in one five-week window. Sweep the full AMM range
and say whether it has happened before. ⚠️ Do **not** sweep with
`intDiv(version, 1000)` per the correction above — sweep on **candle absence per
source per day** in `price_ohlcv_1d`, then confirm each candidate against raw
swap counts in `default.soroban_events`, which is the shape that actually settled
this one.

## Acceptance Criteria

- [ ] Phoenix candles in `[63352612, end]` reflect the variable-length fix
      (compare against `soroban_events`: 8-event **and** 7-event groups both
      priced), in `1m` **and** coarse.
- [ ] Soroswap candles exist for `[63352612, 63434025]` (2026-07-06 09:35 →
      07-11 21:00, minute-snapped) in `1m` and coarse; Soroswap history is
      continuous from activation to the live tip.
      ⚠️ Range **corrected 2026-09-14** from 07-06 → 07-15; do not reprice past
      `63434025`, those rows are already right.
- [ ] Conservation holds per source at every granularity; no level below `1m`.
- [ ] SDEX untouched (row count + `1d` tip unchanged — **capture the baseline
      BEFORE the run this time**; 0097 skipped it and could only sanity-check).
- [ ] `prices-production-cleanup` re-enabled after verification.
- [ ] Both run bounds minute-aligned (not just `--end`) — see
      §Cross-invocation minute boundary.

Merged from [[0271]]:

- [x] **The 07-11 vs 07-15 contradiction is resolved and the true Soroswap range
      is stated.** Neither date is a bug boundary. 07-15 15:57Z is the 0096
      extractor-fix deploy; 07-11 21:00 is where the post-proto27 catch-up replay
      was standing at that moment, so it repriced everything after it with the
      fixed binary. True range: **07-06 09:35 → 07-11 21:00**, `[63352612,
      63434025]`, ~5.4 days. See §Settled 2026-09-14.
- [x] **Trading-lull vs dropped-ingestion is settled from raw swap events.** It
      is **dropped ingestion.** Soroswap swaps exist in
      `default.soroban_events` on every dark day — 387 / 336 / 3,919 / 268 on
      07-07 → 07-10 — against zero candles.
- [x] **The mechanism is identified, and it was already fixed.** The 0096
      `topic[0]` bug, fixed and deployed 2026-07-15 15:57Z (PR #112, `2c53ee4`);
      nothing further is owed on the live path, and there is no
      recurrence-at-next-restart hazard.
      Phoenix and Aquarius were unaffected because their extractors read the
      action from the slot their venues actually use — Soroswap is the only venue
      with a `String` constant in `topic[0]`, which is also why its events carry
      a `NULL` `signature`.
- [ ] A sweep over the full AMM range reports whether other instances exist.
      ⚠️ Sweep on **candle absence per source per day** in `price_ohlcv_1d`, then
      confirm each candidate against raw swap counts — **not** with
      `intDiv(version, 1000)`, which is only a ledger in single-member buckets.
- [ ] `milestone-2-evidence.md` §8's row is updated to the outcome — it currently
      reads "cause under investigation" and promises resolution before
      Tranche 3. **The outcome now exists**, so this is writing it up, not
      investigating.

## Notes

- Milestone 2 by explicit decision (2026-07-17): not required for M1, and the
  data is only ~2% off for Phoenix plus a Soroswap window since measured at
  **~5.4 days** (2026-09-14), not the 9 assumed when this was written. Don't let
  it pull focus from M1.
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
