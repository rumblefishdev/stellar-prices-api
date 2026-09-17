---
id: "0282"
title: "Candle writes are replaced instead of summed whenever a minute bucket spans a reconcile run — Aquarius loses ~50% of its trades daily, and SDEX is affected too"
type: BUG
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0080", "0101", "0100", "0097", "0203"]
tags: [priority-high, effort-medium, milestone-M3, amm, aquarius, ingestion, data-correctness, data-loss, clickhouse]
milestone: 3
links:
  - "../../../../packages/prices-ledger-processor/src"
  - "../../../../packages/aquarius-extractor/src"
  - "../../../../packages/events-backfill/src/run.rs"
  - "notes/R-sdex-loss-measurement-design.md"
history:
  - date: 2026-09-14
    status: backlog
    who: okarcz
    note: >
      Found in the 0101 dry run, which was not looking for it. The dry run
      reported 115,268 aquarius ticks over an 11-day July window where
      price_ohlcv_1m holds 90,667 — a 21% shortfall on a venue 0101 treats as an
      untouched bystander. Checking it against raw events showed the tool is
      exactly right (115,268 ticks = 115,268 raw trade events, 0 failed
      dispatch, 0 unresolved) and the DATABASE is short. Checking recent days
      showed it is not historical: live is losing 41-57% of aquarius trades
      daily, today included. Filed separately from 0101 because it is an active
      production defect, not a historical repair, and because it is larger than
      the gap 0101 exists to close.
  - date: 2026-09-14
    status: active
    who: okarcz
    note: >
      Promoted the day it was filed. 0101 moves to blocked by this task: its
      write run would rewrite aquarius rows this defect is still producing, and
      its acceptance criteria use aquarius as an untouched control. First move is
      the pool-set vs sample question, which decides the mechanism.
  - date: 2026-09-15
    status: active
    who: okarcz
    note: >
      Converted to a directory and added
      notes/R-sdex-loss-measurement-design.md — the instrument for the one
      outstanding measurement, which re-derives truth from the ledger archive
      locally instead of racing an RMT merge. 🔑 Corrected the sequencing: that
      measurement does NOT depend on the live fix and should run in PARALLEL
      with it, because it gates the repair decision for 98% of the estate.
      Verified the window is uncontaminated — both backfill_progress streams end
      2026-07-06, before the live era. Also spawned [[0285]] after the same
      sizing work showed pool_registry does not describe what actually trades.
  - date: 2026-09-15
    status: active
    who: okarcz
    note: >
      Re-measured on production as dev_read. The MECHANISM stands and is now
      confirmed against a last-write-wins ceiling (09-13: predicted 56.6%,
      actual 51.9% — just below, as it must be). Two surrounding claims did
      NOT. The loss never grew: every live-processed day measures ~45-55%,
      July included. The July ~10% days were written by an ACCUMULATING path,
      and the break is 2026-07-15/16 — the day 0064's durable cursor replaced
      the /tmp cursor that had been re-walking the same span and accidentally
      masking the defect. Two acceptance criteria were ticked on those wrong
      readings and are un-ticked here. Also recorded: a fourth fix option
      (ledger in the sort key), a cost analysis that removes cost from the
      decision, and that PR #313 already carries a fix awaiting review.
  - date: 2026-09-17
    status: active
    who: okarcz
    note: >
      PR #313 review items closed, NOT merged, NOT deployed. The Protocol 28
      hold is over ([[0277]] closed clean). Merged develop in again (54 behind,
      clean), and added tests/reconcile_drain.rs — the multi-run test the review
      asked for: steady state one ledger per doorbell, a backlog drain at
      budgets 32 and 16, and a minute denser than the budget. Reverting to the
      old flush-everything code fails the first two; disabling the escape hatch
      fails the third. Corrected the false idempotency docstring, the WARN text,
      and a stale maxIterations 16 comment. Workspace 974 pass / 0 fail, CI
      green on b3559af. Decided NOT to add a LedgersHeldBack metric: held-back
      ledgers are not lost, so it has no alarm threshold and does not close the
      observability criterion.
---

# Aquarius live ingestion drops about half of every day's trades

## Summary

`prices.price_ohlcv_1m` records roughly **half** the Aquarius trades that exist
in BE's `soroban_events`. Not in a window, not after an incident — **every day,
continuously, including today**. Published Aquarius volume and trade counts are
therefore understated by ~50%, and every coarse tier inherits it.

**It is not the extractor.** `events-backfill` runs the *same*
`classify_amm_groups → dispatch → amm_trade_to_tick` chain over the *same*
events and recovers **100%** of them, with zero dispatch failures and zero
unresolved pools. So the defect is in what the live path is fed or what it does
with it, not in how a trade is decoded.

## Evidence

All measured 2026-09-14 on production as `dev_read`.

### It is current, and it is about half

`raw` = `default.soroban_events` rows with `signature = 'trade'` from contracts
in `prices.pool_registry` where `venue = 'aquarius'`.
`db` = `sum(trade_count)` from `prices.price_ohlcv_1m FINAL`, `source = 'aquarius'`.

