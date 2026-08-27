---
id: "0225"
title: "GET /ohlcv returns an empty 200 for actively-trading assets that do not trade against the default USDC quote — 12 of 13 remaining 0120 failures"
type: BUG
status: completed
related_adr: []
related_tasks: ["0120", "0170", "0128", "0210"]
tags: ["priority-high", "effort-medium", "api", "read-surface", "data-correctness", "scf", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/handlers.rs"
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
history:
  - date: 2026-08-25
    status: backlog
    who: stkrolikiewicz
    note: >
      Found by the [[0120]] conformance re-run after [[0135]] landed. 0120
      blocks on 0135/0170/0178 and expected to go green once they were done —
      it will not. 12 of the 13 remaining failures are this defect, and it is
      owned by none of the three. Measured on prod the same session.
  - date: 2026-08-26
    status: active
    who: okarcz
    note: >
      Reassigned from the [[0120]] owner and activated, at their request, after
      review showed this is the SAME defect as [[0170]]'s current scope rather
      than a distinct one.
      🔑 The "Do not fold this into 0170" note below is correct against 0170's
      TITLE, which still describes only the USDC self-pair, but not against its
      SCOPE. 0170 was widened on 2026-08-19 (by this task's own author) to "any
      asset that never traded against canonical USDC", measured on 2026-08-25 at
      **20,481 assets**, and already carries the acceptance criterion "a non-empty
      USD series for the five 0120 majors (CBIJ…, RON, EQL, BOL, AUD)" — the same
      five assets, the same trigger, the same remedy.
      ⚠️ This is the SECOND duplicate spawn of the same defect from the 0120
      conformance runs; 0170's 2026-08-19 entry records the first, retired the
      same day. The root cause is 0170's stale title, corrected in the same PR.
      Sequencing: this task does NOT get its own implementation. [[ADR-0011]]'s
      denomination contract, implemented in 0170, serves this population as a
      side effect. This stays active as the verification vehicle — its ACs are
      the consumer-facing check on 0170's change and its 0120 re-run is the
      regression gate. Close it on 0170's evidence, not on separate work.
      Design question below RESOLVED by ADR 0011: option 3 (convert through a USD
      denomination). Options 1 and 2 are not taken.
  - date: 2026-08-27
    status: completed
    who: okarcz
    note: >
      CLOSED on [[0170]]'s implementation, as designed — this task carried the
      consumer-facing verification and no separate code. Verified through the
      deployed API after the 08-27 batch deploy: AUD 23 buckets, RON 14, BOL
      12, EQL 11, `CBIJ…` 30, every priced bucket labelled `traded`. The
      [[0120]] suite re-run shows no asset returning an empty window (13 did
      before); all 27 remaining failures attributed to [[0229]], [[0230]] and
      [[0178]]. ⚠️ EQL's one unpriced bucket is the arm this task exists for:
      traded but not yet priceable, returned present with null prices —
      a distinction an empty 200 cannot make.
---

# `/ohlcv` reports "no candles" for assets that are trading right now

## Summary

`GET /assets/{id}/ohlcv` filters on **both** legs and defaults the quote to
USDC. An asset that trades actively against **XLM** but not against USDC
therefore gets a `200 OK` with `data: []` — indistinguishable from an asset
that has never traded.

This is **not** [[0170]]. 0170 is the USDC *self-pair* (USDC is never a base).
Here the base leg is present and busy; the requested quote is simply not where
the asset trades.

## Measured on prod, 2026-08-25

The conformance suite asks for `?granularity=1h` over 7 days and `1d` over 30
days, with no `base_currency`, so the quote resolves to USDC. Candle counts in
`price_ohlcv_1d`:

| asset | vs XLM | vs USDC |
|---|---|---|
| AUD | **4,864**, current to today | 1,214, newest **2026-05-20** |
| BOL | **2,085**, current to today | 12, newest **2023-10-27** |
| RON | **203**, current to today | none |
| EQL | **9**, current to today | none |

All four are present as a base leg (7–13 distinct pairs each), so the
`asset_id` filter matches plenty of rows. Only the `quote_asset_id = <USDC>`
conjunct empties the result. The `CBIJBDNZ…` contract asset shows the same
shape at 274 base / 492 quote appearances.

## Why the empty 200 is the wrong answer

The handler already contains the precedent, in [[0170]]'s own citation of
`handlers.rs`:

> its absence is a server-side data gap, not "no candles". Surface it as a 503
> instead of masking it as an empty 200 (which looks like a healthy asset with
> no history).

Same defect, different trigger. A consumer charting AUD sees "no data" for an
asset with 4,864 daily candles.

## Blast radius

- **[[0120]]** — 12 of its 13 remaining failures, and the reason its stated
  unblock plan ("re-run green after 0135/0170/0178") does not hold. 0135 is
  archived and the run still fails.
- **[[0128]]** — the M2 evidence package cites that run.
- Any consumer charting an asset whose liquidity is on XLM rather than USDC,
  which on this network is the common case, not the exception.

## Design question — SETTLED 2026-08-25 by [[ADR-0011]]

✅ **Option 3 was taken**, as the general contract for both read surfaces rather
than a choice made inside this endpoint: `base_currency` is a **denomination**,
not a quote-leg pair filter. Options 1 and 2 are explicitly not taken — 1 makes
the response's meaning data-dependent, and 2 leaves a charting consumer with no
series for an asset that is trading right now.

The conversion needs no `usd_reference` join in the dominant case: `close_usd` is
already on the candle row at 99.9% coverage, so the per-bucket rate derives
in-table as `close_usd / close`. That rate is **measured, not pegged** — it
wobbles 0.9976-1.0008 — which is what makes the denomination reading correct
rather than merely more useful.

Recorded below as raised, for the reasoning that led here:

1. **Fall back to the asset's most-liquid quote** and say so in the response.
   Most useful, but makes the response's *meaning* data-dependent — the same
   objection 0170 raises against inverting some assets and not others.
2. **Keep the empty result, change the status.** A 404/503 with a body naming
   the quotes that DO have candles. Honest, cheap, and pushes the choice to
   the caller.
3. **Convert through `usd_reference`** so a USD-quoted series exists for any
   asset with any priced pair. Most work, and it overlaps [[0170]]'s peg-fill
   arm — worth checking whether one implementation serves both.

Whichever is chosen must state what `/ohlcv` returns *in* (§ the endpoint
returns OHLC in the quote asset), because options 1 and 3 change that.

## Acceptance Criteria

- [x] Decision recorded with reasoning, including what the response is
      denominated in
      → [[ADR-0011]], accepted 2026-08-25: `base_currency` **denominates**, it
      does not filter a quote leg. The response echoes `base_currency`, and
      every candle carries `method` and `derived` so a consumer can tell a
      measured rate from a placeholder and an exact close from a derived
      extreme.
- [x] `GET /assets/{AUD}/ohlcv` over a recent window returns a non-empty,
      correctly-denominated series **through the deployed API**
      → **23 buckets**, all `traded`, 30-day window, on
      `AUD:GBBWRCJSZR…` — the issuer pinned, since 15 AUD issuers exist.
      Denominated, not filtered: AUD has no USDC leg at all, so a pair filter
      returns nothing here by construction.
- [x] The same verified for an asset with NO USDC pair at all (RON or EQL)
      → RON **14 buckets**, EQL **11** (10 priced + 1 unpriced), BOL **12**,
      and the top soroban asset `CBIJ…` **30**, all through the deployed API.
- [x] An asset that genuinely never traded is still distinguishable from this
      case — the whole point; assert both
      → `ohlcv_never_traded_is_distinguishable_from_unrepresentable` and
      `ohlcv_unpriced_bucket_is_returned_with_price_fields_absent` assert both
      arms. Confirmed on prod from the other side: EQL's
      `2026-08-25T11:00:00Z` bucket comes back **present with null prices and
      `volume_base: 1`, `trade_count: 2`** — traded but not yet priceable,
      which an empty series could never express. ⚠️ The prod half evidences the
      *unrepresentable* arm; the never-traded arm rests on the tests, as no
      never-traded asset is in the 0120 fixture set.
- [x] ~~A normal USDC-quoted asset's response is byte-identical to today's~~ —
      **RESTATED, not ticked as written**, in step with [[0170]], which carries
      the same wording and the full reasoning.
      → [[ADR-0011]] §4 puts `method` and `derived` on every candle, so no
      response can be byte-identical to a pre-ADR one. Restated as: **identical
      values in every field that existed before**, differing only by the two
      additive fields. Verified by `ohlcv_merges_sources_and_notes_backfill`,
      whose expected numbers are unchanged from the pre-0170 fixture.
      ⚠️ Additive, so a consumer reading only the old fields is unaffected —
      which is what makes this a restatement rather than a quiet pass. A
      shape change that broke existing readers would have made "no regression"
      false, and the criterion should then have FAILED rather than been reworded.
- [x] [[0120]]'s ohlcv failures drop to 0 on a re-run, or the remainder is
      attributed to a named task
      → Re-run 2026-08-27 against the deployed API: **893 pass, 27 fail, 8
      skip**, and `window is non-empty for a liquid asset` passes for **all 20
      assets** where 13 failed before. Every remaining failure is attributed:
      17 × `decimal strings` → [[0230]] (the suite predates ADR 0011 §5's
      unpriced buckets, and flakes with enrichment lag), 9 × `low <= open,close
      <= high` → [[0229]] (real: derived extremes round past an exact close),
      1 × `/price` 404 for USDC → [[0178]] (the third USDC surface).

## Notes

- ⚠️ **RETRACTED 2026-08-26 — "do not fold this into [[0170]]" was written
  against 0170's TITLE, not its scope.** The title still describes only the USDC
  self-pair; the task was widened to this exact population on 2026-08-19 and
  measured at 20,481 assets on 2026-08-25, and already carries an AC naming these
  same five majors. Same root cause, same remedy, one ADR.
  What survives of the original concern: the two cases must stay separately
  *testable*. They are — 0170 keeps the self-pair assertions, this task keeps the
  XLM-only-quoted assertions, and both run against one implementation.
- The `CBIJBDNZ…` row also carries [[0210]]'s empty `asset_code`, visible in
  the probe output as a blank code column. Different defect, same asset.
