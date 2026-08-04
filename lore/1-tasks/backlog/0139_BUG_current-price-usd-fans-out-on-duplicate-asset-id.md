---
id: "0139"
title: "current_price_usd returns duplicate rows — assets is keyed on natural identity, not asset_id"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0072", "0061", "0067", "0144"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "milestone-M2"]
milestone: 2
links: []
history:
  - date: 2026-08-03
    status: backlog
    who: okarcz
    note: >
      Found during [[0072]] step 5 on ch-prod-01. `current_price_usd` returned
      **4,442 rows for 4,068 `current_prices` rows** — 374 duplicates. Cause:
      `prices.assets` is `ReplacingMergeTree(updated_at) ORDER BY (asset_code,
      issuer_address, contract_address)`, so `FINAL` dedups on natural identity
      and **not** on `asset_id`; **3,275 asset_ids are mapped to two or more
      natural identities**, and the view's `INNER JOIN … ON a.asset_id =
      c.asset_id` multiplies them out. Believed **pre-existing** (the v1
      six-column view carried the same join) — 0072 only made it measurable.
      BE reads this view in-cluster (0199 contract) and has just been pointed at
      its new columns, so they are consuming the duplicates too.
---

# `current_price_usd` fans out on duplicate `asset_id`

## Summary

`prices.current_price_usd` joins `current_prices` to `assets` on `asset_id`:

```sql
FROM prices.current_prices AS c FINAL
INNER JOIN prices.assets  AS a FINAL ON a.asset_id = c.asset_id
```

`prices.assets` (`init.sql:48-66`) is:

```sql
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_code, issuer_address, contract_address)
```

`FINAL` collapses on **natural identity**, so a given `asset_id` survives on as
many rows as it has distinct `(asset_code, issuer_address, contract_address)`
tuples. Joining on `asset_id` therefore fans out.

## Measured on ch-prod-01, 2026-08-03

```
current_prices FINAL rows                       4,068
current_price_usd rows                          4,442   (+374 duplicates)
asset_ids with >1 row in assets FINAL           3,275
```

## The deeper question this exposes

374 duplicate rows is the symptom. **3,275 asset_ids mapped to more than one
natural identity is the disease** — `asset_id` is supposed to be the surrogate
key for a natural identity, and at that scale it is not unique. Before patching
the view, establish which is true:

1. **ID assignment genuinely collides** — two different assets were handed the
   same `asset_id`. Then every table keyed on `asset_id` is suspect, not just
   this view, and the blast radius is far wider than a read surface.
2. **Historical rows with superseded natural identities persist** — e.g. an
   asset whose `contract_address` was filled in later (the §12.4 SAC collapse,
   [[0061]]) creates a *new* natural-identity row while the old one remains,
   both carrying the same `asset_id`. Then `assets` is behaving as designed and
   only the view's join is wrong.

Option 2 is the more likely reading given the §12.4 write-time collapse and
`sac_address` being a later addition — but it must be **measured, not assumed**.
The discriminator: for a sample of duplicated `asset_id`s, inspect the differing
tuples and their `updated_at`. Superseded identities will look like the same
asset gaining a `contract_address`/`sac_address`; genuine collisions will look
like unrelated assets.

## Implementation (once the above is settled)

If option 2 — pick one row per `asset_id` deterministically, e.g. `argMax` over
`updated_at` in a subquery before the join, or key the join on natural identity
rather than `asset_id`. Prefer the latter if `current_prices` can carry it: it
removes the surrogate-key dependency instead of papering over it.

If option 1 — this becomes an ingestion-side task and the view fix is only a
stopgap. Spawn accordingly.

- Audit the **other** read surfaces in `views.sql` for the same join
  (`price_usd_series`, `identity_by_contract`, …) — if they join on `asset_id`
  against `assets`, they fan out identically and this is not a one-view bug.
- Add a test that fails on fan-out: seed two `assets` rows sharing an `asset_id`
  with different natural identities, assert the view returns one row per
  `current_prices` row.
- Tell BE once a direction is chosen — they read this view in-cluster and were
  pointed at it on 2026-08-03.

## Acceptance Criteria

- [ ] Determined whether the 3,275 duplicated `asset_id`s are ID collisions or
      superseded natural-identity rows, with the measurement recorded.
- [ ] `current_price_usd` returns exactly one row per `current_prices` row.
- [ ] Every other view in `views.sql` audited for the same join defect.
- [ ] A test fails if the fan-out reappears.
- [ ] BE informed of the resolution.
