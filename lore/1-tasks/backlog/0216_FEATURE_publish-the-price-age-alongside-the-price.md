---
id: "0216"
title: "Publish how old the price is, so a consumer can apply its own freshness policy instead of inheriting ours"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0135", "0178", "0165", "0151", "0111", "0215"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M3, clickhouse, api, read-surface]
milestone: 3
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-20
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0135]]'s PR #228 review. 0135 decided `price_usd` is the
      latest *priced* close and is deliberately NOT age-bounded; the honest
      completion of that decision is to publish the age rather than blank the
      value. okarcz agreed the bound belongs only in the per-venue pipeline
      and that the age is its own task.
---

# Publish the price's age

## Summary

`price_usd` is the latest **priced** close, which for an asset whose recent
candles are un-enriched — or which simply stopped trading — can be materially
older than the row it sits in. `updated_at` is the MV's refresh time, so it
reads "a minute ago" no matter how old the price is. **No column carries the
age**, so a consumer cannot apply a freshness policy at all.

## Why this, and not a staleness bound

0135 considered bounding `price_usd` and rejected it for a measured reason:
on prod 2026-08-20, **1,091 of 4,444 assets (24.5%)** already publish a hard
zero. A bound tight enough to be a real freshness promise pushes more assets
into that sentinel — and a zero is strictly worse than an old-but-true price,
because a consumer cannot tell "worthless" from "we don't know".

One field cannot answer both *"what is the price"* and *"how fresh is it"*.
Publishing both lets each consumer pick its own threshold; a dashboard and a
liquidation engine legitimately want different ones.

## Design questions to settle first

- **What instant does it name for `price_xlm`?** That column is
  `price_usd / xlm_usd` — a quotient of **two independently dated** closes, so
  it is not a price "as of" any single instant and never was. okarcz's
  position, and it looks right: report the **older of the two**, otherwise
  `price_xlm` is handed an age it does not have.
- **`market_cap_usd`** multiplies `price_usd` by a supply figure carrying its
  own `asset_supply.fetched_at`. Same question, same answer shape.
- **Sentinel vs NULL** for "no price at all". `current_prices` columns are
  non-nullable by convention and use sentinels — [[0151]] owns that argument;
  a zero-epoch `DateTime` is the obvious analogue.

## Implementation notes

Cheap in itself: one row per asset, `ALTER TABLE … ADD COLUMN` is metadata
only, and the MV recomputes every row every minute, so there is **no
backfill**. Three traps, all from the PR #228 review:

- ⚠️ **Positional insert.** The `TO` clause carries an explicit column list
  precisely because an MV inserts positionally otherwise. A new column must go
  into **both** `TO(...)` and the SELECT, in the same order. Getting it wrong
  writes the price into the age column, with no error anywhere.
- ⚠️ **The clamp is coupled to the column type.** The SELECT clamps
  `change_*_pct` to ±999999 because `Decimal(10,4)` holds exactly that.
  Widening a clamp without widening its column overflows the INSERT and the
  refresh throws — and because this MV is **REPLACE, not APPEND**,
  `current_prices` then stops updating for *all* 4,444 assets, not just the
  offending one.
- ⚠️ Same REPLACE property: anything that *filters* rows here removes those
  assets from the table entirely rather than blanking a field. Emit sentinels,
  never filter.

Consider shipping alongside [[0178]], which already rewrites this MV
(DROP + re-CREATE) and already plans a provenance column following [[0165]]'s
`traded`/`peg`/`oracle` vocabulary — one exposure window instead of two, and
"where did this number come from" and "when was it true" are the same
conversation.

## Acceptance Criteria

- [ ] `/price` and `/assets` expose the price's own timestamp, distinct from
      `updated_at`, and the published OpenAPI descriptions say which is which
- [ ] `price_xlm`'s reported age is the older of its two inputs — or the
      opposite choice is recorded with its reasoning
- [ ] `views.sql`'s sentinel table documents the new column for BE
- [ ] Adding the column changes no existing value — verified by comparing a
      full `current_prices` snapshot before and after
- [ ] The `TO(...)` list and the SELECT projection are asserted to match in a
      test, so the positional-insert trap cannot recur silently