| day | raw | db | lost | % |
| --- | --- | --- | --- | --- |
| 2026-09-09 | 11,685 | 6,134 | 5,551 | **47.5** |
| 2026-09-10 | 10,592 | 5,637 | 4,955 | **46.8** |
| 2026-09-11 | 16,958 | 7,314 | 9,644 | **56.9** |
| 2026-09-12 | 6,694 | 3,944 | 2,750 | **41.1** |
| 2026-09-13 | 9,986 | 5,178 | 4,808 | **48.1** |
| 2026-09-14 | 7,429 | 3,446 | 3,983 | **53.6** |

**63,344 raw against 31,653 stored — 50.0% discarded over six days.**

### It is not new, and not an incident

The same comparison over 0101's July window, ledgers 63352609..63518022:

| day | raw | db | % lost |
| --- | --- | --- | --- |
| 2026-07-06 | 9,744 | 6,126 | 37.1 |
| 2026-07-07 | 15,007 | 11,314 | 24.6 |
| 2026-07-08 | 11,174 | 9,077 | 18.8 |
| 2026-07-09 | 8,420 | 7,439 | 11.7 |
| 2026-07-10 | 10,025 | 8,947 | 10.8 |
| 2026-07-11 | 7,334 | 6,642 | 9.4 |
| 2026-07-12 | 7,246 | 6,473 | 10.7 |
| 2026-07-13 | 9,224 | 8,177 | 11.4 |
| 2026-07-14 | 8,788 | 8,087 | 8.0 |
| 2026-07-15 | 11,172 | 9,749 | 12.7 |
| 2026-07-16 | 12,945 | 6,196 | 52.1 |
| 2026-07-17 | 4,189 | 2,440 | 41.8 |

⛔ **CORRECTED 2026-09-15 — this table does not measure the live path, and the
two conclusions drawn from it were both wrong.** It is short on every day, but
07-06 → 07-15 was written by an accumulating path and only 07-16 onward is live.
It IS concentrated in the proto27 freeze, and it is NOT getting worse. See
§"Why it did NOT grow".

### The same code recovers all of them

`events-backfill --dry-run --start 63352609 --end 63518022`, 2026-09-14:

```
events read:               310715
  aquarius ticks:          115268  (swaps failed dispatch: 0)
   phoenix ticks:          1442    (swaps failed dispatch: 70)
  soroswap ticks:          7786    (swaps failed dispatch: 0)
unresolved pools:          0
swaps dropped (unresolved):0
```

115,268 ticks against **115,268** raw `trade` events in the same range — exact.
The live path stored 90,667 for the same span.


## 🔑 ROOT CAUSE — confirmed 2026-09-14

**Neither a pool set nor a sample. It is per-bucket write contention.**

`price_ohlcv_1m` is `ReplacingMergeTree(version)` keyed on
`(timestamp, asset_id, quote_asset_id, source)` — **not** on the pool. Any
`(minute, pair)` bucket written **more than once** keeps only the write with the
highest `version`; every earlier write is replaced, not summed.

Measured over ledgers > 63,900,000, joining raw events to the stored candles:

| bucket touched by | raw trades | stored | predicted if only the last write survives | retained |
| --- | --- | --- | --- | --- |
| 1 ledger | 14,222 | 14,222 | 14,222 | **100.0%** |
| 2-3 ledgers | 13,837 | 5,800 | 5,796 | **41.9%** (predicted 41.9%) |
| 4+ ledgers | 23,362 | 3,528 | 3,509 | **15.1%** (predicted 15.0%) |

Stored tracks the last-write prediction to within **0.6%** across all three
classes. A bucket confined to one ledger is perfect; everything else loses all
but its final write.

### Why, in the code

`reconcile.rs:125-126` builds a `CandleAccumulator` per source and the loop is
explicitly *"accumulate across the whole contiguous run, flush once at the
end"*, flushing at `:202` / `:214`. **That is correct — within one run.** In
steady state a doorbell-driven run is **one ledger** — not because `batchSize`
forces it (see below), but because the cursor is already at the tip, so each
doorbell finds exactly one new ledger to walk. "Flush once at the end of the
run" therefore means *flush once per ledger*, and a minute bucket spans ~11
ledgers at ~5.5 s each. Every one issues its own write for the same bucket, and
RMT keeps the last.

⚠️ **The run is bounded by `MAX_ITERATIONS = 16`, not by 1** — a run *can* walk
up to 16 contiguous ledgers when the cursor is behind. Steady state is the
one-ledger case, and it is the worst case. Measured over 7 days (10,560
minutes): 10 ledgers/min 24.2%, 11 ledgers 75.7%, 12 ledgers 0.05%, never more.
See §"Why it did NOT grow".

🔑 **This is [[0065]]'s "cross-invocation minute boundary", and it is not a
run-boundary edge case.** [[0101]] carries it as a hazard to respect when
*choosing backfill bounds*: the accumulator keeps the boundary minute open
within a process run, and "that guard does not span separate invocations". True
— and in live steady state each doorbell is its own invocation walking a single
ledger, so the guard effectively never applies. What was filed as an operator
footgun is the dominant live data-loss mechanism.

### ⛔ Why it did NOT grow — CORRECTED 2026-09-15

**The premise was false. There was no growth to explain.** Measured on
production as `dev_read`, the live path has lost ~45-55% on every day it
actually processed, July included.

