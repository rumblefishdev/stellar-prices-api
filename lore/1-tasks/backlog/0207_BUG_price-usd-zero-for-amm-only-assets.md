---
id: "0207"
title: "price_usd is \"0\" for every asset priced only by AMM sources — 17 of the 20 conformance majors, including XLM and EURC"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0120", "0072", "0208", "0209"]
tags: [layer-backend, priority-high, effort-medium, milestone-M2, api, pricing, defect]
milestone: 2
links:
  - "../../../tools/scripts/conformance-0120.mjs"
history:
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from the 0120 conformance run (report
      conformance-0120-report-2026-08-19T0810.json).
---

# price_usd is "0" for AMM-only assets

## Summary

`GET /v1/assets/{id}/price` and the `/v1/assets` list return
`price_usd: "0"` for every asset whose `sources` contain no `sdex` (and, with
two soroban exceptions, no `aquarius`) entry — while `vwap_24h` for the same
row is populated and correct. Measured on production 2026-08-19: 88 of the
top-200 volume rows, and 17 of the 20 fixed conformance assets, including
**XLM** (`vwap_24h` 0.157, `price_usd` "0") and **EURC** (1.153 / "0").

## Context

Found by [[0120]]'s derivation probes and confirmed by the suite run. The 0072
runbook's rollout gate (`sources` non-empty) passes while the headline price is
still the zero sentinel, so this slipped through the 0072 verification. The
pattern is exact on the top-200 sample: `price_usd > 0` ⟺ the asset has an
`sdex` source (110 rows) or is one of 2 aquarius-priced soroban rows; all 88
zero rows lack sdex.

## Implementation

- Locate where `price_usd` is materialized (0072's current-prices MV) and why
  AMM (phoenix/soroswap, mostly aquarius) trades do not feed it.
- Decide the fix: feed AMM closes into the same column, or fall back to
  `vwap_24h` when the last-trade price is absent. Document the chosen
  semantics in §4.
- Re-run `npm run conformance:0120` — the 17 `price_usd`/zero-sentinel
  failures must go green.

## Acceptance Criteria

- [ ] `price_usd` non-zero for every conformance asset with a populated
      `vwap_24h`
- [ ] Semantics of `price_usd` vs `vwap_24h` documented in §4
- [ ] 0120 suite passes the price sentinel checks
