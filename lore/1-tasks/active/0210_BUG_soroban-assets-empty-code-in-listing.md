---
id: "0210"
title: "Soroban assets carry an empty asset_code in listing and detail — top-volume rows are unidentifiable to consumers"
type: BUG
status: active
related_adr: []
related_tasks: ["0120"]
tags: [layer-backend, priority-medium, effort-small, milestone-M2, api, metadata, defect]
milestone: 2
links: []
history:
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: "Spawned from the 0120 conformance run."
  - date: 2026-08-21
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active (commit cba60ac). Entry added retroactively on
      2026-08-28: that commit moved the file and flipped `status` but landed no
      history line, so the task read `active` with a `backlog`-only trail.
      Third instance of this class in a week — [[0165]] had status drift,
      [[0120]] carried two archived blockers — so it is the flow, not the
      person.
  - date: 2026-08-28
    status: active
    who: stkrolikiewicz
    note: >
      Measured the population and settled the implementation shape.
      **52** soroban rows in `prices.assets`, all 52 with an empty code, all 52
      active; **20** of them have ever produced a candle; **4** are visible
      through `GET /v1/assets` right now (the listing INNER JOINs
      `current_prices`, so it shows only what traded in 24 h — a first count of
      "4" off the API was that subset, not the registry). Registry total is
      207,493 against ~3,620 rows in `current_prices`. So the backfill is ~52
      RPC calls, not a paginated job. Home is the existing `asset-discovery`
      worker; `asset_metadata` carries only `home_domain` and would need a
      schema change, while `assets.asset_code` is the field the API already
      reads and `asset-discovery` already writes.
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

## Implementation

Resolve `symbol()` (and `name()`) inside the existing **`asset-discovery`**
worker, and write the result to `assets.asset_code`.

- **`asset-discovery` is already the writer of `prices.assets`** and already
  runs hourly (`rate(1 hour)`, `AssetDiscoveryRule`). Its stated job is to scan
  a ledger window, extract every asset appearing in SDEX trades and Soroban AMM
  swaps, and register the new ones — so a new soroban asset is born there, with
  the row already in hand. Adding one resolve step is cheaper than standing up a
  worker beside it, and the same pass backfills the existing 52.
- **Write to `assets.asset_code`**, not to `asset_metadata`. That table carries
  only `asset_id`, `home_domain` and `updated_at`, so a symbol would need a
  schema change; `asset_code` is the column the API already reads on both
  routes. The single-writer concern (schema overview §3.9/§3.10) does not
  bite here precisely because the resolver lives in the writer.
- **Not at mint time in the ledger path.** That path must keep up with the
  chain, and a synchronous Soroban RPC call in it turns a slow or failing RPC
  into ingest lag. It also gets one attempt per asset: if RPC is down at that
  moment the asset stays nameless until somebody notices, whereas an hourly
  pass retries by construction and picks up a contract that changes its symbol.
- **Best-effort, per the `supply-worker` precedent**: a contract that exposes
  no `symbol()` must leave the row alone and let the run succeed, not fail it.
- RPC plumbing exists to reuse — `oracle-worker` and `supply-worker` both call
  `simulateTransaction` already.
- **Update `tools/scripts/conformance-assets.json` in the same PR.** [[0120]]
  pinned `"code": ""` for `CBIJ…` as expected behaviour, so its suite starts
  failing on this fix unless the fixture moves with it.

⚠️ `effort-small` understates this: it is an RPC call plus decoding plus a
write path in an existing worker, plus the fixture change. Small, but not a
one-liner.

## Acceptance Criteria

- [ ] Soroban rows in the listing carry a non-empty code where the contract
      exposes one
- [ ] `CBIJ…` shows its real symbol in list + detail
