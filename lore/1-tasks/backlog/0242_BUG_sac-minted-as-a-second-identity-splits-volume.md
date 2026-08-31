---
id: "0242"
title: "A SAC is minted as a second identity for an asset we already hold — same token under two asset_ids, with volume split across both"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0210", "0139", "0120"]
tags: [layer-backend, layer-database, priority-high, effort-medium, milestone-M2, ingest, identity, defect]
milestone: 2
links:
  - "../../../packages/prices-ingest-core/src/canonical.rs"
history:
  - date: 2026-08-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0210]]. Looking for soroban assets whose symbol could be
      derived without an RPC call, 11 of the 52 turned out to be SACs of classic
      assets already in the registry — which means they are not nameless, they
      are duplicated: the same token exists twice, under two `asset_id`s, and
      three of the pairs have candles on both sides. That is a bigger defect
      than the missing name 0210 is about, so it is split out rather than folded
      in.
---

# A SAC becomes a second identity for an asset we already have

## Summary

`AssetRegistry::resolve_sac` is a **lookup table**, not a derivation: it maps a
contract address to a classic identity only for classic assets the registry has
already interned. A SAC whose underlying asset had not been seen at mint time
therefore falls through and is interned as a fresh `AssetIdentity::Contract`,
with its own `asset_id`.

The asset is then in `prices.assets` twice — once as `('CODE','G…','')`, once
as `('','','C…')` — and every surface keyed on `asset_id` treats them as two
different assets.

## Measured on prod, 2026-08-28

11 of the 52 soroban rows have a `contract_address` equal to the `sac_address`
already stored on a classic row, i.e. they are provably the same asset:

| symbol | classic id | candles | SAC id | candles |
|---|---|---|---|---|
| XCR | 87262 | 2,127 | 1153 | **76** |
| USDM1 | 109012 | 37 | 4142 | **2** |
| POINTS | 2146 | 6,614 | 1738 | **1** |
| USDP | 3374 | 2,067 | 1739 | 0 |
| EUR | 90371 | 55 | 70913 | 0 |
| USD | 90372 | 3 | 70914 | 0 |
| ESP | 93848 | 3 | 93843 | 0 |
| WHLAQUA, FrogST, VEUR, VCHF | — | 0 | — | 0 |

**Three pairs trade on both sides.** XCR's 2,127 classic candles and 76 SAC
candles are the same token, so neither row's `volume_24h_usd` is that token's
volume and neither row's price is formed from all of its trades. The listing
shows it twice: once named with the SDEX history, once nameless with a
fraction of the activity.

The derivation needed to detect this is **already in the codebase and already
persisted** — `assets.sac_address` is populated on 207,471 classic rows by
`sac_address_of()` at intern time. Only `resolve_sac`'s direction is missing.

## Why this is not [[0139]]

0139 is the same `asset_id` appearing on several rows because `assets` is
sorted on natural identity — one id, many rows. This is the inverse: one
economic asset holding **two different ids**. They compound (a joined query can
fan out across 0139's duplicates of either identity) but the fixes are
unrelated, and neither blocks the other.

## Implementation sketch

- Make `resolve_sac` derive rather than look up, or seed the index from
  `assets.sac_address` at registry load, so a SAC mints onto its classic
  identity and no new split is created. That stops the bleeding; it does not
  heal the 11.
- Decide what happens to the existing pairs. Merging two `asset_id`s touches
  `price_ohlcv_*`, `current_prices` and every side table — closer to a data
  migration than a metadata fix, and it needs its own plan and rollback.
  Cross-referencing them instead (a pointer column, no re-keying) is the
  cheaper option and may be enough for the read surface.
- ⚠️ Worth looking at how BE models this before choosing: their
  `default.asset_sac` (455,116 rows) carries `asset_code`, `issuer_id`,
  `contract_id`, `sac_contract_id`, `sac_deployed` — an explicit *link* between
  a classic asset and its SAC rather than a second identity. If their shape
  avoids the defect by construction, that is the design to copy.

## Acceptance Criteria

- [ ] A SAC whose classic asset is in the registry no longer mints a second
      identity — pinned by a test that interns the SAC first and the classic
      asset second, and vice versa
- [ ] The 11 existing pairs are resolved, or the decision to leave them with a
      cross-reference is recorded with its reasoning
- [ ] For a pair with candles on both sides (XCR is the sharpest), the API
      reports one asset with one volume, or the split is documented as known
- [ ] No new duplicate `asset_id` is introduced by whatever is chosen (count
      per natural identity before and after)
