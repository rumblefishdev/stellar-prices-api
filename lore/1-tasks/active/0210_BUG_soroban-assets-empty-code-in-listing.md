---
id: "0210"
title: "Soroban assets carry an empty asset_code in listing and detail — top-volume rows are unidentifiable to consumers"
type: BUG
status: active
related_adr: []
related_tasks: ["0120"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M2, api, metadata, defect]
milestone: 2
links: []
history:
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: "Spawned from the 0120 conformance run."
  - date: 2026-08-20
    status: active
    who: stkrolikiewicz
    note: >
      Promoted, and the implementation section rewritten: the original plan
      (resolve `symbol()` into `assets.asset_code`) would have caused a
      production defect, not fixed one. `asset_code` is part of the sort key
      of a `ReplacingMergeTree`, so amending it on an existing row writes a
      SECOND row rather than replacing — the same `asset_id` on two natural
      identities, which is exactly the live fan-out [[0139]] measured on
      3,275 ids. `GET /assets` would then return the token twice, once
      nameless and once named. Independently, `views.sql:117-121` forces
      `asset_code = ''` for contract-kind rows *deliberately and in
      anticipation of this task* ("the view does not depend on the writer
      keeping it blank"), so the symbol would not surface there anyway.
      Re-scoped onto `asset_metadata`. Effort revised from small to medium.
  - date: 2026-08-28
    status: active
    who: stkrolikiewicz
    note: >
      Measured the population, and corrected the record. **This branch already
      held the right answer and never merged**, so `develop`'s copy of the task
      still showed the original `assets.asset_code` plan — and a fresh pass on
      2026-08-28 re-derived the same re-scope from scratch and briefly committed
      the WRONG destination to `develop` (ca62fea) before finding this branch.
      Nothing here changes: the 2026-08-20 analysis stands, verified again
      against `init.sql` (`assets` really is `ORDER BY (asset_code,
      issuer_address, contract_address)`) and `views.sql:117-121` (contract rows
      really are normalised to `asset_code = ''`).
      New this pass — the measured population, which nobody had counted:
      **52** soroban rows in `prices.assets`, all 52 with an empty code, all
      active; **20** ever produced a candle; **4** are visible through
      `GET /v1/assets` (the listing INNER JOINs `current_prices`, so an
      API-side count answers "how many consumers see this broken today", not
      "how many rows need fixing"). Registry total 207,493. So the backfill is
      ~52 RPC calls, not a paginated job — which is why the worker can copy
      supply-worker's *shape* without needing its scale machinery.
---

# Soroban assets have empty asset_code

## Summary

Every soroban-native row in `GET /v1/assets` (including `CBIJ…`, ranked #6 by
24h volume at ~$29.5k) has `asset_code: ""` in both the listing and
`GET /v1/assets/{id}` (`code: ""`). Consumers see a top-10 asset with no
name — only a 56-char contract address.

## Context

Found by [[0120]]. Soroban token metadata (symbol/name) lives on the token
contract; the ingest pipeline evidently never resolves it into
`prices.assets`.

## Population (measured on prod, 2026-08-28)

| | count |
|---|---|
| soroban rows in `prices.assets` | **52** |
| …with an empty `asset_code` | **52** (all of them) |
| …that ever produced a candle | 20 |
| …visible in `GET /v1/assets` today | 4 |
| registry total, for scale | 207,493 |

The three numbers differ for a reason worth knowing before anyone re-counts:
the listing does `FROM current_prices INNER JOIN assets`, so it shows only
assets that traded in the last 24 h. A count taken off the API answers "how
many consumers can see broken today", not "how many rows need fixing".

So the backfill is **~52 RPC calls**, not a paginated job with a time budget.

## Where the symbol must NOT go

`prices.assets.asset_code`. Two independent reasons, both load-bearing:

1. **It is in the sort key.** `ORDER BY (asset_code, issuer_address,
   contract_address)` on a `ReplacingMergeTree(updated_at)` — dedup happens
   *within* a sort key, never across it. Writing `('SHX','','C…')` where
   `('','','C…')` exists leaves BOTH rows, one `asset_id` on two identities.
   Every `ON a.asset_id = c.asset_id` join then fans out: `GET /assets`
   returns the token twice, once nameless and once named. That is [[0139]],
   already live on 3,275 ids — this would deliberately add to it.
2. **The read surface forces it blank anyway.** `views.sql:117-121` normalises
   contract-kind rows to `asset_code = ''` at read time and says why: *"so the
   (contract ⇒ asset_code='') interop contract holds even if
   discovery/metadata ever populates a symbol into a Soroban token's
   asset_code — the view does not depend on the writer keeping it blank."*
   Someone closed this door on purpose.

## Implementation

Destination is **`prices.asset_metadata`** — `asset_id`-keyed, sort key
unaffected by content changes, single-writer, already `LEFT JOIN`ed by the API
(`queries_ch.rs:198`) for `home_domain`. Today it holds only `home_domain` and
**has no production writer at all** (`write_asset_metadata`,
`writer.rs:295-310`, is called from one test).

- **Schema:** add `symbol String DEFAULT ''` to `asset_metadata`.
- **Resolver:** the Soroban RPC capability already exists and is generic —
  `build_simulate_envelope(contract, func, args)` in `oracle-worker`
  (`lib.rs:170-204`) takes any contract, any function, any args, and
  `fetch_lastprice` (`lib.rs:225-254`) is the `simulateTransaction`
  round-trip. Only its last line is Reflector-specific. Factor out the
  round-trip, add a `ScVal::Symbol`/`ScVal::String` parser, call
  `symbol()`.
- **Worker:** copy `supply-worker`'s stalest-first pattern
  (`lib.rs:162-186`) — `LEFT JOIN` the side table, order by
  `coalesce(fetched_at, 0) ASC`, time+count budget, write a sentinel on
  "absent" so an unresolvable contract rotates out instead of starving the
  queue. Filter is the inverse of supply-worker's:
  `asset_type = 'soroban' AND contract_address != ''`.
- **API:** select `m.symbol` and add it to `AssetListItem` / `AssetDetail` as
  its own field. **Not** as `asset_code` — the response keeps
  `asset_code: ""` for contract assets, per the §4.1 interop contract.
### Two alternatives measured on 2026-08-28, both declined

**BE's `default.soroban_contract_metadata`, on the same box.** It carries
`contract_id`, `name`, `symbol`, `decimals` over 3,922 rows, and an in-cluster
join is an established pattern in the other direction (ADR 0007 kept the shared
box partly because a dedicated one "breaks BE's in-cluster `price_usd_series`
JOIN"). Coverage of our 52 is **17** — but the right 17: `CBIJ…` resolves to
`SolvBTC`, plus `XAUM` and `xSolvBTC`, i.e. three of the four assets a consumer
can see today, and it yields `decimals` too, which `symbol()` alone does not.
**Declined as the mechanism** because 17/52 is not coverage, and an acceptance
criterion should not depend on another team's table before they have agreed to
it being a dependency. Worth raising with BE separately — as a supplement that
would cut RPC calls, not as the path.

**The SAC subset (below).** Measured: **11** of the 52 are SACs of classic
assets already in the registry, resolvable with no RPC at all. But none of the
11 is visible through the API, so it closes no part of this task's acceptance
criteria — and the split it exposes turned out to be an identity defect rather
than a naming one, now [[0242]]. Keep the idea here as an optimisation for
later; do **not** sequence it first, contrary to what this section said before
the measurement.

- **Free subset, no RPC (deferred — see above; measured at 11 of 52, none of
  them API-visible):** SAC `transfer`
  events already carry `CODE:ISSUER` as topic[3] and that data flows through
  `soroban.rs:285-311` today and is discarded. Verify by re-deriving
  `sac_address()` (`canonical.rs:125-133`) from the parsed code+issuer and
  comparing to the emitting contract, then feed `register_sac`. That names
  the SAC subset offline and shrinks the population needing an RPC call —
  a SAC whose classic asset never traded on SDEX is currently minted as a
  nameless `Contract` purely because `resolve_sac` is a lookup table, not a
  derivation.
- 0120 keeps `code: ""` equality for `CBIJ…` in its fixed list — that stays
  correct under this design, since `asset_code` remains `''`. Update the
  suite to assert the new `symbol` field instead.

⚠️ [[0139]] is unfixed, so any `asset_id`-keyed side table inherits ambiguous
ids. `asset_metadata` is already `asset_id`-keyed and already joined, so this
adds no new exposure — but do not treat the join as sound.

## Acceptance Criteria

- [ ] Soroban assets expose their on-chain symbol through the API, in a field
      of their own — `asset_code` stays `''` for contract-kind assets and the
      §4.1 interop contract is unchanged
- [ ] `CBIJ…` shows its real symbol in list + detail
- [ ] No new row is created in `prices.assets` for an asset that gains a
      symbol (verify the row count per `asset_id` before/after)
- [ ] The resolver rotates unresolvable contracts out instead of retrying them
      forever (sentinel on absent, as supply-worker does)
