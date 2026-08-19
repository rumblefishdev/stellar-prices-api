---
id: "0208"
title: "Canonical USDC is unlisted, unpriced and has no OHLCV — volume and price attribute only to the base asset of a pair"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0120", "0207", "0172"]
tags: [layer-backend, priority-high, effort-large, milestone-M2, api, pricing, defect]
milestone: 2
links:
  - "../../../tools/scripts/conformance-assets.json"
history:
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from the 0120 conformance run. USDC is a §9 Tranche 1 named
      major; AC 1 cannot fully pass while this stands.
---

# Quote-side majors are invisible: canonical USDC unlisted and unpriced

## Summary

Circle's canonical USDC (`USDC:GA5ZSEJY…K4KZVN`) on production:

- `GET /v1/assets/{id}` → 200, resolves fine (`is_active: true`)
- `GET /v1/assets?search=USDC` → 14 impostor USDCs, canonical absent
- `GET /v1/assets/{id}/price` → **404 `no current price`**
- `GET /v1/assets/{id}/ohlcv` (7d/1h and 30d/1d) → **0 buckets**

The store attributes volume, candles and current price to the **base** asset
of each pair only. USDC is the quote in nearly every pair it trades in, so the
most liquid asset on Stellar is invisible in the listing and unpriceable —
while 14 low-volume impostors (one with a vanity issuer suffix `…KQKZVN`
mimicking the real `…K4KZVN`) are listed.

## Context

Found by [[0120]] list derivation; USDC is one of the six §9 Tranche 1 named
majors, so Tranche 2 AC 1 ("correct responses for 20 major assets") cannot
fully pass until quote-side majors are servable. XLM's thin `volume_24h_usd`
($176k) is the same effect — its quote-side volume is not attributed.

## Implementation

- Confirm the attribution model in the candle/current-price pipeline
  (base-only vs base+quote).
- Design: either mirror pairs (attribute to both legs), or synthesize
  quote-side rows for current prices/volume (a USD-stable's price can also come
  from the peg/oracle arm).
- Mind [[0172]]/[[0182]] — quote-side attribution interacts with the USDT peg
  bug family.
- Re-run the 0120 suite: USDC's `returns 200` price check and non-empty OHLCV
  windows must pass, and canonical USDC must appear in the paginated listing.

## Acceptance Criteria

- [ ] `GET /v1/assets/{USDC canonical}/price` returns a current price ≈ 1.00
- [ ] Canonical USDC appears in `GET /v1/assets` (searchable, ranked by its
      real volume)
- [ ] USDC OHLCV non-empty for recent windows
- [ ] 0120 suite passes for USDC