🔑 **The instrument: compare actual retention to the last-write-wins ceiling.**
Predicted = the share of raw trades landing in each `(minute, contract)`
bucket's **highest** ledger. That is a contract-level *upper* bound, because the
real bucket key is pair-level and more crowded — so a live-written day must land
**below** it. A day landing **above** it cannot have been written one-ledger-at-
a-time at all.

| day | raw | predicted | actual | reading |
| --- | --- | --- | --- | --- |
| 2026-07-06 | 13,899 | 50.8% | 74.0% | **+23 — accumulated** |
| 2026-07-07 | 15,007 | 48.4% | 75.4% | **+27 — accumulated** |
| 2026-07-08 | 11,174 | 56.7% | 81.2% | **+25 — accumulated** |
| 2026-07-10 | 10,025 | 62.9% | 89.2% | **+26 — accumulated** |
| 2026-07-12 | 7,246 | 64.0% | 89.3% | **+25 — accumulated** |
| 2026-07-14 | 8,788 | 66.8% | 92.0% | **+25 — accumulated** |
| 2026-07-15 | 11,172 | 58.8% | 87.3% | **+29 — accumulated** |
| 2026-07-16 | 12,945 | 54.1% | 47.9% | −6 — live |
| 2026-07-17 | 9,394 | 61.1% | 55.1% | −6 — live |
| 2026-07-18 | 6,990 | 68.1% | 63.7% | −4 — live |
| 2026-09-13 | 9,986 | 56.6% | 51.9% | −5 — live |

✅ The instrument reproduces this file's own raw counts exactly — 07-07, 07-08,
07-10, 07-12, 07-14, 07-15, 07-16, 09-12 (6,694) and 09-13 (9,986) all match to
the row. 07-06 and 07-17 differ only because the table above uses whole days and
§Evidence used 0101's partial ledger bounds.

🔑 **What actually changed is [[0064]], deployed 2026-07-15.** Before it the
cursor lived in `/tmp` and reset to `INITIAL_CURSOR = 63,352,611` on every
Lambda recycle, so the reconcile loop *"oscillated floor → ~63,372k → floor
forever"* — re-walking the same span in multi-ledger runs, re-accumulating each
bucket whole and writing it back complete. That is the +23 to +29 signature, and
it stops dead at 07-16, the first full day on the durable ClickHouse cursor.

⚠️ **So the defect did not get worse — fixing the freeze stopped masking it.**
A broken cursor was accidentally repairing this bug as a side effect of its own
failure. [[0064]] was correct and necessary; it simply removed the accident.

⚠️ **A run is NOT always one ledger.** `MAX_ITERATIONS = 16` on production: the
loop walks up to 16 contiguous ledgers from the cursor, stopping at the first
gap. `batchSize: 1` caps *doorbells* per invocation, not ledgers per run. So
"loss is a function of ledgers per run" is right, and widening the run is
mechanically available — it is just not what happened in July.

⛔ **A catch-up burst does NOT reduce loss on its own — measured, not assumed.**
On 2026-09-14 Galexie stalled *delivery* for ~35 min (the network kept closing
10-11 ledgers/min throughout), then a ~380-ledger backlog drained in one burst.
Retention did not move: **39.5% before / 40.7% during / 36.3% after**, and every
minute from 11:00 to 12:29 sat at or below the ceiling. The backlog was in
Galexie's uploads, not our cursor, so the loop never got far enough behind to
walk long runs. ⚠️ Do not treat "it was catching up" as evidence of accumulation
without checking the cursor lag.

### Why aquarius is worst

It is not aquarius-specific. Aquarius has **488 registered pools**, many trading
the same asset pairs, so its `(minute, pair)` buckets are the most crowded and
straddle the most ledgers. Phoenix (19 pools) and Soroswap (221) have the same
defect at lower rates — Phoenix stored 1,306 against 1,442 extracted in the July
window, which this explains and 0099's 7-event gate does not.

### ⛔ RETRACTED — "SDEX IS AFFECTED" was WRONG. SDEX loss is UNMEASURED.

**Retracted 2026-09-14, same day, before any code was written.** The contested
SDEX buckets are **not** partial slices. Captured with every column:

```
sdex 11:14 a=4     q=111 tc=1 vb=0.7684586         vq=0.911451   o=c=1.186077 v=64423870000
sdex 11:14 a=4     q=111 tc=1 vb=0.7684586         vq=0.911451   o=c=1.186077 v=64423870001
sdex 11:14 a=79851 q=3   tc=1 vb=792346985.1833504 vq=0.0000001  o=c=0        v=64423863003
sdex 11:14 a=79851 q=3   tc=1 vb=792346985.1833504 vq=0.0000001  o=c=0        v=64423863004
```

**Byte-identical in every field**, differing only in `version` by one in
`op_index`. That is the **same trade written twice**, not two slices of a
minute. RMT keeps one, `trade_count` and volume come out correct, and **nothing
is lost.** Contrast the genuine aquarius case, where the two rows carry
*different* data (`tc 2 / vol 2,513.67` against `tc 1 / vol 1,000`).

So: **SDEX has no measured loss.** It remains *plausible* on the mechanism —
same loop, same accumulator, same flush — but it is unmeasured, and "contested
bucket count" turned out to be the wrong instrument because it cannot tell a
duplicate from a slice. Distinguish them by **comparing the rows' payloads**,
not by counting them.

