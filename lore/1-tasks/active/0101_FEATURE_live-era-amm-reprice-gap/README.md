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
  - "notes/R-pre-run-baseline-2026-09-14.md"
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
      refill shrinks to 07-06 09:35 -> 07-11 21:00, ledger 63,433,849. Two
      method corrections recorded: intDiv(version,1000) is only a ledger in
      single-member buckets (the coarse rollups sum version), and
      prices.unresolved_pools holds backfill rows only, so it cannot witness a
      live drop. Still NOT started on the reprice itself.
  - date: 2026-09-14
    status: active
    who: okarcz
    note: >
      Run plan written as the §RUN RUNBOOK section, bounds measured against
      default.ledgers. ONE run, [63352609, 63518022], because events-backfill has
      no venue filter and Phoenix's buggy window contains Soroswap's. Re-measured
      three July-era premises in §Implementation and TWO ARE FALSE. (a) --start
      63352612 is NOT minute-aligned - 0097 ended at 09:35:16, mid-minute, so the
      documented start splits minute 09:35 across two invocations; corrected to
      63352609. (b) The 1m partitions are NOT gone - cleanup has been off since
      ~07-20 and 1m holds rows back to 2015-11-18, with the whole July AMM window
      present (51,116 aquarius / 1,620 soroswap / 1,025 phoenix). That inverts
      the risk: re-enabling cleanup AFTER the run is now the destructive step,
      not a precondition. (c) "Soroswap needs no delete" is false - it has two
      rows inside the dark window and the 21:00 one is a PARTIAL, since the
      replay entered that minute mid-way. Also corrected the resumption ledger
      from 63,434,026 to 63,433,850: the first figure came from min() over _1d
      where version is a SUM, which is the exact trap documented the same day.
      Live contention is gone (tip is two months past the window). Flagged for
      settlement before any delete: the 0267/0268 USDC corrections akot rolled
      out 09-10/11 cover this same July range.
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
| **Soroswap ZERO** | 2026-07-06 09:35 → **2026-07-11 21:00**, last dark ledger `63433849` | Live ran with the 0096 `topic[0]` bug and emitted **no** soroswap candles. It stops at 07-11 not because anything was fixed then, but because that is where the post-proto27 catch-up replay stood when the 07-15 fix deployed — from that ledger on, the replay repriced with the fixed binary. |

So Soroswap history is complete up to 07-06 (0097) and from 07-11 21:00 (the
replay), with **~5.4 days missing between**. Do not describe Soroswap history as
continuous until this lands.

> ✅ **Measured 2026-09-14**, not assumed — see
> [notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md](notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md).
> The range was **07-06 → 07-15 (~9 days)** in every earlier revision of this
> file. It is shorter. Repricing past the resumption only rewrites rows the
> fixed extractor already wrote correctly — harmless, but not the point of the
> run. See §📕 RUN RUNBOOK for the bounds actually used.

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

Same sequence as 0097 §1–4. ⚠️ **Three of the four premises below were written in
July and were re-measured on 2026-09-14. Two of them are now false.** Corrections
inline; the executable version is §📕 RUN RUNBOOK.

1. **Pick the bounds deliberately.**
   ⚠️ **The live-contention hazard is GONE.** This was written when the range
   abutted the live frontier. Live is now two months past it (tip 2026-09-14),
   so no minute in this window is contested with live.
   🔴 **But `--start = 63352612` is NOT minute-aligned and never was.** 0097
   ended at `63352611` = **09:35:16** — mid-minute. Starting at `63352612`
   splits minute `09:35` across two invocations, which is precisely the
   undercount in §Cross-invocation minute boundary, and it is why 0097 was
   observed misbehaving "at 09:35". **Start at `63352609`**, the first ledger of
   that minute, so this run owns the whole minute and its complete candle
   carries the highest version.
