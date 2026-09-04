---
id: "0127"
title: "Tranche 2 backfill-depth gate — earliest_data_available ≤ 2022-01-01 and USDC 1d candles spot-checked"
type: FEATURE
status: active
related_adr: ["0005", "0009"]
related_tasks: ["0088", "0106", "0114", "0116", "0101", "0128", "0170", "0165"]
tags: [layer-indexing, priority-high, effort-medium, milestone-M2, backfill, sdex, verification, acceptance]
milestone: 2
links:
  - "../../../lore/1-tasks/archive/0088_FEATURE_soroban-backfill-run-tracker/README.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Owns Tranche 2
      acceptance criteria 5 and 6 — the backfill-depth milestone and the
      independent-source spot-check of 1d candles. The **run** itself is
      [[0088]]; this task is the gate and the verification on top of it.
  - date: 2026-09-04
    status: active
    who: okarcz
    note: >
      Activated. Picked as the last Tranche 2 acceptance gate still open to this
      author — [[0121]], [[0122]] and [[0126]] are closed, [[0120]] belongs to
      another developer, and [[0128]] cannot start until this one lands.
      ⚠️ The Context section below is written as though the run were still in
      flight; it is not. **[[0088]] is completed and archived**, so this task is
      now purely measurement and write-up, with no waiting. Its two stated
      dependencies are also cleared: [[0106]] (the stored-vs-computed
      `earliest_data_available` design) and [[0114]] (the `close_usd = 0`
      repair that AC 6's USDC spot-check would otherwise have failed against)
      are both completed. Re-confirm the frontier from the data before trusting
      any of this — the two traps recorded below are exactly the ones that
      produced a wrong reading of this measurement before.
---

# Tranche 2 backfill-depth gate

## Summary

Tranche 2 has two backfill-shaped acceptance criteria:

- **AC 5** — `GET /backfill/status` shows `earliest_data_available` ≤
  **2022-01-01**
- **AC 6** — `GET /assets/USDC.../ohlcv?timeframe=all` returns data points from
  at least January 2022, *"with correct 1d candles verifiable against known USDC
  price history (spot-check dates provided by reviewer)"*

The §9 milestone text asks for roughly **January 2022 → present**: 4+ years of
SDEX history covering the whole Soroban era plus two pre-Soroban years.

This task is the **gate and the proof**, not the run. The run is [[0088]].

## Context

**Where the run stands.** The SDEX pre-Soroban tail backfill is active and on
rate — walking **up** from genesis, with pass 1 due around 2026-07-27. Because
it walks upward from ledger 1, reaching the 2022-01-01 bar is a matter of the
run completing its passes, not of starting new work. Confirm the current
frontier against 0088 before scheduling anything here.

**⚠️ Two traps recorded in prior sessions, both of which produced a wrong
reading of exactly this measurement:**

1. `earliest_data_available` is **stored, not computed** (§4.5: *"recorded by
   the push step … not computed live via `MIN(timestamp)`"*, task 0106). A row
   can exist for 2021 while the status endpoint still reports 2024 because the
   stored value was never advanced. **Check the endpoint and the table, and
   reconcile them** — an endpoint-only check can pass while the data is absent,
   or fail while the data is present.
2. **Marker rows survived a cleanup that candles did not** (recorded during the
   0088 run). Progress markers therefore *overstate* coverage. Verify against
   actual `price_ohlcv_*` rows, never against markers alone.

**⚠️ Coarse-table USD coverage.** [[0114]] found `close_usd = 0` across
86–100% of coarse rows for large historical spans, and the repair is mid-flight.
AC 6 asks for candles *"verifiable against known USDC price history"* — if the
USD column is still zero for the 2022 span, the spot-check fails for a reason
that has nothing to do with backfill depth. **Confirm 0114's repair covers the
2022 window before scheduling the spot-check**, and note that a USDC-quoted
candle needs no USD reference at all (stablecoin-direct tier), which makes USDC
the most favourable possible subject.

**⚠️ Doc drift.** §9's Tranche 2 text still describes `sdex-cloud-push` cycles.
Per **ADR 0009** that step no longer exists — the backfill CLI writes directly
to Hetzner. `last_push_at` survives as a real column meaning "most recent direct
write". Correct §9 as part of this task so the reviewer is not sent looking for
a tool that was retired.

## Implementation

- Reconcile the three views of depth and record all three:
  `GET /backfill/status.sdex.earliest_data_available`, the stored
  `backfill_progress` row, and an actual `MIN(timestamp)` over
  `price_ohlcv_1d` / `_1h` for SDEX-sourced rows.
- Fix the stored value if it lags reality (the O(1)-read design is right; a
  stale value is a bug in the advance path, not a reason to compute live).
- **Continuity, not just depth.** `earliest_data_available ≤ 2022-01-01` is
  satisfied by a single old row. Verify the span is actually *covered*: candle
  counts per month from 2022-01 forward, with any gap explained. A reviewer
  spot-checking a random date inside a hole gets a worse impression than one
  told about the hole up front.
- Choose and document spot-check dates for USDC (and ideally XLM) 1d candles,
  and compare close prices against an independent public source. Record source,
  dates, our value, their value, and the delta. Expect and explain small
  differences: our candles are SDEX/AMM venue prices, not a global composite.
- Verify through the **public API** (`?timeframe=all`), not just SQL — AC 6 is
  an API-level claim, and `timeframe=all` auto-selects `1d` granularity (§4.2),
  which is the path a reviewer will take.
- Confirm `backfill_note` appears correctly while the backfill is still running
  (§4.2) — it is the honest signal that history is partial.

## ⚠️ Two ACs are blocked by [[0170]] — no amount of backfill fixes them

> 🗄️ **SUPERSEDED 2026-09-04 — [[0170]] is completed and this section is
> falsified by measurement.** `GET /assets/{USDC}/ohlcv?timeframe=all` no
> longer returns `data: []`; it returns 2,042 daily points from 2021-02-01.
> Kept for the mechanism it records, which is still the reason USDC has no
> real candles. What replaced the empty response is a *derived* one — see
> the Findings below, which is the section to act on.

Established 2026-08-10, from code plus [[0165]]'s existing prod measurement:
**`GET /assets/{USDC}/ohlcv` returns `200 OK` with `data: []` in every mode**, at
every timeframe, at any backfill depth.

`/ohlcv` resolves the path asset as the **base** leg and a separate **quote** leg
from `base_currency`, then filters on both (`queries_ch.rs:545`). `base_currency`
defaults to `USD` → USDC, so the default request asks for a **USDC/USDC
self-pair**. `?base_currency=XLM` fails too: canonicalisation stores that pair as
base=XLM / quote=USDC and never the inverse. 0165 measured `price_ohlcv_1d WHERE
asset_id = <USDC>` → **0 candles**, unconditional on quote.

**AC 3 and AC 4 below are unsatisfiable until [[0170]] ships**, and [[0165]] does
**not** cover it — 0165 rewrites the `price_usd_series` view, which `/ohlcv` never
reads. Do not schedule this task off the back of [[0088]] finishing; that
unblocks AC 1–2 only.

## Findings — reconciliation pass (2026-09-04)

All read-only. Control plane and the deployed API read with the free-tier key
(`pricing-api-free-production`, 1 req/s), a handful of `curl`s; ClickHouse read
by the operator on prod. No load, no writes.

### ✅ AC 1 — the three views agree, and AC 5 of Tranche 2 passes

| view | oldest SDEX data |
|---|---|
| `GET /backfill/status` → `sdex.earliest_data_available` | `2015-11-18T03:47:00Z` |
| stored `prices.backfill_progress` row (`FINAL`) | `2015-11-18 03:47:00` |
| **actual `min(timestamp)`, `price_ohlcv_1d`, `source='sdex'`** | **`2015-11-18 00:00:00`** (the day bucket of that minute) |
| oldest active partition, **every** `price_ohlcv_*` tier | **`201511`** |

The bar is 2022-01-01. We clear it by **six years**, and the depth is not thin:

| year | 1d SDEX candles | assets |
|---|---|---|
| 2021 | 914,679 | 41,829 |
| **2022** | **4,048,196** | **82,096** |

🔑 **Both traps the task warns about were checked and neither fired.** That is
worth stating as a result rather than an absence:

1. **The stored value is a monotonic high-water mark and can never move
   forward.** `sink.rs:229` merges it with `merge_min` — *"Monotonic window:
   never narrow what a prior run already recorded."* So a deletion would leave
   it untouched and the endpoint would keep asserting coverage it no longer has.
   The task warns the stored value can *lag* reality; it can also **overstate**
   it, and that is the direction that fails a reviewer. Here it does not: real
   rows corroborate it on the minute.
2. **Markers outliving their data** is ruled out independently by the partition
   census — `201511` is the oldest *active* partition on all seven tiers, so
   the 2015 rows are physically present, not implied by a progress row.

### 🔴 AC 3/AC 4 — [[0170]] shipped, but what it unblocked is *derived*, not measured

**The blocker section above is now falsified and must not be read as current.**
It states `GET /assets/{USDC}/ohlcv` returns `200 OK` with `data: []` in every
mode. Measured 2026-09-04, it returns **`200` with 2,042 daily points spanning
2021-02-01 → today**. [[0170]] is completed and the self-pair problem is gone.

What came back is a different problem, and a worse one for AC 4:

| | USDC | `native` (control) |
|---|---|---|
| points, `timeframe=all` | 2,042 | 2,414 |
| span | 2021-02-01 → 2026-09-04 | 2018-05-15 → 2026-09-04 |
| **points with `trade_count > 0`** | **0** | **2,414 (all)** |
| points with `volume_base > 0` | 0 | all |
| `derived: true` | **all 2,042** | — |
| distinct `close` values | 177 — **1,865 of them exactly `1`** | real market values |
| method mix | `peg` 1,864 / `oracle` 178 | — |

**Not one USDC candle has a trade behind it.** 2021 through 2025 is 100%
`method: peg` — a flat, asserted `1`. Only from mid-2026 does `oracle` produce
varying values, and those still carry `trade_count: 0`.

The `native` control is what makes this specific rather than general: real
candles with real trade counts back to 2018, so the pipeline is fine. It is
**structural to USDC** — it is our top-preference *quote* asset, so it
essentially never appears as a base, so there are no candles, so the peg fill
covers the whole history. Same root cause as [[0165]].

#### 🔴 The date that fails us

Tranche 2 AC 6 asks for *"correct 1d candles verifiable against known USDC price
history — **spot-check dates provided by reviewer**"*. **We do not choose the
date.** The most famous date in USDC's price history is **2023-03-11**, when it
broke its peg to roughly **$0.87-0.88** after the Silicon Valley Bank failure —
precisely the date a reviewer who knows the space would pick, because it is the
one that distinguishes a price feed from an assumption.

```
2023-03-10  close=1  method=peg  trades=0
2023-03-11  close=1  method=peg  trades=0   ← reality was ~$0.87
2023-03-12  close=1  method=peg  trades=0
```

`native` on the same day shows the market genuinely moving
(`0.0782 → 0.0588 → 0.0836`, 36,208 trades), so the underlying data exists — we
simply do not use it for USDC. **We would return `1` no matter what the truth
was.** That is not a defect in the peg fill; it is what a peg fill *is*. But it
means the word *"correct"* in AC 6 is not something we can promise for a
reviewer-chosen date, and the mitigation already sketched in AC 4 below
(spot-check non-peg assets, state why USDC is excluded) is now the **likely**
path rather than the fallback.

⚠️ Do not present this as "USDC pricing is broken" — for ~99% of dates the
answer is right. The claim is narrower and must stay narrow: **it is right by
construction rather than by measurement, and on at least one well-known date it
is wrong.**

### ⚠️ `GET /backfill/status` contradicts itself on the endpoint AC 5 points at

The number the reviewer is asked to check is correct. Everything printed beside
it says the backfill never started:

```json
"sdex": { "status": "completed", "progress_pct": 0.0,
          "ledgers_remaining": 63795748, "current_ledger": 1, "start_ledger": 1 }
```

`progress_pct` (`backfill/handlers.rs:72`) computes
`(current - start) / (target - start)` and its doc comment asserts
*"`current_ledger` advances upward from `start_ledger`"*. The SDEX archive
stream walks **downward** to genesis — `sink.rs`'s `resolve_current`: *"a
forward stream keeps the max, a backward stream the min"*. At `current_ledger:
1` the stream is finished, and the formula reads that as 0%. `ledgers_remaining`
is wrong for the same reason.

Cheap to fix and worth fixing before submission: a completed stream reporting
**0.0% and 63.8 M ledgers remaining** is the kind of thing a reviewer stops on.

⚠️ Also visible on the same response: `soroban_amm` is `status: "running"` with
`last_push_at` of **2026-07-14** — seven weeks stale — at 63,352,611 of
63,475,475. Not AC 5's subject, but it is on the same reviewer-facing payload.

### ⚠️ USD coverage in the historical span is thin, and thinnest where it is oldest

`close_usd = 0` on `price_ohlcv_1d`, SDEX, by year:

| year | candles | `close_usd = 0` | share |
|---|---|---|---|
| 2015 | 5 | 5 | 100% |
| 2016 | 66 | 66 | 100% |
| 2017 | 2,025 | 2,025 | 100% |
| 2018 | 29,003 | 28,982 | 99.93% |
| 2019 | 67,839 | 67,774 | 99.90% |
| 2020 | 66,944 | 64,338 | 96.11% |
| 2021 | 914,679 | 121,873 | 13.32% |
| **2022** | **4,048,196** | **1,465,252** | **36.20%** |

The 2015-2020 rows exist as candles but carry **no USD price at all**. That does
not endanger AC 5 (depth is depth) and it does not endanger the USDC
spot-check (peg-filled, so USD by construction). It endangers **AC 4's
"≥2 assets"**: pick a long-tail asset in 2022 and there is a ~1-in-3 chance its
daily candle has no USD close. Choose the spot-check assets from the priced
majority and say so, rather than discovering it in front of the reviewer.

### Not this task's, recorded in passing

- The **six `price_ohlcv_*_bak` tables are still on prod**, unchanged at
  296,740,338 rows across `202402`-`202607`. Row counts match [[0177]]'s table
  exactly, so nothing has moved since it was filed. Already owned by [[0105]]
  and [[0177]] — no new task, but the shared-disk cost is still being paid.
- `newest_data_available` reads `2026-07-06 09:35:00` on **both** streams while
  `price_ohlcv_1d` holds rows through 2026-09-04. Consistent, not a defect:
  `backfill_progress` is written by the backfill, and the live ingest path does
  not touch it. It is deliberately not surfaced by the API (`dto.rs`).

## Acceptance Criteria

- [x] `GET /backfill/status` reports `sdex.earliest_data_available` ≤
      2022-01-01, and the stored value is reconciled against real candle rows —
      **done 2026-09-04**. `2015-11-18T03:47:00Z` on the endpoint, the same in
      the stored `backfill_progress` row, `min(timestamp) = 2015-11-18` in
      `price_ohlcv_1d`, and `201511` as the oldest active partition on all seven
      tiers. Six years under the bar. Both of the task's stated traps were
      checked and neither fired.
- [ ] Month-by-month candle counts from 2022-01 to present recorded; every gap
      explained or filled
- [x] ~~🔴 BLOCKED BY [[0170]]~~ — `GET /assets/{USDC}/ohlcv?timeframe=all`
      returns 1d candles from 2022-01 or earlier through the deployed API —
      **satisfied 2026-09-04**: 2,042 points from **2021-02-01**, no gaps.
      ⚠️ Ticked on its literal wording only. Every one of those points is
      `derived: true` with `trade_count: 0`, and 1,865 of them are exactly `1`.
      This criterion asks whether candles are *returned*; AC 4 below asks
      whether they are *right*, and that is where the peg fill bites.
- [ ] ~~🔴 BLOCKED BY [[0170]]~~ (USDC is the reviewer's named example) —
      spot-check table published: ≥5 dates × ≥2 assets, our close vs an
      independent public source, deltas explained. **Unblocked, but now at risk
      for a different reason** (2026-09-04): USDC's series is peg-derived, so it
      is right by construction rather than by measurement, and on **2023-03-11**
      — the SVB depeg, when USDC traded ~$0.87-0.88 — we return exactly `1`.
      The reviewer picks the dates. The mitigation below is therefore the
      **likely** path, not the fallback: *spot-check two non-peg assets and
      state explicitly why USDC is excluded — a weaker package, since USDC is
      the easiest close for a reviewer to verify independently.* 🔑 Choose those
      assets from the USD-priced majority; ~36% of 2022 daily candles carry
      `close_usd = 0`.
- [ ] USD-denominated fields are non-zero for the spot-checked span (depends on
      [[0114]]'s repair reaching it) — or the limitation is stated explicitly.
      **Measured 2026-09-04**: `close_usd = 0` on **36.20%** of 2022 daily SDEX
      candles, 13.32% of 2021, and **96-100% of 2015-2020**. The span is
      covered; the *estate within it* is not uniformly priced. Open until the
      spot-checked assets are chosen and their own coverage confirmed.
- [ ] `backfill_note` present and accurate while `sdex.status = "running"` —
      ⚠️ **the precondition no longer holds**: `sdex.status` is `completed`
      (`completed_at` 2026-07-27). Re-read this as "the note is correct for a
      completed stream", or retire it. 🔴 Separately, the same payload publishes
      `progress_pct: 0.0` and `ledgers_remaining: 63795748` beside
      `status: "completed"` — the formula assumes a forward walk and this stream
      ran backward. Fix before submission; it is on the exact endpoint Tranche 2
      AC 5 sends the reviewer to.
- [ ] §9's residual `sdex-cloud-push` language corrected per ADR 0009
- [ ] `sdex.last_push_at` fresh within the configured cadence at review time —
      currently **2026-08-11**, 24 days ago, on a stream that completed
      2026-07-27. Decide whether freshness is even meaningful for a completed
      archive stream before asserting anything about it in [[0128]].

## Design Decisions

### Emerged

1. **🔒 `docs/scf/milestone-1-evidence.md` is left exactly as submitted — no
   correction, no annotation.** Decided with the operator 2026-09-04.

   The finding is real and is not in dispute: Figure 7 captures a live
   `/backfill/status` response under the **old forward formula** —
   `current_ledger: 50457424`, `progress_pct: 79.47`,
   `ledgers_remaining: 13032807` — and the prose beneath reads *"The `sdex`
   stream is ~79 % through the chain."* Under the arithmetic PR #283 corrects,
   the same row is **20.53%**, and the corrected number is the truthful one:
   `current_ledger` sat at the Soroban activation boundary, so real coverage was
   `[50457424, 63490231]`. The same paragraph also calls
   `earliest_data_available` *"the public archive floor … not the ingested
   depth"*, which `backfill/dto.rs` records as a corrected misreading.

   **It stays as it is.** M1 is submitted and accepted; the document is a record
   of what was sent, not a live description of the system. Editing submitted
   evidence after the fact changes what the record says was claimed, which is a
   worse failure than the overstatement it would fix.

   🔴 **Do not "helpfully" annotate this file in a later session.** A future pass
   re-reading Figure 7 against a corrected endpoint will find the same
   discrepancy and reach for the same fix; this entry is the answer. The place
   to be accurate is [[0128]], the M2 package, which is not yet written.

   ⚠️ **Consequence to carry into [[0128]]**: re-running that same `curl` for an
   M2 exhibit will show a number that looks like a regression against M1's
   Figure 7 (79.47 → 20.53, then 100 once the archive completed). It is not —
   it is the same row read correctly. If M2 reproduces this endpoint, say so
   in one line rather than letting a reviewer discover an unexplained
   discontinuity between the two packages.


## Notes

- Do **not** let this task drive the backfill run itself — that is [[0088]],
  which has its own ETA and its own operator cadence. This is the checkpoint.
- [[0101]] (live-era AMM reprice gap: a Soroswap 9-day hole and Phoenix ~2%
  shortfall) affects AMM completeness in 2026, not the 2022 span. It does not
  block AC 5/6, but it should be closed before [[0128]] claims full coverage.