### 🔑 A separate, real finding: `operation_index` is not stable across re-processing

`reconcile.rs:6-7` claims re-processing is *"idempotent: ReplacingMergeTree
collapses re-inserts by `version`"*. **It does not.** The pairs above are the
same trade at two different `op_index` values, so `version` differs and RMT
keeps both as distinct rows rather than collapsing them.

Benign today — the duplicates carry identical payloads, so the surviving row is
correct. But the stated idempotency guarantee is false, and it is the guarantee
that makes crash-recovery safe. A re-insert that carries a **higher version and
less data** is exactly the aquarius failure, so this is the same hazard one step
away from firing.

⚠️ Also note `version = ledger_sequence * 1000 + operation_index`
(`bucket.rs:48`) allows only **1000 operations per ledger**. A ledger with more
collides into the next ledger's version space. Not observed, but unbounded by
anything in the code.

### Original (aquarius) evidence, which stands

Caught directly. BE holds no classic-trades table, so there is no external
source of truth for SDEX; instead the **losing writes themselves** were observed
before the background merge destroyed them.

A contested bucket is visible as **more than one physical row** for the same
`(timestamp, asset_id, quote_asset_id, source)` in a non-FINAL read. Sampling
the in-flight minute three times, 20 s apart:

| sample | sdex buckets | contested | |
| --- | --- | --- | --- |
| 1 | 834 | 0 | merges had collapsed them |
| 2 | 940 | 0 | merges had collapsed them |
| 3 | 595 | **23** | **3.87%** |

⚠️ **3.87% is a lower bound, not a rate.** RMT merges collapse duplicate rows
within seconds, so a snapshot sees only what has not yet merged. Two of three
samples saw nothing at all while the defect was certainly occurring.

The captured aquarius example, same mechanism, with both sides still present:

| bucket | trade_count | volume_base | version | ledger |
| --- | --- | --- | --- | --- |
| `11:04`, asset 4 / quote 3 | 2 | 2,513.6719 | 64423761005 | 64,423,761 |
| `11:04`, asset 4 / quote 3 | 1 | 1,000.0000 | 64423766004 | 64,423,766 |

Two writes, five ledgers apart, same minute. RMT keeps the higher version, so
**1 trade / 1,000 survives and 2 trades / 2,513.67 is discarded**. The true
minute is 3 trades / 3,513.67.

⚠️ **Correction to the wording above: it is one write per RUN, not per ledger.**
The accumulator flushes once per reconcile run, and a run covers a few ledgers —
the captured pair is five apart. A bucket is only damaged when it **spans a run
boundary**, which is why single-ledger buckets are always intact. The retention
arithmetic matching last-*ledger* so closely implies most runs are short.

**Quantifying SDEX needs a different instrument**, since the evidence
self-destructs: either tight sampling of the in-flight minute over a long
period (lower bound only), instrumenting the writer to count buckets it writes
more than once, or reconciling against Horizon / the ledger archive.

### What the fix has to do

Make the bucket write **additive rather than replacing**, or ensure exactly one
write per bucket ever happens. The mechanism is already proven in this codebase:
`events-backfill` accumulates the full range before writing one row per bucket
and recovers 100%. Options worth weighing: widen the run so a bucket cannot
straddle it (fragile — it only narrows the window), carry the open minute across
invocations in durable state, or move `1m` to a summing engine so concurrent
partial writes add. The last changes the table contract and needs its own
decision.

### ⏳ PR #313 carries the fix — review items closed, NOT merged, NOT deployed

**State 2026-09-17:** head `cc5c9b8`, `develop` merged in, all four CI checks
green on it. Nothing merged and nothing deployed — both wait on the operator.

What changed since the review:

- ✅ **Escape hatch alarmed** (`a2de661`) — `ForcedPartialFlushes` metric and
  `prices-{env}-ledger-processor-forced-partial-flush` alarm. `maxIterations`
  raised **16 → 32** in `infra/envs/production.json`.
- ✅ **Multi-run test added** (`b3559af`) — `tests/reconcile_drain.rs`
  re-stamps one fixture ledger into a longer chain served from memory, then
  drives `run()` repeatedly. Pins four properties: each complete minute is
  written **exactly once**, with its **whole** trade count; the cursor parks on
  a **minute boundary** after every run; the drain **terminates**, holding back
  only the open minute. Covers steady state (one ledger per doorbell), a
  backlog after an outage (budgets 32 and 16), and a minute denser than the
  budget.
  - Proven to bite: with the old flush-everything code put back, the
    steady-state and backlog tests **fail**; with the escape hatch disabled,
    the dense-minute test **fails**.
  - ⚠️ **They self-skip in CI**, like `reconcile_e2e.rs` — the template is a
    gitignored fixture. They protect local runs only. Running them in CI needs
    either one committed fixture (a few MB) or a ledger-with-trades builder.
- ✅ **Docstring corrected** — it no longer claims re-processing is idempotent
  by `version` collapse (see §"`operation_index` is not stable"). It now states
  the property the design rests on: **the cursor always parks on a minute
  boundary**, so no outage splits a minute.