2. **`prices-production-cleanup` must stay disabled — now by standing decision.**
   🔒 **Operator, 2026-09-14: it stays DISABLED until M3 is complete**, to keep
   historical backfill data in the database. So this is no longer a window this
   task opens and closes; it is a precondition already met and not this task's to
   reverse.
   ⚠️ **It is already disabled, and has been since ~2026-07-20** (the 0215 root
   cause turns on that date). Measured 09-14: `price_ohlcv_1m` holds rows back
   to **2015-11-18**, and the July AMM window is fully present — 51,116
   aquarius, 1,620 soroswap, 1,025 phoenix. So the claim that "the 07-06→07-10
   partitions are already gone" is **false**; nothing has been lost to
   retention.
   🔴 **This inverts the risk.** The danger is no longer that the rows vanished
   before the run — it is that **re-enabling cleanup after the run drops every
   `1m` row older than 7 days**, this window included. Re-enable **only** after
   the pre-roll has landed in coarse *and* verified. That is the 0090 incident,
   and it is now the single most destructive step in this task.
3. **DELETE-first in `1m` AND coarse — for Phoenix *and* Soroswap.**
   The sharp edge is unchanged: recovered swaps sit **mid-bucket**, so they
   raise volume/trade_count **without** raising the bucket's
   `max(ledger*1000 + op_index)`. The corrected row **ties** the stale one on
   `version`, and RMT's tie-break is not contractual — the fix can silently fail
   to land while the data looks fine. Scoped
   `ALTER TABLE … DELETE … SETTINGS mutations_sync = 2`, as 0097 did in coarse.
   🔴 **"Soroswap needs no delete (no rows to contest)" is false.** Soroswap has
   **two** rows inside the dark window — `21:00` (ledger 63,433,850) and `21:06`
   (63,433,917). The `21:00` one is a **partial**: the replay entered that minute
   mid-way, so the minute holds some swaps the buggy binary dropped. It ties on
   version for the same reason Phoenix does. Delete both venues.
   Aquarius needs no delete — it was never miswritten, and a rewrite that
   reproduces identical values ties harmlessly.
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
| Soroswap's first candle after the gap | **63,433,850** (2026-07-11 21:00) |

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

## 📕 RUN RUNBOOK

Planned 2026-09-14, bounds measured against `default.ledgers`. Read
[`docs/runbooks/events-sourced-amm-reprice.md`](../../../../docs/runbooks/events-sourced-amm-reprice.md)
first — this section only records what is *different* for this window.

**Tool: `events-backfill`, the CH-to-CH reprice from 0097.** Not the
sdex/combined backfill CLI — that re-downloads ledgers, has no per-venue filter,
and would touch SDEX.

### One run, not two

`events-backfill` takes only `--start`/`--end` in ledger space; it has **no
venue filter** and reprices every AMM source in the range. Phoenix's buggy window
**contains** Soroswap's, so a single run covers both, and two runs would only add
two more boundary minutes to get wrong.

| bound | ledger | closed_at | why |
| --- | --- | --- | --- |
| `--start` | **`63352609`** | 2026-07-06 09:35:00 | first ledger of the minute 0097 ended inside. ⚠️ **Not** `63352612` — see §Implementation 1 |
| `--end` | **`63518022`** | 2026-07-17 11:59:59 | last ledger of 11:59; the 0099 deploy is at 11:57:52 (ledger `63518000`), and the extra two minutes absorb Lambda containers still serving the old binary |

Range is **165,414 ledgers** — one chunk at the 320k default. Live's tip is
2026-09-14, two months clear, so nothing here is contested with live.

### Sequence

1. **[local repo] Build and ship the binary**, then prove it runs on the box:
   `cargo build --release -p events-backfill`, `scp` it to `~/events-backfill`,
   and run `~/events-backfill --version` over ssh before trusting it.
2. ✅ **[read-only] Baselines CAPTURED 2026-09-14, before any write** —
   [notes/R-pre-run-baseline-2026-09-14.md](notes/R-pre-run-baseline-2026-09-14.md).
   SDEX per level + the July partition, AMM per source at `1m` over the exact
   window and at every coarse level over whole July buckets, with `close_usd`
   coverage. Re-run the identical queries after step 9 and diff.
   🔑 **Aquarius is the control** — its totals must come back unchanged.
   🔑 Soroswap's earliest `1m` row currently reads **2026-07-11 21:00**; after
   the run it must read `2026-07-06 09:3x`. That single cell is the task.
