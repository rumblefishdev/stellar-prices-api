---
id: "0227"
title: "~30% of oracle readings land at 1970-01-21 — the Reflector timestamp is divided by 1000 unconditionally, and it is not always milliseconds"
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
---

# Reflector timestamps are divided by 1000 whether or not they are milliseconds

> **Consolidates [[0199]] (2026-08-13) and [[0086]] (2026-07-06)**, both archived
> as superseded on 2026-08-26. Three sightings of one defect, fourteen months of
> evidence between them. Their measurements are folded in below and attributed.

## Summary

`prices.oracle_prices` holds **6,372 rows** (3,186 per asset, XLM and USDC) whose
timestamp reads **1970-01-21**. The prices on those rows are fine. Only the
timestamp is destroyed, by our own unit conversion.

Roughly **30% of every day's oracle readings** currently land this way, and it is
still happening. The defect is **at least as old as 2026-07-06** (0086) — see the
onset correction below, which is the single most important thing the fold changed.

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

## 🔴 Three assets, not two

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

🔑 **The corruption is still accruing, and the rate is now pinned.** This task was
filed this morning at **3,186** rows per asset; the census hours later reads
**3,211** — **+25 per asset, +50 total, in a single session.** Consistent with the
172-174 rows/day series. It is not a historical artifact.

🔑 **A second independent confirmation of the ×1000 mapping, from 0086:** the two
timestamps are one second apart (`:41`, `:42`), which is ~1000 s apart in real
time — exactly two consecutive oracle-watcher runs at its real cadence. The
scaled domain reproduces the true polling interval.

## 🔴 The same divide exists twice, in two different stacks

| site | stack | shape |
|---|---|---|
| `oracle-worker/src/lib.rs:298` | **EventBridge** | `(pd.timestamp / 1000)` |
| `prices-ingest-core/src/soroban.rs:667` | **Compute** (ledger-processor) | `(ts_ms / 1000) as u32` |

⚠️ Deploying one is a **silent half-fix** — see [[oracle-writers-span-two-stacks]].
The event-decode path was not measured for this defect; whether it is affected
depends on whether the event payload carries the same shape as the RPC response,
and that is an open question, not an assumption to carry.

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

❌ **~30% of the rate readings for the last five weeks are being discarded**,
thinning the ASOF join's coverage during precisely the period the depeg-aware
oracle tier exists to cover. A candle enriched in that window had fewer rate
points to match against than it should have.

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

## Implementation

- Magnitude check at both sites rather than a unit assumption: treat a value
  ≥ 10¹¹ as milliseconds and divide, otherwise take it as seconds. Bounds chosen
  so both shapes are unambiguous for any date this system can serve.
- ⚠️ Do **not** fix only `oracle-worker`. Establish the event-decode path's
  actual payload shape first, then fix and deploy both stacks or state explicitly
  why one is exempt.
- Test 0086's "conditional, not constant" reading against the code: is there a
  fallback timestamp field, or a secondary path, that explains why only *some*
  readings are affected? A magnitude check masks this either way — answer it
  before it is masked.
- Read `raw_data` to settle **upstream vs ours** (0199). The column keeps the
  original payload, so it can say directly whether Reflector sent seconds or we
  mangled milliseconds. This is the cheapest available test of the refuted
  "Reflector changed" hypothesis.
- Repair the 6,372 existing rows. The ×1000 mapping is proven, so they are
  recoverable rather than lost — but `oracle_prices` is an input to enrichment,
  so a repair means deciding whether affected candles get re-enriched.
  ⚠️ 0199 proposed **deleting** them instead, on the belief the true instant was
  unrecoverable. That belief predates this task's reconstruction; recovery is now
  the better option and the delete recommendation is retired.
- Add a guard so this cannot recur silently: a reading whose timestamp is
  implausible for this system (before the oracle window, or in the future)
  should be **rejected loudly**, not written.

## Acceptance Criteria

- [x] Whether a 1970 row can win the enrichment ASOF join is **established by
      measurement**, and the answer is recorded. This gates the severity.
      → **It wins 100% of the time and is rejected 100% of the time.** 471,087/471,087
      candles in the 2025 USDC slice matched a 1970 row; 0 survived the 300 s guard,
      smallest gap 1.73e9 s. **Severity settled: data-loss, not wrong-value.**
      See "SETTLED 2026-08-26" above. 0199's "it is inert" was the right conclusion
      from insufficient reasoning — its first clause is outright false.
- [ ] Both conversion sites handle either unit, with a test per site covering a
      seconds input, a millis input, and the boundary between them.
- [ ] The event-decode path's real payload shape is stated as evidence, not
      assumed from the oracle-worker comment.
- [ ] Whatever ships is deployed to **both** stacks, or the exemption is written
      down with its reasoning.
- [ ] New readings stop landing before the oracle window — verified on prod after
      deploy, not from the code.
- [ ] The 6,372 existing rows are repaired or explicitly written off, with the
      decision recorded.
- [ ] A malformed timestamp is rejected loudly rather than written, with a test.

### Folded in from [[0086]]

- [ ] The **conditional** nature is explained — why the bulk of readings land
      correctly and only some divide wrongly. A magnitude check that fixes the
      symptom without answering this closes the criterion only if the
      investigation is recorded as inconclusive, not skipped.
- [ ] Partition `197001` stays empty after the fix, verified across a live
      oracle-watcher run — and the interaction with [[0083]]'s cleanup worker is
      settled before [[0200]] re-enables it.
- [ ] All three affected assets confirmed clean (USDC 3, XLM 4, **USDT 111** —
      whose evidence [[0196]] purged, so its absence today proves nothing).

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
- [ ] **Onset settled via `prices.usd_rate` gaps.** The snapshotter
      (`writer.rs:493`) drops corrupt readings with `o.timestamp > ORACLE_EPOCH_FLOOR`,
      and `usd_rate` is forever-retained rather than swept — so every corrupt
      reading leaves a permanent *hole* there. Its per-day row count is therefore a
      fossil record of the corruption rate that survives even where `oracle_prices`
      partition `197001` was swept. A uniform shortfall back to 2026-03-11 kills the
      "Reflector changed on 2026-07-20" hypothesis outright; a step change dates the
      real onset.
- [ ] ⚠️ If neither test dates the onset, say so and stop — the pre-07-20 rows were
      deleted, and an undatable onset is an honest answer. Do not infer one.
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
- 🔑 **Filed three times in seven weeks and fixed none of them.** 0086
  (2026-07-06, from a cleanup-RBAC proof), 0199 (2026-08-13, from the 0196 purge
  measurement), 0227 (2026-08-26, from an 0170 coverage query) — three unrelated
  investigations, each of which found this table by accident and filed a fresh
  task rather than finding the existing one. That is the argument for the loud
  guard: the defect is discoverable only by stumbling over it, so it will keep
  being re-found until something rejects it at write time.