- ✅ WARN text no longer has source indentation baked in; a comment still
  quoting `maxIterations: 16` now says 32.
- ✅ **Second review, three boundary gaps fixed** (`cc5c9b8`, 2026-09-17):
  a CLI (`run_terminal`) run crossing a minute dropped its last minute; with
  multi-ledger S3 objects a minute straddling the cursor could be written and
  then re-written partial (not reachable at one ledger per object); an object
  with no ledgers fired the forced-partial-flush alarm. Each new end-to-end
  test fails on the previous commit. Workspace 980 pass / 0 fail.
- ⛔ **`LedgersHeldBack` metric — decided NOT to add** (2026-09-17). Held-back
  ledgers are not lost, just written a minute later; nearly every run holds
  back 1-12, so the metric has no alarm threshold. A stopped feed already
  fires `rollup-freshness-1m`. It would also add a `PutMetricData` call to
  nearly every run (~15k/day) for no signal. See the observability criterion
  below for where the real gap is.

### 🔜 Before merging / deploying #313

1. **Merge order with [[0286]]'s PR #320.** Both change
   `packages/prices-ledger-processor/src/reconcile.rs`,
   `packages/prices-ingest-core/src/lib.rs` and `infra/src/lib/types.ts`, and
   #320 lists "0282 deployed" as a precondition. So **#313 merges first and
   #320 rebases onto it** — to be agreed with its owner.
2. **Deploy order: ObservabilityStack FIRST, then ComputeStack.** The alarm
   watches a metric only the new binary emits, and `notBreaching` reads no-data
   as OK, so landing the alarm early is harmless. The reverse leaves a window
   where the escape hatch can fire unseen.
3. **Expect the first run after deploy to write one minute from its tail
   only.** The old code left the cursor mid-minute. Self-heals from the next
   minute — do not read it as the fix failing.
4. **Verify** with the raw-vs-stored comparison over one full day (criterion 2).

### 🗄️ Pre-review notes, 2026-09-15 — kept for the reasoning

`fix/0282_reconcile-flushes-partial-minutes`, "end a reconcile run on a whole
minute" (588+/24-, 6 files), open and mergeable with **no review**. It holds
back any minute the run did not see the END of, rewinds the cursor to the last
ledger of the last complete minute, and re-reads the held-back ledgers next run
— so no accumulator state has to survive between invocations and a cold start
behaves like a warm one.

⚠️ **Three things to check before it ships:**

1. ✅ *(cleared 2026-09-17 — [[0277]] crossed clean and is closed)*
   🔴 **Do not deploy it across the Protocol 28 crossing.** [[0277]] still owes a
   before/after frontier measurement at the 2026-09-16 17:00 UTC activation.
   Changing the reconcile loop in the same window makes an ingestion hiccup
   un-attributable. Sequence them.
2. ⚠️ **The deadlock escape hatch is thin, and its only signal is a WARN.** If a
   minute ever holds ≥ `maxIterations` ledgers the run can never see past it and
   would hold back forever — doorbells consumed successfully, queue age 0, no
   alarm. The PR forces a partial flush and logs WARN. Measured over 7 days
   (10,560 minutes): **10 ledgers 24.2%, 11 ledgers 75.7%, 12 ledgers 0.05%,
   never more than 12** — so the worst walk is 13 against a budget of 16. Not
   fragile (reaching 16 needs a ~3.75 s close time, a deliberate network
   change), but raise `maxIterations` anyway — headroom is a cap, not a cost —
   and alarm on that WARN.
3. ⚠️ **Publishing latency.** A minute's candles land only once a ledger from the
   following minute is fetched, so the newest candle is up to ~60-78 s old
   instead of ~5 s. `rollup-freshness-1m` fires at 900 s, so the alarm is clear
   — but check the API consumers, not just the alarm ([[0246]] is what a
   staleness assumption costs).

### 🔑 A fourth option, not previously listed: put the ledger in the sort key

Keep `ReplacingMergeTree` — keep idempotency — but extend `ORDER BY` with the
ledger so two writes for the same minute no longer collide. Nothing is replaced
because nothing shares a key; re-inserting the same ledger still collapses. The
minute-level total moves to the read/rollup path.

⛔ **Why a plain summing engine is the weakest option.** It is not idempotent,
and this design's crash recovery *depends* on idempotency (`reconcile.rs:6-7` —
the cursor is written last so a crashed run is re-processed). With
`maxReceiveCount: 10` a doorbell can be redelivered ten times, so summing turns
a bounded silent under-count into an unbounded silent **over**-count — and
over-counting is worse, because under-counting is detectable against raw events
and inflated volume is not. Compounding it, `operation_index` is not stable
across re-processing (see above), so nothing would dedupe. Separately, only
`trade_count` and the three `volume_*` columns actually sum; `high`/`low` need
max/min, and **`open`, `close` and `close_usd` are order-dependent** and need
`argMin`/`argMax` state — which means `AggregatingMergeTree`, new column types,
and a rewrite of every reader. `_1m` feeds `mv_ohlcv_1m_to_15m` (and the whole
rollup chain), `mv_current_prices`, and many `FROM price_ohlcv_1m FINAL` sites,
and BE reads `close_usd` — the one column that cannot be added up.