3. ✅ **`prices-production-cleanup` is confirmed DISABLED** — verified
   2026-09-14 from EventBridge (`State: DISABLED`) and CloudTrail (disabled
   2026-07-20 16:22:33, last fire 2026-07-20, zero invocations in 56 days). Step
   satisfied, and durable — `enabled: false` is in the CDK since 0204.
4. **[prod host] Dry-run** `events-backfill --dry-run --verbose` over the bounds,
   under `tmux`. Compare its per-source tick counts against raw swap counts from
   `soroban_events` for the same range (query shape in the notes). ⚠️ Aquarius
   should come back matching what live already wrote; if it does not, the
   extraction path has changed since July and the blast radius is bigger than
   this task — **stop and re-scope**.
5. 🔴 **[local repo, branch + PR] Adapt `preroll-amm-reprice.sql` BEFORE step 8.**
   This is a code change, not a param tweak. Three defects for a mid-month
   window, all of which 0097's window happened to avoid:
   - **Params** are hardcoded to 0097 (`start_ts = '2024-02-20 17:00:10'`,
     `end_ts = '2026-07-06 09:35:16'`).
   - 🔴 **STAGE 1's year chunks only half-respect the params.** The middle chunk
     is hardcoded `>= '2025-01-01' AND < '2026-01-01'` and the third is
     `>= '2026-01-01' AND < {end_ts}`. Pointed at a July-2026 window it re-rolls
     **all of 2025 and the first half of 2026** — rows STAGE 0 never deleted, so
     phoenix version-ties survive there untouched. Replace the chunking with one
     bounded chunk.
   - 🔴 **Bucket alignment.** 0097's window ended on a boundary; this one sits
     **mid-month**. The `1M` bucket is stamped `2026-07-01`, *below* `start_ts`,
     so STAGE 0 will not delete it and STAGE 2 will insert a **partial** July
     monthly bucket beside the existing full one. Same for the `1w` buckets
     straddling both ends. Fix: align the pre-roll window to **whole coarse
     buckets** — rebuild phoenix+soroswap coarse across all of July, from the
     week containing 07-01 to the week containing 07-31. Safe precisely because
     cleanup has been off: `1m` holds all of July, so whole buckets can be
     rebuilt from it.
   - **STAGE 0 scope** widens from phoenix-only to
     `source IN ('phoenix','soroswap')`.
6. **[prod host] DELETE first in `price_ohlcv_1m`**, scoped to the window,
   `SETTINGS mutations_sync = 2`: `phoenix` **and** `soroswap`. Not aquarius.
   (Coarse deletes are STAGE 0 of the pre-roll, step 8.)
7. **[prod host] Run** step 4's command without `--dry-run`.
8. **[read-only] Verify `1m`** per source against the dry-run counts.
9. **[prod host] Pre-roll** with the adapted script. Keep **FINAL** — the targets
   are not TRUNCATEd, so non-FINAL double-counts. ⚠️ If a DELETE errors, **stop**:
   an emptied coarse level with no re-insert is a history hole.
10. **[read-only] Verify conservation** per source, one granularity at a time,
    and SDEX untouched against step 2's baseline.
11. **[read-only, after enrichment has run] Confirm `close_usd` recovers.**
    ⚠️ The reprice writes **`close_usd = 0`** — `OhlcvCandle` has no such field
    and `writer.rs:177` says so outright (*"DEFAULT 0 — the 0026 enrichment
    Lambda fills this"*). So every row this run rewrites loses its enriched USD
    price until the enrichment pass re-prices it, and BE reads `close_usd` only
    ([[be-reads-close-usd-only-not-volume-columns]]).
    ✅ **It recovered after 0097** — measured 2026-09-14 over that range:
    phoenix **100%**, soroswap **95.9%**, aquarius 67.5% priced. Expect the same
    shape here. If July stays at zero, enrichment's frontier does not reach back
    that far, and that is the concrete case for [[0148]].
12. **Do NOT re-enable `prices-production-cleanup`.** 🔒 Operator decision
    2026-09-14: it stays DISABLED until M3 is complete. One fire would drop every
    `1m` row older than 7 days — 793M rows back to 2015, not merely this window.
    ✅ It can **no longer** be re-enabled by accident: `eventbridge-stack.ts`
    declares `enabled: false` since task 0204 (2026-08-20), so CDK and production
    agree and a deploy of an unrelated stack cannot flip it on. (An earlier
    revision of this section said the opposite; that described the pre-0204
    state.)

### Where it runs, and who runs it

⚠️ **On the Hetzner host as ClickHouse's `default` user against
`localhost:8123`** — the tool's single client reads `default.*` (BE's tables) and
writes `prices.*`, and the prices mTLS user cannot read `default.*`. Connect per
[[hetzner-ch-prod-ssh-access]]. Password via `read -rs` into the env, never
`--clickhouse-password` (argv is world-readable via `/proc/<pid>/cmdline`).

