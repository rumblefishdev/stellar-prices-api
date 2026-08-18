---
id: "0120"
title: "Endpoint conformance — all 7 route groups return correct, schema-valid responses for 20 major assets"
type: TEST
status: active
related_adr: ["0008"]
related_tasks: ["0072", "0118", "0119", "0124", "0128"]
tags: [layer-backend, priority-high, effort-medium, milestone-M2, api, testing, verification, acceptance]
milestone: 2
links:
  - "../../../packages/prices-api/src/lib.rs"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Owns Tranche 2
      acceptance criterion 1 — the only AC that covers the full public API
      surface, which M1 deployed but never verified beyond
      `GET /backfill/status`.
  - date: 2026-08-18
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active — starting the endpoint conformance pass.
---

# Endpoint conformance for 20 major assets

## Summary

Tranche 2 AC 1: *"All 7 endpoint groups return correct, schema-valid responses
for at least 20 major assets."*

M1 deployed the whole route surface but verified only `GET /backfill/status`
(stated plainly in `milestone-1-evidence.md` Table 4). This task is the
verification pass that turns "routed" into "correct".

## Context

Two distinct claims hide inside AC 1, and they need separate evidence:

- **Schema-valid** — the response matches the documented shape in §4.1–§4.5
  and the generated OpenAPI spec. Mechanically checkable.
- **Correct** — the numbers mean what they claim. Not checkable against the
  spec; needs either an independent source or an internal cross-check.

This task owns both, with the numeric-correctness half deliberately narrow:
deep VWAP reconciliation is [[0123]] and historical spot-checking is [[0127]].
Here the bar is *"no field is a stub, a sentinel, or obviously wrong"*.

**Dependency:** several §4 response fields are still stubs until [[0072]] lands
(`price_xlm`, `change_24h_pct`, `change_7d_pct`, `sources` are all at their
table DEFAULTs). Running this suite before 0072 will correctly fail. Sequence
after 0072 and [[0119]].

## Implementation

- **Fix the asset list.** Pick 20 named assets and record them in the task, not
  in a shell variable — the same list must be reusable by [[0121]], [[0123]],
  [[0127]] and the [[0128]] evidence package. Seed from §9's Tranche 1 list
  (XLM, USDC, EURC, AQUA, BTC, ETH) and extend with the highest-volume assets
  the store actually holds. Include at least one asset per identifier form:
  `native`, `CODE:ISSUER`, and a `C…` contract address.
- **Exercise all 7 groups** per asset: `GET /assets`, `GET /assets/{id}`,
  `GET /assets/{id}/price`, `GET /assets/{id}/ohlcv`, `POST /prices/batch`,
  `GET /oracles/{id}`, `GET /backfill/status`.
- **Schema-validate** each response against the generated OpenAPI spec rather
  than hand-written assertions, so the spec and the tests cannot drift apart.
- **Sanity assertions** beyond the schema:
  - no documented field is absent, and no field is left at its zero/empty
    sentinel for an asset that should have a value
  - numeric strings parse; `Decimal(38,14)` precision survives the JSON
    round-trip (values are serialised as strings by design — §3.3)
  - OHLCV invariants hold: `low ≤ open,close ≤ high`, timestamps strictly
    increasing and aligned to the requested granularity, no duplicate buckets
  - `GET /assets` pagination: walking the cursor to exhaustion yields every
    asset exactly once, and `has_more` is accurate at the boundary (extends
    the 0074 250-row pagination test to the M2 asset set)
  - `POST /prices/batch` returns the same numbers as the per-asset `/price`
    calls for the same assets in the same window
- **Run against production** over the real gateway with an API key — this is an
  acceptance check, not a unit test. Keep it a scripted, re-runnable artifact so
  [[0128]] can cite a fresh run.

## Acceptance Criteria

- [ ] The 20-asset list is fixed, documented in-task, and covers all three
      identifier forms
- [ ] All 7 route groups exercised for every asset; every response validates
      against the OpenAPI spec
- [ ] No documented response field is a stub/sentinel for a liquid asset
- [ ] OHLCV invariants asserted (OHLC ordering, bucket alignment, no dupes)
- [ ] Cursor pagination on `GET /assets` proven exhaustive and duplicate-free
- [ ] `POST /prices/batch` agrees with per-asset `/price` for the same assets
- [ ] Suite is re-runnable and its output is citable evidence for [[0128]]
- [ ] Any defect found is fixed or spawned as its own task — a documented
      failure list is not a pass

## Notes

- Deliberately **not** the Tranche 3 "integration test suite runs in CI"
  deliverable. This is an acceptance pass against the deployed API; wiring an
  equivalent suite into GitHub Actions is M3.
- Expect `sources` to name **Aquarius** here — that is where §9's "Aquarius
  appearing as a named source in VWAP" bullet is actually observed. If it does
  not appear, the cause is [[0072]] (column not written) or [[0080]]
  (concentrated pools not extracted), not this task.
