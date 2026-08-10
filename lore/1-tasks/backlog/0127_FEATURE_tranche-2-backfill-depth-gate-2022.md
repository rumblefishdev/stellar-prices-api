---
id: "0127"
title: "Tranche 2 backfill-depth gate — earliest_data_available ≤ 2022-01-01 and USDC 1d candles spot-checked"
type: FEATURE
status: backlog
related_adr: ["0005", "0009"]
related_tasks: ["0088", "0106", "0114", "0116", "0101", "0128", "0170", "0165"]
tags: [layer-indexing, priority-high, effort-medium, milestone-M2, backfill, sdex, verification, acceptance]
milestone: 2
links:
  - "../../../lore/1-tasks/active/0088_FEATURE_soroban-backfill-run-tracker/README.md"
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

## Acceptance Criteria

- [ ] `GET /backfill/status` reports `sdex.earliest_data_available` ≤
      2022-01-01, and the stored value is reconciled against real candle rows
- [ ] Month-by-month candle counts from 2022-01 to present recorded; every gap
      explained or filled
- [ ] 🔴 **BLOCKED BY [[0170]]** — `GET /assets/{USDC}/ohlcv?timeframe=all`
      returns 1d candles from 2022-01 or earlier through the deployed API
- [ ] 🔴 **BLOCKED BY [[0170]]** (USDC is the reviewer's named example) —
      spot-check table published: ≥5 dates × ≥2 assets, our close vs an
      independent public source, deltas explained. *Mitigation if 0170 slips:
      spot-check two non-peg assets and state explicitly why USDC is excluded —
      but that is a weaker package, since USDC is the easiest close for a
      reviewer to verify independently.*
- [ ] USD-denominated fields are non-zero for the spot-checked span (depends on
      [[0114]]'s repair reaching it) — or the limitation is stated explicitly
- [ ] `backfill_note` present and accurate while `sdex.status = "running"`
- [ ] §9's residual `sdex-cloud-push` language corrected per ADR 0009
- [ ] `sdex.last_push_at` fresh within the configured cadence at review time

## Notes

- Do **not** let this task drive the backfill run itself — that is [[0088]],
  which has its own ETA and its own operator cadence. This is the checkpoint.
- [[0101]] (live-era AMM reprice gap: a Soroswap 9-day hole and Phoenix ~2%
  shortfall) affects AMM completeness in 2026, not the 2022 span. It does not
  block AC 5/6, but it should be closed before [[0128]] claims full coverage.