This is a **prod write**, so the operator runs every step from 4 onward, and the
`--dry-run` in step 3 too ([[feedback-user-runs-prod-ch-queries]]). The baseline
and verification reads in steps 1, 6 and 8 are `dev_read` and need no hand-off.

### ✅ Settled — the 0267/0268 overlap is NOT a conflict

Raised because [[0276]] rolled the 0267/0268 USDC corrections onto production on
2026-09-10/11 and [[0279]] holds `repair_0268_` snapshots until 2026-09-18, over
the same July range this reprice rewrites. **Answered from the source
2026-09-14: `events-backfill` never writes `close_usd`.** `OhlcvCandle`
(`bucket.rs:8-23`) has no such field, and the writer says so at
`writer.rs:177` — *"DEFAULT 0 — the 0026 enrichment Lambda fills this"*.

So the reprice cannot revert anyone's corrected values to an older formula. What
it does is **zero** `close_usd` on every row it rewrites, after which the
enrichment pass re-prices them using whatever logic is current — which *is* the
0267/0268-corrected logic. The outcome is right; the cost is a visible window of
zeros in between. Tracked as step 11, not as a blocker.
⚠️ Those tasks are **akot's** — report, do not act
([[team-adam-kot-task-ownership]]).

## Acceptance Criteria

- [ ] Phoenix candles in `[63352612, end]` reflect the variable-length fix
      (compare against `soroban_events`: 8-event **and** 7-event groups both
      priced), in `1m` **and** coarse.
- [ ] Soroswap candles exist for 2026-07-06 09:35 → 07-11 21:00 in `1m` and
      coarse; Soroswap history is continuous from activation to the live tip.
      ⚠️ Range **corrected 2026-09-14** from 07-06 → 07-15. ⚠️ The resumption
      minute **21:00 is itself partial** — the replay entered it mid-minute at
      ledger 63,433,850 — so the run must cover it, not stop below it.
- [ ] Conservation holds per source at every granularity; no level below `1m`.
- [ ] SDEX untouched (row count + `1d` tip unchanged — **capture the baseline
      BEFORE the run this time**; 0097 skipped it and could only sanity-check).
- [x] ~~`prices-production-cleanup` re-enabled after verification.~~
      🔒 **WITHDRAWN 2026-09-14 by operator decision** — the rule stays
      **DISABLED until M3 is complete**, to keep historical backfill data in the
      database. This task must not re-enable it, and the run is not blocked by
      it: verified from EventBridge + CloudTrail that it has been disabled since
      **2026-07-20 16:22:33** and has not fired in 56 days, so the July `1m` rows
      this reprice targets are intact. Releasing it is [[0200]]'s call, after M3.
- [ ] Both run bounds minute-aligned (not just `--end`) — see
      §Cross-invocation minute boundary.

Merged from [[0271]]:

- [x] **The 07-11 vs 07-15 contradiction is resolved and the true Soroswap range
      is stated.** Neither date is a bug boundary. 07-15 15:57Z is the 0096
      extractor-fix deploy; 07-11 21:00 is where the post-proto27 catch-up replay
      was standing at that moment, so it repriced everything after it with the
      fixed binary. True range: **07-06 09:35 → 07-11 21:00**, `[63352612,
      last dark ledger 63,433,849, ~5.4 days. See §Settled 2026-09-14.
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