| | hold-back (#313) | ledger in the sort key |
| --- | --- | --- |
| publishing delay | up to ~1 min | none |
| extra S3 fetching | ~6-7x | none |
| idempotent | yes | yes |
| rows in `_1m` | unchanged | ~12x more |
| blast radius | the processor | schema + every `_1m` reader |

### 💰 Cost is NOT a factor — measured, so it can stop being argued

At ~15,480 ledgers/day, one ledger per S3 object, bucket and Lambda both in
`eu-central-1` **same region and account, so transfer is free**:

- **S3 GETs** — a minute of *n* ledgers is re-read `n(n+1)/2` times, ≈ 6x at 11
  ledgers/min. 470k → ~3.06M GETs/month, **+≈$1/month** at $0.0004/1,000.
- **Lambda** — invocation count is unchanged (same doorbells); only duration
  grows. 266 ms avg today; even at 1.5 s that is **+≈$3/month** at 512 MB arm64.
  ⚠️ Not a timeout risk: p99 is 452 ms and max 2.4 s against a **60 s** timeout.
- **ClickHouse writes drop ~10x** (15,480/day → ~1,440/day) and save **nothing**
  — it is a flat-rate self-hosted Hetzner box, not a metered service. Egress
  falls 2-3 GB/month, inside the 100 GB free allowance.

**Net ≈ +$4/month.** The real gain from 10x fewer INSERTs is *capacity*: ~10x
fewer parts to merge on a box shared with BE, where we are 3.3% of a disk BE has
already filled once ([[hetzner-disk-is-shared-and-be-owns-96pct]]). Decide on
latency vs blast radius, not on the bill.

## Superseded leads (kept — they were how it was found)

⛔ All three were **wrong**, and are kept only so they are not re-run.

1. ⛔ **Concentrated-liquidity pools.** FALSIFIED — aquarius `trade` events have
   exactly **one** shape in the window (4 topics: `trade`, token_in, token_out,
   user; 446,108 events, 212 pools). There is no shape split to explain a
   split outcome.
   24 aquarius pools also emit `pool_state`, the CLMM marker, and they account
   for **169,286 of 446,031** recent trades — **37.9%**. That is the right order
   of magnitude but does not reach 41-57%, so it cannot be the whole story.
   [[0080]] is exactly this shape check and should be folded in or done first.
2. ⛔ **Registry coverage at live time.** FALSIFIED as the cause — there are
   **zero** minutes where trades exist and no candle was stored, which is what a
   dropped pool set would produce. (Still worth noting separately: all 488
   aquarius and all 19 phoenix `pool_registry` rows have **empty
   `token0`/`token1`**; only soroswap's 221 are populated. It does not cause
   this, because the tokens are carried in the event topics.) The reprice filters by
   `prices.pool_registry`; live builds its own view. 168 aquarius contracts
   emitted trades in the July window against 488 registered. If live's registry
   is narrower than the backfill's at any moment, its swaps are dropped —
   silently, because they do not reach `unresolved_pools` either (see below).
3. ✅ **Something between the doorbell and dispatch** — this one was right, and
   resolved above. The shared extraction chain
   is exonerated by measurement, so the divergence is upstream of it: which
   events live sees, or which it hands to the reconciler.

⚠️ **Nothing in the system records this.** `prices.unresolved_pools` holds
`source = 'backfill'` rows only — the live path never writes there — so the one
table meant to record "a swap we could not classify" is blind to a defect that
has been discarding half a venue for months. Any fix should close that too.

## Implementation

### 🔑 Sequencing — and the one thing that does NOT have to wait

⛔ **CORRECTED 2026-09-15.** The SDEX measurement was first sequenced behind the
live fix. That was wrong. It measures candles **already written**, nothing is
rewriting them, and it gates the repair decision for **98% of the estate**
(16.6 M of the 17.0 M live-era candles are SDEX). **Start it in parallel.**

| work | depends on | can start |
| --- | --- | --- |
| Live fix (PR #313) | ~~[[0277]]'s Protocol 28 crossing~~ ✅ done; merge before [[0286]]'s #320 | **now** — operator's merge + deploy |
| **SDEX quantification** | **nothing** | **now** — see [notes/R-sdex-loss-measurement-design.md](notes/R-sdex-loss-measurement-design.md) |
| [[0285]] classification | nothing | now |
| Repair scope decision | SDEX number + 0285 classification | after both |
| AMM repair ([[0101]] folded in) | live fix deployed + verified | after the fix |

⚠️ **The repair itself must wait for the fix** — repairing while the leak runs
just re-corrupts. That constraint is real; it simply does not apply to
*measuring*.

- Identify the mechanism before changing anything. The discriminator that works
  is **raw `soroban_events` counts vs stored `trade_count`**, per day and then
  per pool; the per-pool split is what will separate "a class of pool" from
  "a fraction of all pools".
- Establish whether the lost trades are a **pool set** (always the same
  contracts) or a **sample** (a fraction of every pool). Those have different
  causes and different fixes.
- Fix the live path, then decide the historical repair separately — the CH-to-CH
  reprice (`events-backfill`) already demonstrably recovers these rows, so the
  repair is cheap once the leak is closed.
- Re-check Phoenix and Soroswap the same way. Phoenix is also short in the July
  window (1,306 stored against 1,442 extracted) for reasons that are **not**
  0099's 7-event gate — every swap group in that window is fully populated — so
  this defect may not be aquarius-only.

## Acceptance Criteria

- [x] **The mechanism is identified** — per-bucket RMT write contention, and
      the pool-set vs sample question is answered: **neither**. See §ROOT CAUSE.
- [ ] The live path is fixed and verified by the same raw-vs-stored comparison
      running at ~0% loss for a full day.
- [x] ⛔ **The "~10% → ~50% growth" question is answered by DISSOLVING it** —
      re-measured 2026-09-15, there was no growth. Every live-processed day is
      ~45-55%; the July ~10% days were written by an accumulating path, and the
      break is [[0064]]'s durable cursor on 2026-07-15/16. ⚠️ This criterion was
      previously ticked with the OPPOSITE answer. See §"Why it did NOT grow".
- [ ] ⛔ **Whether SDEX is affected is NOT answered** — previously ticked "YES"
      on 23 contested buckets of 595; that reading was retracted the same day
      (the rows are byte-identical duplicates, not slices) and the tick was
      never removed. SDEX remains plausible on the mechanism and **unmeasured**.
      See §RETRACTED.
- [ ] SDEX's loss is **quantified**, with an instrument that does not depend on
      catching duplicates before the merge. 📐 **Design drafted** —
      [notes/R-sdex-loss-measurement-design.md](notes/R-sdex-loss-measurement-design.md):
      re-derive truth from the ledger archive into a LOCAL ClickHouse and diff
      against prod, over archive partitions 993 and 1000 (~13% of the live era).
      Decision rule is recorded there **before** the run. Not blocked by the
      live fix — run it in parallel.
- [ ] A decision is recorded on repairing the historical estate, with a range.
- [ ] Live-path drops become observable — a dropped swap leaves a trace
      somewhere, rather than nothing at all. ◐ **Half covered by #313:** the one
      path that still loses data after the fix — a forced partial flush — is
      alarmed (`ForcedPartialFlushes`). ⏳ **Still silent:** a swap from a pool
      the live path cannot resolve. Only the backfill writes
      `prices.unresolved_pools`, so live leaves no trace at all. Close to
      [[0285]]; fold it in there or spawn it — not decided.
- [ ] Phoenix's parallel shortfall is measured and either folded in or spawned.

## Notes

- Found during [[0101]]'s dry run. 0101's own acceptance criteria assume
  Aquarius is untouched; that assumption is void, and a 0101 write run would
  move aquarius by +27% in its window. 0101 is sequenced behind this decision.
- The reprice tool is **not** implicated and needs no change — its correctness
  is what made this measurable.
- ⏳ **PR #313 is ready, not merged, not deployed** (2026-09-17, CI green on
  `cc5c9b8`). See §"PR #313 carries the fix".
- ⚠️ **This task's TITLE still says "and SDEX is affected too"**, which was
  retracted on 2026-09-14. It is what the board renders, so it is worth
  correcting deliberately rather than silently.

# 📕 DEPLOY RUNBOOK — PR #313

Written 2026-09-17. The operator merges and deploys; the read-only checks can be
run by anyone with `soroban-readonly` and `dev_read`. Generic procedure:
[`docs/runbooks/deploy-ledger-processor.md`](../../../../docs/runbooks/deploy-ledger-processor.md).
This section carries only what is specific to #313.

**Order is fixed: merge → build → diff both → Observability → Compute →
verify.** The alarm watches a metric only the new binary emits and reads
no-data as OK, so landing it first is harmless. The reverse leaves a window in
which the escape hatch can fire unseen.

## Step 0 — [GitHub, or local machine with `gh`] merge #313

```bash
gh pr merge 313 --squash
```

Squash, like #314. Check the squash message carries no AI co-author trailer.
Tell [[0286]]'s owner it is in, so #320 can rebase.

## Step 1 — [local machine, repo root] preflight

```bash
export AWS_PROFILE=soroban-admin
export AWS_REGION=eu-central-1
git checkout develop && git pull --ff-only
git log --oneline -1                  # expect the #313 squash commit
npm run xdr:verify-protocol-gap       # expect: current (pinned 28 | mainnet 28)
```

## Step 2 — [local machine, repo root] build the bootstrap

🔴 Skipping this deploys the OLD binary with a green result.

```bash
cargo lambda build -p prices-ledger-processor --release --arm64 --features lambda
ls -l target/lambda/prices-ledger-processor/bootstrap   # mtime = seconds ago
file target/lambda/prices-ledger-processor/bootstrap    # ELF 64-bit ... ARM aarch64
```

## Step 3 — [local machine, `infra/`] diff BOTH stacks by name

```bash
cd infra
npx nx build infra
npx cdk --app "node dist/bin/production.js" diff Prices-production-Observability --method=template --strict
npx cdk --app "node dist/bin/production.js" diff Prices-production-Compute --method=template --strict
```

**Expected — previewed 2026-09-17 from the PR branch with `soroban-readonly`:**

| stack | expected changes | anything else → |
| --- | --- | --- |
| Observability | `[+]` `LedgerProcessorForcedPartialFlushAlarm`; `[~]` `OverviewDashboard`; `DashboardAlarmCount` 53 → 54; many `[~]` `AlarmDescription` that only turn `?` into `—` / `≥` / `§` / `→` / `×` | **stop** |
| Compute | `[~]` `LedgerProcessorFunction` only: `Code.S3Key` and `Environment.Variables.MAX_ITERATIONS` 16 → 32 | **stop** |

The `?` → `—` edits are cosmetic: an earlier deploy was synthesised from a
shell with a different locale (same as [[0277]]). No other Lambda in the
Compute stack showed a change, so the local `target/lambda/*` for them matched
production at preview time — re-check that it still does.

## Step 4 — [local machine, `infra/`] deploy Observability FIRST

```bash
make deploy-production-observability
```

Then check (read-only): `prices-production-ledger-processor-forced-partial-flush`
exists and is `OK` or `INSUFFICIENT_DATA`.

## Step 5 — [local machine, `infra/`] deploy Compute

```bash
make deploy-production-compute     # NOT make deploy-production (all stacks)
```

## Step 6 — [local machine or read-only] prove the running binary changed

```bash
aws lambda get-function-configuration \
  --function-name prices-production-ledger-processor \
  --query '[LastModified,Architectures[0],CodeSha256,Environment.Variables.MAX_ITERATIONS]' --output text
```

`LastModified` seconds ago, `arm64`, `MAX_ITERATIONS` = `32`. **Record the
`CodeSha256` here.**

## Step 7 — [read-only, first ~30 min] the new loop is behaving

- **Logs** (`/aws/lambda/prices-production-ledger-processor`): most runs log
  *"run is entirely inside one open minute — holding back, cursor unmoved"*;
  about one run a minute logs *"reconcile run complete"* with `held_back` ≥ 0.
  No *"iteration budget exhausted"* WARN.
- **Cursor** (`prices.ingest_cursor FINAL`) moves roughly once a minute and
  stays at the tip.
- **Frontier** — `behind_sec` for sdex/aquarius/soroswap ≤ ~80 s (was ~50 s).
  That is the designed latency, not a stall. `rollup-freshness-1m` fires only
  at 900 s.
- **DLQ** `prices-ingest-dlq-production` = 0, Lambda `Errors` = 0,
  `Prices/Ingest ForcedPartialFlushes` has no datapoints.
- ⚠️ **The first minute after the deploy is written from its tail only** — the
  old code left the cursor mid-minute. Expected; self-heals from the next
  minute. Do not read it as the fix failing.

## Step 8 — [prod CH, `dev_read`] the acceptance measurement, one full day

Run on the day AFTER the first full UTC day on the new binary. Target: `pct_lost`
≈ 0 (criterion 2). Adjust `d_from` / `d_to`.

**Baseline before the fix** (measured 2026-09-17 ~11:30 UTC, the last row is
a partial day):

| day | raw | stored | lost | % lost |
| --- | --- | --- | --- | --- |
| 2026-09-15 | 30,680 | 11,177 | 19,503 | **63.6** |
| 2026-09-16 | 21,183 | 8,453 | 12,730 | **60.1** |
| 2026-09-17 | 6,988 | 3,369 | 3,619 | **51.8** |

```sql
WITH
  toDate('2026-09-15') AS d_from,
  toDate('2026-09-17') AS d_to,
  pools AS (
    SELECT c.id AS cid
    FROM default.soroban_contracts AS c
    WHERE c.contract_id IN (SELECT contract_id FROM prices.pool_registry FINAL WHERE venue = 'aquarius')
  ),
  days AS (
    SELECT sequence AS seq, toDate(closed_at) AS day
    FROM default.ledgers
    WHERE toDate(closed_at) BETWEEN d_from AND d_to
  ),
  raw AS (
    SELECT d.day, count() AS raw_trades
    FROM default.soroban_events AS e
    INNER JOIN days AS d ON d.seq = e.ledger_sequence
    WHERE e.signature = 'trade'
      AND e.contract_id IN (SELECT cid FROM pools)
      AND e.ledger_sequence BETWEEN (SELECT min(seq) FROM days) AND (SELECT max(seq) FROM days)
    GROUP BY d.day
  ),
  stored AS (
    SELECT toDate(timestamp) AS day, sum(trade_count) AS stored_trades
    FROM prices.price_ohlcv_1m FINAL
    WHERE source = 'aquarius' AND toDate(timestamp) BETWEEN d_from AND d_to
    GROUP BY day
  )
SELECT r.day, r.raw_trades, s.stored_trades,
       r.raw_trades - s.stored_trades AS lost,
       round(100 * (r.raw_trades - s.stored_trades) / r.raw_trades, 1) AS pct_lost
FROM raw AS r
LEFT JOIN stored AS s ON s.day = r.day
ORDER BY r.day
FORMAT PrettyCompact
```

`raw` counts `trade` events only from contracts in `prices.pool_registry`,
which [[0285]] shows does not match what actually trades. So a small residual
(either sign) after the fix is a registry question, not this defect.

## Rollback

Check out the commit before the #313 squash, rerun steps 2, 3 and 5.
Redeploying Compute also restores `MAX_ITERATIONS=16`. The Observability alarm
can stay — without the new binary it simply never receives data.
