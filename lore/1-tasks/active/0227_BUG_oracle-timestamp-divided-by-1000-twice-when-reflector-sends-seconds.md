---
id: "0227"
title: "100% of oracle POLL readings land at 1970-01-21 — `lastprice` returns SECONDS and the ÷1000 was copied from the event path; the rows are wholly redundant"
type: BUG
status: active
related_adr: []
related_tasks: ["0167", "0173", "0170", "0061", "0141", "0199", "0086", "0083", "0196", "0200", "0215"]
tags: ["priority-high", "effort-small", "oracle", "data-correctness", "enrichment", "ops", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/oracle-worker/src/lib.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
history:
  - date: 2026-08-26
    status: backlog
    who: okarcz
    note: >
      Found while checking an unrelated `usd_rate` coverage question for
      [[0170]]. A monthly histogram of `oracle_prices` returned a `1970-01-01`
      bucket holding 3,186 rows per asset — with entirely plausible PRICES
      (XLM 0.1526-0.2199, USDC 0.9999-1.0013) and only the timestamp wrong.
      Root cause is ours: `oracle-worker/src/lib.rs:298` divides Reflector's
      timestamp by 1000 unconditionally. Reflector began returning some readings
      already in SECONDS around 2026-07-20; dividing those again yields
      1.787e6 = 1970-01-21. Nothing in our code changed at that date — the git
      log for that window is 0095/0097/0088/0096, all rollups and backfill — so
      the payload changed upstream and we had no tolerance for either shape.
      ⚠️ The same unconditional divide exists a second time at
      `prices-ingest-core/src/soroban.rs:667`, which ships in a DIFFERENT stack.
      Fixing one and deploying is the silent half-fix recorded in
      [[oracle-writers-span-two-stacks]].
      Ongoing at the time of filing: today's partial count is 74 rows.
  - date: 2026-08-26
    status: active
    who: okarcz
    absorbs: ["0199", "0086"]
    note: >
      ACTIVATED, and [[0199]] + [[0086]] folded in and archived as superseded —
      all three describe one defect on one table. The fold materially CORRECTED
      this task's own framing, which is why it was worth doing rather than
      closing the two as stale duplicates.
      🔴 The 2026-07-20 onset date above is REFUTED. 0086 measured 1970-stamped
      Reflector rows on prod on **2026-07-06**, two weeks earlier, and recorded
      that cleanup dropped partition `197001` only for the oracle-watcher to
      recreate it within minutes. `prices-production-cleanup` was disabled on
      **2026-07-20** ([[0200]], corroborated in [[0215]] and 0136) — the same
      date this task read as the onset. So 2026-07-20 is when the rows STOPPED
      BEING SWEPT, not when the bug started, and the "Reflector changed
      upstream" hypothesis loses its supporting evidence.
      Also corrected: the affected asset set is **three** assets (USDC 3, XLM 4,
      USDT 111 — 0086 caught all three), not two. USDT reads as unaffected today
      only because [[0196]]'s purge deleted its copy.
      Inherited from 0199: the independent /10^6-vs-/10^3 arithmetic, the
      `min(timestamp)` coverage trap that already misled [[0167]] once, and
      `raw_data` as the upstream-vs-ours discriminator. Inherited from 0086: the
      cleanup-worker interaction and the two-runs-1s-apart confirmation of the
      x1000 mapping.
  - date: 2026-08-26
    status: active
    who: okarcz
    note: >
      AC 1 CLOSED by measurement on prod — the question that gated severity.
      A 1970 row wins the enrichment ASOF **100% of the time** in the exposed
      population (471,087 of 471,087 USDC-quoted `_1d` candles in 2025), because
      real `reflector` coverage does not start until 2026-03-11 14:00 while
      candles run to 2015. And the 300 s staleness guard rejects **100% of them**,
      smallest gap 1.73e9 s — a 5.8-million-fold margin.
      🔑 **Severity settled: data-loss, not wrong-value.** No candle was ever
      priced from a 1970 row; re-enrichment stays out of scope. [[0199]]'s "it is
      inert" reached the right answer from reasoning that does not hold — its
      claim that the row "never matches" is outright false.
      🔴 Two findings beyond the AC. (1) Prod runs `join_use_nulls = 0`
      (`changed = 0`) and `enrich_batch` sets no SETTINGS, so an unmatched row
      yields DEFAULT not NULL — `o.price_usd IS NOT NULL` filters nothing and the
      tier's whole correctness rests on the one arithmetic guard. [[0170]]'s trap
      in a second place. (2) Asset 111 (USDT) has **zero** `reflector` rows at
      all, so the oracle tier cannot price a USDT-quoted candle — bears on
      [[0212]], [[0209]], [[0173]]. Not checked for other `oracle_name` values.
      Also: rows grew 3,186 -> 3,211 per asset within the session, confirming the
      defect is live and matching the 172-174/day series.
  - date: 2026-08-27
    status: active
    who: okarcz
    note: >
      🔑 **ROOT CAUSE FOUND, and it is not the one this task was filed on.**
      `raw_data` classified across BOTH bands (yesterday's census classified only
      the 1970 rows) returns a perfect partition, asset 3: corrupt = **3,264 rows,
      100% POLL** (`{"symbol":…}`); real = **48,311 rows, 100% EVENT**
      (`{"asset":…}`), 2026-03-11 14:00 -> 2026-08-27 07:50. Zero mixing, no
      `other`. The POLL path has never written a correct row; the EVENT path has
      never written a bad one.
      The two paths read the timestamp from different places carrying different
      units: `soroban.rs:659-667` takes `topic[2]` of the Reflector `update`
      event, documented in code as the u64 **ms** timestamp -> ÷1000 correct;
      `lib.rs:137-165` takes `PriceData.timestamp` from the `lastprice` return,
      which is **SECONDS** (SEP-40 / Soroban ledger convention) -> ÷1000 = 1970.
      The bug is `lib.rs:295`'s comment — the divide was copied across "to match
      the event-decoded path" on the assumption both carry the same shape.
      ✅ The 372-vs-288 write-volume gap is CLOSED, arithmetic exact: 288 EVENT +
      86.4 POLL stored slots = 374.4/day predicted vs **373.2 measured**.
      ✅ **Nothing is lost.** Good readings run 3.331 per 1000 s window where a
      corrupt row exists vs 3.316 where none does (theoretical max 3.333) — 99.94%
      of perfect. Cadence confirmed exactly 300 s (11,021 gaps of 300, 8 of 600).
      ✅ 0086's "conditional, not constant" is ANSWERED and REFUTED — not
      conditional at all, 100% of POLL readings are destroyed. It is a second
      WRITER, not a second branch.
      🔑 Every corrupt row is a **100% exact `Decimal(38,14)` price twin** of an
      EVENT row in the same reconstructed window (3,264/3,264) — so the POLL
      write is wholly redundant and the rows are worth **deleting, not
      repairing**; yesterday's "recovery is the better option" is retired again.
      🔴 Correction: USDT's absence is **BY DESIGN**, not a stopped writer.
      `reflector_key_to_identity` (`soroban.rs:109-118`) has no USDT arm — removed
      by [[0172]] and gated on [[0173]], with an explicit "do not restore" comment.
      ⏳ Lands on [[0226]]: the oracle-worker loads 620,615 assets to write 2 rows,
      and those 2 rows are the garbage ones.
---

# The oracle POLL path divides a SECONDS timestamp by 1000 — every one of its readings lands in 1970

> ⚠️ The filename slug (`…divided-by-1000-twice-when-reflector-sends-seconds`) is
> stale — there is no double division and Reflector did not change. Kept because
> the branch and PR #256 are named after it.

> **Consolidates [[0199]] (2026-08-13) and [[0086]] (2026-07-06)**, both archived
> as superseded on 2026-08-26. Three sightings of one defect, fourteen months of
> evidence between them. Their measurements are folded in below and attributed.

## Summary

Reflector's price reaches `prices.oracle_prices` by **two independent paths**, and
they disagree about the unit of the timestamp. One handles it correctly. The other
has never once got it right.

| path | reads | unit received | after `÷1000` | rows (asset 3) |
|---|---|---|---|---|
| **EVENT** — `soroban.rs:659-667` | `topic[2]` of the `update` event | **milliseconds** | correct ✅ | **48,311 — all good** |
| **POLL** — `lib.rs:137-165, 298` | `PriceData.timestamp` from `lastprice` | **seconds** | 1970-01-21 ❌ | **3,264 — all corrupt** |

**100% of POLL readings are destroyed. 0% of EVENT readings are.** The prices on
the corrupt rows are fine; only the timestamp is wrong, and we wrote it that way.

🔑 **And it has never mattered, because the EVENT path shadows it.** Every corrupt
row is an exact price twin of an EVENT row for the same reading — 3,264 of 3,264
matching to all 14 decimals. Nothing is lost, no candle was ever priced from a 1970
row, and `oracle_prices` has been fully correct this whole time by accident.

⚠️ **The three framings this task was filed under are all dead**, each killed by
measurement and each recorded below rather than deleted: "~30% of readings" (it is
100%, of one path, and none are lost), "Reflector changed upstream on 2026-07-20"
(that is the day cleanup stopped sweeping), and "the same divide exists twice so
fix both stacks" (one site is proven correct by 48,311 rows).

## The defect

`packages/oracle-worker/src/lib.rs:298`:

```rust
// Reflector reports millisecond timestamps; oracle_prices.timestamp
// is DateTime (epoch seconds), so divide by 1000 to match the
// event-decoded path (prices-ingest-core soroban.rs). The clamp is
// a backstop for the 2106 u32 ceiling, not the unit conversion.
timestamp: (pd.timestamp / 1000).min(u32::MAX as u64) as u32,
```

The comment states the assumption and the code never checks it. A value already
in seconds (~1.787 × 10⁹) becomes ~1.787 × 10⁶, which is 1970-01-21.

**0199's independent derivation of the same thing**, reached from the number
alone before the code was read: `1970-01-21 15:41:56` ≈ 1,784,516 epoch seconds,
while a mid-2026 instant is ≈ 1.786 × 10¹² ms. Dividing the millisecond value by
**10⁶** instead of **10³** lands almost exactly on the observed value — i.e. a
*double* division, one correct `/1000` plus a second one. That is precisely what
`lib.rs:298` does to an input already in seconds.

🔑 Two people, two routes, one answer. The mechanism is not in doubt.

## 🔴 The 2026-07-20 onset is REFUTED — it is a cleanup artifact

This task originally read the surviving row series as starting 2026-07-20 and
concluded Reflector changed its payload shape upstream on that date. **The fold
falsifies that.**

| date | source | what was seen |
|---|---|---|
| **2026-07-06** | **[[0086]]** | 6 rows in partition `197001` at `1970-01-21 15:22:41/42`, assets USDC/XLM/USDT |
| **2026-07-20** | [[0200]] / 0136 / [[0215]] | `prices-production-cleanup` **DISABLED** |
| 2026-08-13 | [[0199]] | USDC + USDT both `first_seen = 1970-01-21 15:41:56` |
| 2026-08-26 | this task | 6,372 rows, clean daily series from 2026-07-20 |

0086 recorded the mechanism that explains the whole shape: cleanup dropped
partition `197001`, and *the oracle-watcher recreated it within minutes*
(`modification_time 18:32:28`, post-drop). While cleanup ran, the evidence was
being deleted about as fast as it was written.

🔑 **2026-07-20 is the day the rows stopped being swept, not the day the bug
started.** The series looks like an onset because it is the left edge of the
retention change, and the two dates coincide exactly.

⚠️ **Consequence for the fix:** "Reflector changed what it sends around
2026-07-20" is now an unsupported hypothesis, not a finding. The defect may be
old and *intermittent* — which is what 0086 actually concluded from its own data:

> **Conditional, not constant**: the bulk of Reflector rows land correctly in
> `202607`, so only some code path divides by 1000. Likely a fallback timestamp
> field, or a Reflector `timestamp` returned in a different unit than the primary
> (ledger-close) path assumes.

That reading — *some readings, not all, and possibly a different field* — must be
tested against the code before a magnitude check is accepted as sufficient. A
magnitude check fixes the symptom either way, but if a fallback field is involved
we would be leaving the real selector unexamined.

## 🟡 Three assets, not two — ⚠️ SUPERSEDED, it is two by design (see 2026-08-27 below)

0086 caught all three peg/pivot assets on 2026-07-06:

```
timestamp             asset_id  oracle_name  price_usd          raw_data
1970-01-21 15:22:41   3         reflector    1.00014287729141   {"symbol":"USDC"}
1970-01-21 15:22:41   4         reflector    0.19963717762499   {"symbol":"XLM"}
1970-01-21 15:22:41   111       reflector    0.99949868096361   {"symbol":"USDT"}
1970-01-21 15:22:42   3         reflector    1.00051271277553   {"symbol":"USDC"}
1970-01-21 15:22:42   4         reflector    0.19941785827334   {"symbol":"XLM"}
1970-01-21 15:22:42   111       reflector    0.99948602782064   {"symbol":"USDT"}
```

USDT (asset_id 111) reads as unaffected in today's measurement **only because
[[0196]]'s purge deleted its copy** — recorded in 0199, which saw USDT's 1970 row
alive on 2026-08-13 before the purge took it. Do not conclude USDT is exempt.

### 🔴 Measured 2026-08-26 — USDT has NO `reflector` rows at all, 1970 or otherwise

The step-1 census returned **two rows, not three**. Asset 111 is absent from
`prices.oracle_prices WHERE oracle_name = 'reflector'` entirely — not zero 1970
rows, **zero rows**:

| asset_id | rows_1970 | first_real | last_real | rows_total |
|---|---|---|---|---|
| 3 (USDC) | 3,211 | 2026-03-11 14:00 | 2026-08-26 16:45 | 51,341 |
| 4 (XLM) | 3,211 | 2026-03-11 14:00 | 2026-08-26 16:45 | 51,341 |
| **111 (USDT)** | — | — | — | **absent** |

0086 saw asset 111 being written by `reflector` on 2026-07-06 (`price_usd
0.99949868096361`). [[0196]] purged those rows. **Nothing has re-added any since**
— so this is not the purge alone, it is the purge plus a writer that stopped.

⚠️ **Consequence beyond this task: the oracle tier cannot price a USDT-quoted
candle at all.** `o.asset_id = p.quote_asset_id` can never match for 111. That is
a live coverage hole and it lands squarely on [[0212]] (1.56 M `_1m` rows still
carrying the $1 peg), [[0209]] and [[0173]].

⚠️ **Scope of the claim — do not overstate it.** This is measured for
`oracle_name = 'reflector'` only. Whether asset 111 has rows under a *different*
`oracle_name` was not checked and must be, before anyone concludes USDT has no
oracle coverage. Filed as a follow-up rather than assumed either way.

### ✅ CORRECTED 2026-08-27 — USDT's absence is BY DESIGN, not a stopped writer

The section above concluded *"this is not the purge alone, it is the purge plus a
writer that stopped."* **That is wrong, and the code says so plainly.**
`reflector_key_to_identity` (`prices-ingest-core/src/soroban.rs:109-118`) maps
`XLM`/`native` and `USDC` and nothing else. The `USDT` arm was **removed by
[[0172]]**, and the doc comment immediately above it is explicit:

> ⚠️ **Do not restore the `USDT` arm to fix an apparent coverage gap.** The oracle
> tier runs *before* the pivot and wins where it applies, so an oracle row for this
> identity silently re-introduces the $1 peg that task 0172 removed. Restoring it
> requires fixing the symbol→issuer mapping first (task [[0173]]).

Both writers share that function, so **neither** can emit asset 111. Nothing
stopped; a deliberate decision was taken and documented. 0086's 2026-07-06 sighting
of USDT rows simply predates 0172.

⚠️ **The consequence still stands, and it is still owed** — the oracle tier cannot
price a USDT-quoted candle, which bears on [[0212]], [[0209]] and [[0173]]. But it
is a **known, owned design gap under 0173**, not a live defect for this task to
chase. The AC below is amended accordingly.

🔑 **The corruption is still accruing, and the rate is now pinned.** This task was
filed this morning at **3,186** rows per asset; the census hours later reads
**3,211** — **+25 per asset, +50 total, in a single session.** Consistent with the
172-174 rows/day series. It is not a historical artifact.

🔑 **A second independent confirmation of the ×1000 mapping, from 0086:** the two
timestamps are one second apart (`:41`, `:42`), which is ~1000 s apart in real
time — exactly two consecutive oracle-watcher runs at its real cadence. The
scaled domain reproduces the true polling interval.

## ✅ The same divide exists twice — and only one of them is wrong

| site | stack | shape | input unit | verdict |
|---|---|---|---|---|
| `oracle-worker/src/lib.rs:298` | **EventBridge** | `(pd.timestamp / 1000)` | **seconds** | ❌ 100% corrupt |
| `prices-ingest-core/src/soroban.rs:667` | **Compute** (ledger-processor) | `(ts_ms / 1000) as u32` | **milliseconds** | ✅ 48,311 rows correct |

🔑 **Identical arithmetic, opposite outcomes — because the inputs differ.** The
line is not the bug; the *assumption that both lines see the same thing* is, and
it is written down at `lib.rs:295`.

⚠️ **The half-fix warning is now inverted, and that is worth stating explicitly.**
This section originally read: *"Deploying one is a silent half-fix — the event-decode
path was not measured for this defect."* Measured, the risk runs the other way —
changing `soroban.rs:667` is the change that could break something, since it is the
only path producing usable rows. [[oracle-writers-span-two-stacks]] still holds as
a rule (verify writer behaviour by measurement, never by deploy exit status); it
was the *unmeasured symmetry assumption* that misled, not the memory.

## Evidence — measured on prod 2026-08-26

Reconstructing the corrupted timestamps by multiplying by 1000 gives a clean,
consecutive daily series from **2026-07-20 to 2026-08-26**, at a metronomic
172-174 rows/day across both assets:

```
2026-07-20   152      <- left edge = cleanup disabled, NOT onset
2026-07-21   174
   …          …
2026-08-13   148      <- Hetzner disk-full incident
2026-08-14   120      <- 11.5 h ingest stall
   …          …
2026-08-26    74      <- partial, still accruing
```

🔑 **The reconstruction validates itself.** Rows for 08-13 and 08-14 dip to 148
and 120 against an otherwise flat 172-174 — that is the known Hetzner disk-full
stall showing through. A wrong ×1000 mapping would not reproduce a real outage on
the correct dates, so the mapping is established rather than merely plausible.

⚠️ **The rate figure was corrected once already.** 3,186 of 51,235 readings per
asset is 6.2% *across all history*, but the corruption spans only five weeks —
within that window it is ~86 of ~288 readings per asset per day, i.e. **~30%**.
Quote the window figure; the all-time one understates the live problem.

⚠️ **And the window figure is now a floor, not a span.** With the onset refuted,
"five weeks" is the length of the *surviving* record. Pre-07-20 rows were swept,
so the true duration of the defect is unmeasured and at least seven weeks
(2026-07-06 → today).

## Impact — contained, but not harmless

✅ **Nothing corrupt reached `prices.usd_rate`.** It holds 48,049 rows for USDC
against 51,235 oracle readings, and `51,235 − 3,186 = 48,049` exactly. The
snapshotter rejects every affected row. 0199 measured the same thing from the
other side: `usd_rate`'s min for USDT was a sane `2026-03-11 14:00`.

## ✅ REFUTED 2026-08-26 — nothing is being discarded, and coverage is NOT thinned

This task claimed *"~30% of the rate readings for the last five weeks are being
discarded, thinning the ASOF join's coverage… a candle enriched in that window had
fewer rate points to match against than it should have."* **Measured, that is
false.**

`prices.usd_rate` is the negative image: the snapshotter (`writer.rs:493`) drops
every corrupt reading on an epoch floor, and the table is forever-retained rather
than swept — so a real shortfall would be permanently visible as a hole. Per month,
canonical USDC, `method = 'oracle'` (cadence is 5-minutely, so a clean day is 288):

| month | rates stored | days | per day | short |
|---|---|---|---|---|
| 2026-03 | 5,864 | 21 | 279.2 | 3.0% |
| 2026-04 | 8,550 | 30 | 285.0 | 1.0% |
| 2026-05 | 8,867 | 31 | 286.0 | 0.7% |
| 2026-06 | 8,564 | 30 | 285.5 | 0.9% |
| 2026-07 | 8,889 | 31 | 286.7 | 0.4% |
| **2026-08** | 7,397 | 26 | **284.5** | **1.2%** |

🔑 **August — deep inside the "corrupted" window — is as complete as April.** The
shortfall never exceeds 3%, and July, the month the corruption supposedly began, is
the *best* month in the table at 0.4%. Daily resolution across 2026-06-20 → 07-31
shows the same: 283-288 every single day, no step at 07-06, none at 07-20, and a
run of clean 288s from 07-21 onward.

**So there is no step, and no change in coverage at either candidate onset date.**

🔴 **CORRECTION 2026-08-26 — an earlier revision of this section went further than
the data supports.** It concluded *"the corrupt rows are ADDITIONAL junk, not LOST
readings — every scheduled poll still produces a good row."* That rested on an
assumed 288/day cadence which the write-volume measurement below **contradicts**.
What the flat `usd_rate` series actually establishes is narrower and still
valuable: **the corruption rate did not CHANGE at 2026-07-06 or 2026-07-20.**
Whether readings are lost is reopened — see "the write volume does not fit the
schedule" below.

✅ **Re-closed 2026-08-27, and the original wording was right after all.** The
corrupt rows ARE additional junk rather than lost readings, and every scheduled
poll DOES still produce a good row — the good series runs 3.331 per 1000 s window
against a theoretical 3.333, at an exactly 300 s cadence. What that revision got
wrong was its *reason*: it assumed one writer at 288/day. There are two, and the
good rows come from the other one. Right answer, wrong mechanism — recorded
because a conclusion that survives a refutation of its own premise is worth
distrusting until it is re-derived.

⚠️ **Severity drops a second time.** With AC 1 already showing the rows are inert in
the ASOF, and coverage now shown to be intact, what remains is: ~170 junk rows/day
that nothing reads, `min(timestamp)` defeated as a coverage measure, and partition
`197001` re-materialising against the cleanup worker. Real, worth fixing, and
**not** a correctness threat to any price.

### ✅ CLOSED 2026-08-27 — the arithmetic does not close, and it DID mean a second writer

> **Model A was right and model B was wrong**, though not in the form either was
> written. The second writer is not a retry and not a rogue path — it is
> `soroban.rs`'s event decoder, doing its job correctly on the same 5-minute
> cadence, and *it* produces the good rows while the POLL path produces only bad
> ones. The reasoning below reached the right fork; the section is kept intact
> because the question it asked is exactly the one that cracked the task open.

The numbers no longer add up, and this must be resolved before the fix is written.

| quantity | value |
|---|---|
| `usd_rate` rows, USDC, all months | 48,131 |
| `oracle_prices` real rows, USDC (51,341 − 3,211) | 48,130 |
| → real readings per day | **~285**, i.e. essentially the full 288 cadence |
| corrupt rows, USDC | 3,211 over ~37 days = **~87/day** |
| **implied total writes/day in the corrupt window** | **~372** |
| **scheduled poll cadence** | **288** |

🔑 **372 > 288.** The poll path writes at most one row per symbol per 5-minute run,
so it cannot produce ~285 good rows *and* ~87 bad ones in the same day. Something
else is writing, or something is collapsing. Two candidate models, both testable:

- **A — a second writer.** [[oracle-writers-span-two-stacks]] records that
  `reflector_key_to_identity` is shared with ledger-processor's event-decode path,
  which carries the *other* unconditional divide (`soroban.rs:667`). If that path
  writes under `oracle_name = 'reflector'`, it is the source — and **fixing
  `lib.rs:298` alone would change nothing**, which is the silent-half-fix trap this
  task already warns about, arriving from the opposite direction.
- **B — timestamp collision.** With `stored = sent / 1000` on integer division, two
  polls 300 s apart differ by only 0.3 in the stored domain, so **~3.3 consecutive
  polls truncate onto the same stored second** and a ReplacingMergeTree keyed on
  that timestamp would collapse them. 0086 saw exactly this shape — consecutive
  stored seconds `:41` and `:42`, which it read as ~1000 s apart in real time.

⚠️ **These are hypotheses, not findings.** Model B predicts heavy duplication at
each stored second; model A predicts none. They are distinguishable in one query
and must be settled before the magnitude check is designed.

❌ **It defeats `min(timestamp)` as a coverage measure** (from 0199). [[0167]]'s
whole argument turned on when oracle coverage starts, and a 1970 row makes the
naive query answer "1970" — it hid the real start date once already. Any coverage
query on this table written before the fix should be re-checked.

⚠️ **Operationally, it defeats the cleanup worker** (from 0086). Partition
`197001` re-materialises on every affected run, so [[0083]]'s worker drops it and
the next oracle run recreates it. Moot while cleanup is off, and live again the
moment [[0200]] turns it back on — so this must be fixed before that decision
lands, or the two will fight.

## ✅ SETTLED 2026-08-26 — the 1970 rows win the ASOF *always*, and the guard rejects them *always*

The open question above is closed by measurement. The answer is not the one
either 0199 or this task assumed, and the shape matters.

`price_ohlcv_1d`, USDC-quoted (`quote_asset_id = 3`), calendar 2025 — a slice
lying entirely before the first real oracle observation, run with
`join_use_nulls = 1` so a genuine 1970 row is distinguishable from a no-match:

| metric | value |
|---|---|
| candles in slice | 471,087 |
| `no_match` | **0** |
| ASOF won by a **1970 row** | **471,087 — 100%** |
| ASOF won by a real row | 0 |
| **survives the 300 s staleness guard** | **0** |
| smallest gap `p.timestamp - o.timestamp` | **1,733,901,838 s (~55 years)** |

🔑 **Both halves of the question have counter-intuitive answers.** A 1970 row does
not merely *sometimes* win the join — it wins **every single time** in the exposed
population, because real `reflector` coverage does not begin until
**2026-03-11 14:00** while candles run back to 2015. And the guard rejects every
one of them, with a margin of 1.73 × 10⁹ against a 300 s bound — a factor of 5.8
million. There is no plausible drift, clock skew or config change that closes that
gap.

**So the severity is settled: this is a data-loss bug, not a wrong-value bug.**
No candle has ever been priced from a 1970 row. Re-enrichment stays out of scope.

⚠️ **0199's conclusion was right; its reasoning was not sufficient to hold it.**
It wrote *"a 1970 row never matches a real candle and cannot poison a price… It is
inert."* The first clause is **false** — the row matches constantly, 100% of the
time. Only the second clause survives, and it survives because of a guard 0199
never checked. Recorded because "the conclusion was correct" is the most dangerous
possible reason to stop measuring.

🔑 **Why it is genuinely inert, stated precisely:** ASOF picks the newest
`o.timestamp <= p.timestamp`, so a 1970 row can only win when *no* real reading
precedes the candle. In exactly that case the alternative is no match at all —
which `o.price_usd IS NOT NULL` would drop anyway. The 1970 rows therefore
displace nothing. They cost scan work, not correctness.

### 🔴 Hardening finding — `o.price_usd IS NOT NULL` is dead code in production

Measured alongside: the prod server has **`join_use_nulls = 0`, `changed = 0`**,
and `enrich_batch` (`ch_enrich.rs:805`) sets no `SETTINGS` clause. At that default
an unmatched `LEFT JOIN` row yields the column **DEFAULT, not NULL** — so
`o.price_usd` returns `0` and `o.timestamp` returns `1970-01-01`. `IS NOT NULL` is
therefore **always true** and filters nothing.

⚠️ **The entire correctness of the oracle tier rests on the single arithmetic
guard** `(p.timestamp - o.timestamp) <= 300`. It holds today by an enormous
margin, so nothing is broken — but the redundancy the code appears to have is not
there. This is [[0170]]'s `join_use_nulls` trap in a second place, and it means a
real 1970 row and a total no-match are indistinguishable to the production query.

⚠️ It also means an unmatched row is only rejected because epoch 0 is far from
`p.timestamp`. Sound for any candle this system can hold, but it is arithmetic
doing a null-check's job, and it should be named as such rather than left to be
rediscovered.


## ✅ The writer is identified, and the compression is confirmed — 2026-08-26

Two hypotheses were open. Both are now settled, and the second settles the onset
question with a mechanism rather than an inference.

### Writer: the POLL path only

`raw_data` discriminates the two sites — `oracle-worker/src/lib.rs:301` writes
`{"symbol":…}`, `soroban.rs:698` (`decode_reflector`) writes `{"asset":…}`. Both
tag `oracle_name = 'reflector'`, so both would appear. Measured over all 1970 rows:

| writer | asset_id | rows | distinct ts | min stored | max stored |
|---|---|---|---|---|---|
| **POLL → `lib.rs:298`** | 3 | 3,212 | 3,210 | 1970-01-21 15:41:56 | 1970-01-21 16:36:03 |
| **POLL → `lib.rs:298`** | 4 | 3,212 | 3,210 | 1970-01-21 15:41:56 | 1970-01-21 16:36:03 |
| EVENT → `soroban.rs:667` | — | **0** | — | — | — |

🔑 **Not one row from the event-decode path.** The fix belongs in `oracle-worker`.

⚠️ **This census classified only the 1970 rows, and that was the gap.** It could
say which writer produced the corrupt band but not what the *good* band was made
of — so it left "the event path may simply not be decoding Reflector `update`
events on prod at all" open, and read the absence of EVENT rows as possible
merge-loss rather than as evidence. Classifying **both** bands on 2026-08-27
answered it in one query: see "🔑 SOLVED 2026-08-27" below. The exemption for
`soroban.rs:667` is now evidence-backed rather than assumed.

### Compression confirmed — 98.83% dense against a 0.331% control

| band | asset | rows | distinct ts | span (s) | density |
|---|---|---|---|---|---|
| **1970 (corrupt)** | 3, 4 | 3,212 | 3,210 | 3,248 | **98.83%** |
| real (control) | 3, 4 | 48,134 | 48,134 | 14,526,301 | **0.331%** |

The control is exactly `1/302` — one reading per ~302 s, the 5-minute cadence. The
corrupt band is **299× denser**, saturating essentially every second of its span.

Reconstructed ×1000, the corrupt span is **2026-07-20 → 2026-08-26** — 3,248
stored seconds = 37.6 real days. So the stored row count measures **the width of
the surviving window, not the number of corrupted readings**: the two are related
by the 1000× compression, and the reading count cannot be recovered from it.

🔑 **This is what finally settles the onset, and it settles it against the
original claim.** `usd_rate` is flat at 284-287/day in *every* month from
2026-03-11 onward, so the corruption rate did not change at 07-20. Yet the
surviving corrupt window begins **exactly** at 07-20 — the day
`prices-production-cleanup` was disabled. A rate that did not change, plus a
window that starts precisely when deletion stopped, is the signature of an old
defect whose evidence was being swept. **The "Reflector changed upstream around
2026-07-20" hypothesis is dead**, and 0086's 07-06 sighting is simply a sample
taken before the sweep stopped.

### ✅ RESOLVED 2026-08-27 — the write volume fits two schedules, not one

Kept in full below because the reasoning was sound and only the conclusion was
premature. The gap closes exactly once the second writer is identified: **288
EVENT rows/day + 86.4 POLL stored slots/day = 374.4 predicted, against 373.2
measured.** Neither hypothesis in the section below was right — it was not Lambda
retries (model A as framed) and not readings collapsing out of the good series
(model B). It was two writers on the same 5-minute cadence, one of them 100%
corrupt and compressing 3.33:1 in the stored domain.

⚠️ **And "how many readings are lost" — the question this section reopened — is
answered: none.** See the measurement below.

### 🔴 the original OPEN framing — the write volume does not fit the schedule

`infra/envs/production.json:17` sets `"oracleWatcher": "rate(5 minutes)"` — **288
invocations/day**, one row per asset per invocation. Measured, per asset:

| | rows/day |
|---|---|
| good (`usd_rate`, Aug) | 284.5 |
| corrupt slots (3,210 / 37.6 d) | 85.4 |
| **total** | **~370** |
| **schedule permits** | **288** |

**~29% more rows than the schedule allows.** The two figures cannot both come from
one row per scheduled invocation, and this is unresolved.

⚠️ It also means the corruption *rate* is still unknown. Saturation at 98.83% is
reached by anything from ~86 to 288 corrupt polls/day — one per slot, or 3.3 per
slot collapsed by the ReplacingMergeTree. The stored count cannot distinguish
them, so **how many readings are actually lost is still an open question**, and the
"~30%" in the title is not yet earned by measurement.

Leading hypothesis: **Lambda async retries.** EventBridge invokes asynchronously
and retries twice on failure; [[0214]] measured exactly this shape on the
enrichment worker — *"3×/hour (one trigger plus two Lambda async retries)"*. A
partially-failing oracle invocation would write extra rows on each retry.

## 🔑 SOLVED 2026-08-27 — two writers, two units; the POLL path is 100% corrupt and 100% redundant

Four measurements on prod, asset 3 (USDC). Together they close the root cause, the
write-volume gap, the loss question and 0086's conditionality question.

### A — nothing is lost

A corrupt row stored at second `s` was really taken somewhere in the real window
`[s×1000, s×1000+999]` — 1,000 real seconds, which at a 300 s cadence should hold
**3.333** good readings. Counting good rows per window, split on whether that
window also holds a corrupt row (`join_use_nulls = 1`, per [[0170]]'s trap):

| band | windows | good rows | good per window |
|---|---|---|---|
| window has a corrupt row | 3,263 | 10,870 | **3.331** |
| window has none (control) | 38 | 126 | **3.316** |

🔑 **99.94% of theoretical perfect, inside the corrupt windows.** A displaced
reading would read 2.33 — a 30% drop. The observed difference is **0.45%**. The
corrupt rows displace nothing.

### B — the cadence is exactly what the config says

| gap between consecutive good rows | count |
|---|---|
| **300 s** | **11,021** |
| 600 s | 8 |

Eight missed polls in 38 days, nothing else. (`1784505600` also appears once — the
first row having no predecessor, not a gap.) So `"oracleWatcher": "rate(5 minutes)"`
= 288/day is real, and the good series *achieves* it: 288/day, every day, flat.

### C — 86.4 is a saturation ceiling, not a rate

`corrupt_slots` reads **86-87 every single day**. That is **86,400 ÷ 1,000** — the
number of 1000-second windows in a day. The compressed band is not "86 corrupted
readings a day", it is *every available slot in the compressed domain, filled*. The
count never measured a rate. This confirms the 2026-08-26 compression finding
numerically rather than by inference.

### D — the corrupt row is the same reading, written twice

| | value |
|---|---|
| corrupt rows | 3,264 |
| with an **exact** `Decimal(38,14)` price twin in the same window | **3,264** |
| | **100%** |

At 14 decimal places this is not coincidence. Every corrupt row carries a price
already stored correctly by another row.

### E — the discriminator: one outage, one survivor

Hourly across the 2026-08-13 Hetzner disk-full incident:

```
2026-08-13 20:00   good 12   corrupt 2
2026-08-13 21:00   good 12   corrupt 0   <- corrupt writer dies
   …                  …            …
2026-08-14 06:00   good 12   corrupt 0
2026-08-14 07:00   good 12   corrupt 3   <- and returns, 10 h later
```

**The good series does not miss a single poll through the entire outage.** Two
series, one incident, one survivor — so two writers with independent failure modes.
(`run_oracle` fails the whole pass on a CH write failure, `lib.rs:271`; the
ledger-processor kept going.)

### The mechanism

`raw_data` classified across **both** bands — the step 2026-08-26's census missed:

| band | writer | rows | first seen | last seen |
|---|---|---|---|---|
| corrupt (1970) | **POLL `{"symbol":"USDC"}`** | 3,264 | 1970-01-21 15:41:56 | 1970-01-21 16:36:57 |
| real | **EVENT `{"asset":"USDC"}`** | 48,311 | 2026-03-11 14:00 | 2026-08-27 07:50 |

A perfect partition — zero mixing, no `other`. The two paths take the timestamp
from different fields, and those fields carry different units:

- **EVENT**, `soroban.rs:659-667` — `topic[2]` of the `update` event, whose own doc
  comment says *"topic[2] is the u64 **ms** timestamp"*. `÷1000` → correct.
  (Its fallback, `ev.created_at * 1000`, deliberately normalises ledger-close
  seconds *up* to ms first. That is the "fallback timestamp field" 0086
  hypothesised — real, and handled correctly. Just not the culprit.)
- **POLL**, `lib.rs:137-165` — `PriceData.timestamp` from the `lastprice` return.
  Reconstructing: 1,784,516 × 1000 = 1,784,516,000 = 2026-07-20 **in seconds**,
  the SEP-40 / Soroban ledger convention. `÷1000` → 1970.

🔑 **The bug is the comment at `lib.rs:295`.** *"Reflector reports millisecond
timestamps … divide by 1000 **to match the event-decoded path**"* — the divide was
copied across on the assumption that the RPC return carries the same shape as the
event topic. It does not. This task's own warning was right: *"whether the event
payload carries the same shape as the RPC response … is an open question, not an
assumption to carry."*

### What follows

- ✅ **The fix is one line, in one stack.** `soroban.rs:667` is proven correct by
  48,311 rows over 5.5 months.
- ✅ **Not conditional.** 0086 was right about *path* and wrong about *conditional*
  — 100% of POLL readings are destroyed. It is a second writer, not a second branch.
- ✅ **Delete, do not repair.** Every corrupt row is a price twin of a correctly
  stored row; a ×1000 repair would land on instants the EVENT path already covers
  and add zero information.
- 🔴 **The POLL write to `oracle_prices` is wholly redundant.** Every reading it
  produces is already stored correctly, and the `usd_rate` snapshotter reads the
  EVENT rows. Lands on [[0226]] — that worker loads **620,615 assets to write 2
  rows**, and those 2 rows are the garbage ones. Deleting the write would resolve
  0226 outright. ⚠️ Not free: the poll worker also owns the `usd_rate` snapshot and
  [[0228]] is open on XLM never being snapshotted, so both need settling first.
  Recorded here, decided in 0226.

## Implementation

> Rewritten 2026-08-27 against the measured root cause. The pre-measurement plan
> — magnitude checks at both sites, `raw_data` as an upstream discriminator, and
> repairing the rows — is superseded; the reasons are recorded in the sections
> above rather than deleted.

- **`lib.rs:298` — stop dividing.** `lastprice` returns **seconds**. Correct the
  comment at `lib.rs:295` in the same change: it is the actual defect, and it
  names the wrong path as its authority.
- **Keep a magnitude check anyway, as a tolerance rather than a fix.** Treat a
  value ≥ 10¹¹ as milliseconds and divide, otherwise take it as seconds — bounds
  chosen so both shapes are unambiguous for any date this system can serve. The
  unit is an upstream contract we do not control, and this task exists because
  someone assumed it once.
- **Reject loudly.** A reading whose timestamp is implausible for this system
  (before the oracle window, or in the future) must be **rejected and alarmed**,
  not written. This is the single most valuable deliverable here: the defect was
  discoverable only by stumbling over it, and it was stumbled over three times.
- ⚠️ **`soroban.rs:667` is EXEMPT from change, on evidence** — 48,311 correct rows
  since 2026-03-11, zero corrupt. A magnitude check there is a harmless no-op and
  may be added for symmetry, but it is not required and must not gate the deploy.
  This retires the standing "do not fix only oracle-worker" instruction, which was
  correct while the event path's shape was unknown.
- **Delete the 3,264 (×2 assets) corrupt rows; do not repair them.** Every one is
  an exact price twin of an EVENT row already stored correctly, so a ×1000 repair
  adds no information and would collide with rows the EVENT path already owns.
  ⚠️ Deletion is `ALTER … DELETE` / partition drop on `197001` — the same
  partition [[0083]]'s cleanup worker targets, so sequence it against [[0200]].
- **Then ask whether the POLL path should write to `oracle_prices` at all**
  ([[0226]]). It is wholly redundant today. Blocked on the `usd_rate` snapshot
  ownership and [[0228]]; recorded here, decided there.

## Implementation — shipped 2026-08-27

One function, one stack (`oracle-worker`), and a guard that did not exist before.

```rust
pub fn reflector_timestamp_to_epoch_seconds(raw: u64, now_secs: u64) -> Result<u32, BadTimestamp>
```

- **The unit is decided by magnitude, not by declaration.** At or above
  `REFLECTOR_MILLIS_THRESHOLD` (1e11) the value is milliseconds; below it,
  seconds. That threshold sits in a ~3,000-year dead zone — 1e11 seconds is the
  year 5138, 1e11 milliseconds is 1973 — so no real reading is ambiguous and the
  exact placement is not load-bearing.
- **Implausible readings are refused, not written.** Before Stellar genesis, or
  more than `FUTURE_SKEW_SECS` (1 h) ahead of our clock, and the sample is
  dropped with an `error` log naming the raw value; `skipped` counts it. Both
  failure shapes of a unit mistake are covered: seconds-read-as-millis lands in
  1970, millis-read-as-seconds lands in the far future.
- **`now_secs` is passed in**, so the boundary is testable rather than
  wall-clock dependent. Read once per pass, not per symbol, so the window cannot
  move underneath a batch. On the impossible branch (a clock before 1970) the
  future bound is disabled rather than made infinitely strict — the guard exists
  to catch a unit mistake, and a broken clock must not silently stop the feed.

🔑 **The test that would have caught this in 2026-03 is
`a_seconds_reading_is_taken_as_seconds`** — a single assertion that a plausible
`lastprice` value survives the conversion unchanged. There was no test on this
conversion at all; the unit was asserted only in a comment, and the comment was
wrong.

⚠️ **`soroban.rs:667` is untouched, by measurement** — 48,311 correctly stamped
rows across 5.5 months, zero corrupt. The standing "deploy both stacks" rule
(recorded because `reflector_key_to_identity` is shared) does not apply to this
defect, and the exemption is evidenced rather than assumed.

## Acceptance Criteria

- [x] Whether a 1970 row can win the enrichment ASOF join is **established by
      measurement**, and the answer is recorded. This gates the severity.
      → **It wins 100% of the time and is rejected 100% of the time.** 471,087/471,087
      candles in the 2025 USDC slice matched a 1970 row; 0 survived the 300 s guard,
      smallest gap 1.73e9 s. **Severity settled: data-loss, not wrong-value.**
      See "SETTLED 2026-08-26" above. 0199's "it is inert" was the right conclusion
      from insufficient reasoning — its first clause is outright false.
- [x] `lib.rs:298` takes `lastprice`'s timestamp as **seconds**, with a magnitude
      check as tolerance, and a test covering a seconds input, a millis input and
      the boundary. The `lib.rs:295` comment is corrected in the same change.
      → `reflector_timestamp_to_epoch_seconds(raw, now_secs)`. Decides the unit
      from the **magnitude** rather than from any declared unit, so the mistake
      that caused this — trusting a comment about another code site — cannot
      recur in the same shape. The false comment is gone; what replaces it says
      what the two arms actually read. Tests:
      `a_seconds_reading_is_taken_as_seconds` (the defect itself — this exact
      input became 1970-01-21 before), `a_millis_reading_is_converted`,
      `the_threshold_sits_in_a_dead_zone_where_neither_unit_is_plausible`.
- [x] The event-decode path's real payload shape is stated as evidence, not
      assumed from the oracle-worker comment.
      → **`topic[2]`, u64 milliseconds**, per `soroban.rs:652-667`'s own doc
      comment, with the `ev.created_at * 1000` fallback normalising ledger-close
      seconds up to ms first. Corroborated on prod by **48,311 EVENT-tagged rows,
      100% correctly stamped, 2026-03-11 → 2026-08-27**. This also settles the
      standing "the event path may not be decoding on prod at all" doubt: it is.
- [x] Whatever ships is deployed to **both** stacks, or the exemption is written
      down with its reasoning.
      → **`soroban.rs:667` is exempt, on measurement.** 48,311 correct rows, zero
      corrupt, across 5.5 months. Only `oracle-worker` ships. Recorded in
      Implementation above.
- [ ] New readings stop landing before the oracle window — verified on prod after
      deploy, not from the code.
- [x] The 6,372 existing rows are repaired or explicitly written off, with the
      decision recorded.
      → **Decision: DELETE.** All 3,264 per asset are exact `Decimal(38,14)` price
      twins of EVENT rows already stored correctly (3,264/3,264), so a ×1000
      repair adds no information. ⚠️ The *execution* of the delete is still owed
      and is sequenced against [[0200]] — see Implementation.
- [x] A malformed timestamp is rejected loudly rather than written, with a test.
      → `BadTimestamp::{BeforeGenesis, InTheFuture}`, carrying the raw value so
      the log line alone is diagnosable. The row is **not written**, `skipped` is
      incremented and it logs at `error`. Tests:
      `an_implausible_reading_is_rejected_loudly` (including a reading already
      divided once — exactly what this bug produced — and `0`),
      `a_reading_slightly_ahead_of_our_clock_is_accepted` pinning the skew
      boundary.

### Folded in from [[0086]]

- [x] The **conditional** nature is explained — why the bulk of readings land
      correctly and only some divide wrongly.
      → **It is not conditional. The premise was wrong.** 0086 inferred a
      conditional branch from "the bulk of rows land correctly"; the bulk land
      correctly because a **different writer** produces them. Measured: corrupt =
      100% POLL, real = 100% EVENT, zero mixing. Every POLL reading is destroyed;
      no EVENT reading is. 0086 was right that "only some code path divides by
      1000" and wrong that the split is per-reading. Its "likely a fallback
      timestamp field" guess was also real but innocent — `soroban.rs:666`'s
      `ev.created_at * 1000` — and correctly handled.
- [ ] Partition `197001` stays empty after the fix, verified across a live
      oracle-watcher run — and the interaction with [[0083]]'s cleanup worker is
      settled before [[0200]] re-enables it.
- [ ] Both writable assets confirmed clean (USDC 3, XLM 4).
      ⚠️ **Amended 2026-08-27 — USDT 111 is struck from this criterion.** Its
      absence is by design, not evidence lost to [[0196]]'s purge:
      `reflector_key_to_identity` has no USDT arm, removed by [[0172]] and gated on
      [[0173]], with an explicit "do not restore" comment. Both writers share that
      function, so neither can emit asset 111. 0086's 2026-07-06 sighting predates
      0172. The coverage consequence is real and owned by 0173 — not by this task.

### Folded in from [[0199]]

- [x] ~~`raw_data` inspected and the upstream-vs-ours question answered from the
      stored payload~~ — **IMPOSSIBLE AS WRITTEN, criterion retired 2026-08-26.**
      `oracle-worker/src/lib.rs:301` writes `raw_data` as
      `format!("{{\"symbol\":\"{symbol}\"}}")` — a literal WE construct, holding
      the symbol and nothing else. There is no stored payload and no original
      timestamp, so the column cannot answer the question. 0199's premise that it
      "keeps the original payload" was wrong.
      → Replaced by the `usd_rate` gap test below, which reaches the same question
      through durable data.
- [x] **Onset settled via `prices.usd_rate` gaps.** ✅ Closed 2026-08-26 and
      reinforced 2026-08-27. `usd_rate` is flat at 284-287/day in every month from
      2026-03-11, so the corruption rate never changed — while the surviving
      corrupt window begins **exactly** on 2026-07-20, the day
      `prices-production-cleanup` was disabled. A rate that did not change plus a
      window starting precisely when deletion stopped is an old defect whose
      evidence was being swept. The "Reflector changed upstream" hypothesis is
      dead. 2026-08-27 adds the mechanism: `usd_rate` tracks the **EVENT** path,
      which was never corrupt, so its flatness was always going to be flat.
      **The true onset is UNDATABLE** — see the criterion below, which is the one
      that applies. Original text follows.
      The snapshotter
      (`writer.rs:493`) drops corrupt readings with `o.timestamp > ORACLE_EPOCH_FLOOR`,
      and `usd_rate` is forever-retained rather than swept — so every corrupt
      reading leaves a permanent *hole* there. Its per-day row count is therefore a
      fossil record of the corruption rate that survives even where `oracle_prices`
      partition `197001` was swept. A uniform shortfall back to 2026-03-11 kills the
      "Reflector changed on 2026-07-20" hypothesis outright; a step change dates the
      real onset.
- [x] ⚠️ If neither test dates the onset, say so and stop.
      → **Neither test dates it, and this is the honest answer: the onset is
      undatable.** The POLL path has been wrong since it was written; the
      pre-2026-07-20 rows were swept by cleanup; and `usd_rate` records the EVENT
      path, which never carried the defect, so it holds no fossil of it. The only
      hard floor is **2026-07-06** ([[0086]]'s direct sighting). No onset is
      inferred.
- [ ] Coverage queries that use `min(timestamp)` on `oracle_prices` are
      re-checked — this defect already gave [[0167]] a wrong start date once.
- [ ] Whether the 13-month retention should have dropped partition `197001` is
      answered. ⚠️ 0086 supplies the likely answer — it *was* being dropped and
      immediately recreated — so confirm that rather than re-deriving it.

## Out of scope

- [[0173]]'s USDT mis-attribution — a different oracle defect on the same table.
- [[0228]] — XLM's readings never being snapshotted. Found in the same session and
  on the same table, but a design question rather than a bug.
- Re-enriching candles, unless the ASOF finding above shows wrong values were
  written.
- Re-enabling the cleanup worker — that is [[0200]]. This task only has to leave
  `197001` in a state that will not fight it.

## Notes

- Found by a query aimed at something else entirely. The monthly histogram was
  checking `usd_rate` coverage for [[0170]]; the `1970-01-01` bucket was not what
  it was looking for.
- ⚠️ **Nothing value-based would ever have caught this.** The prices on the
  affected rows are entirely plausible — XLM at 0.15-0.22, USDC at ~1.00. Only
  the timestamp is wrong, and no alarm or guard reads timestamps for plausibility.
  Same class as [[0215]]'s invisible failure, where every data signal read normal.
- 🔑 **The comment was the bug, and it cited the wrong authority.** `lib.rs:295`
  does not merely assert milliseconds — it justifies the divide as being *"to
  match the event-decoded path"*. A cross-reference to a second code site was
  treated as evidence about an upstream payload. It is worth noticing that the
  most confident-looking sentence in the file was the false one, and that it stood
  for months precisely because it looked like it had already been checked.
- 🔑 **Yesterday's census asked the right question of half the data.** The
  2026-08-26 `raw_data` classification ran over the 1970 rows only, found no EVENT
  rows there, and correctly refused to read that as exoneration. One query over
  *both* bands settled everything. The lesson is not "measure more" — it is that a
  census restricted to the anomalous population cannot see what the normal
  population is made of, and the control group is where the answer was.
- 🔑 **Filed three times in seven weeks and fixed none of them.** 0086
  (2026-07-06, from a cleanup-RBAC proof), 0199 (2026-08-13, from the 0196 purge
  measurement), 0227 (2026-08-26, from an 0170 coverage query) — three unrelated
  investigations, each of which found this table by accident and filed a fresh
  task rather than finding the existing one. That is the argument for the loud
  guard: the defect is discoverable only by stumbling over it, so it will keep
  being re-found until something rejects it at write time.
