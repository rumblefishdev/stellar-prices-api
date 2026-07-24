---
id: "0129"
title: "Joining prices.assets on asset_id fans out ~0.4% even under FINAL — FINAL dedupes by natural key, not asset_id"
type: BUG
status: backlog
related_adr: ["0003"]
related_tasks: ["0114", "0054"]
tags: [layer-database, clickhouse, data-quality, priority-medium, effort-small, assets, join]
links:
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0114]]'s residual-composition check. Two queries over the
      same span disagreed on total rows by exactly 335,830 (82,671,727 vs
      82,335,897) — and their zero counts differed by exactly the same amount.
      The difference is an `INNER JOIN prices.assets ON a.asset_id =
      c.quote_asset_id` fanning out. Immaterial to 0114's conclusion (0.4%
      against a 100%-vs-0.9% split) but a live hazard for any other query that
      joins on `asset_id`.
---

# Joining `prices.assets` on `asset_id` fans out even under `FINAL`

## Summary

`prices.assets` is `ReplacingMergeTree(updated_at)` with

```sql
ORDER BY (asset_code, issuer_address, contract_address)
```

so `FINAL` collapses duplicates **by natural key**. `asset_id` is an
application-assigned surrogate and is *not* part of the sort key, so nothing in
the engine guarantees it is unique after collapse. If one `asset_id` ever gets
attached to two different natural-key rows, every join on `asset_id` silently
multiplies rows — and `FINAL` will not save you, because both rows are legitimate
distinct keys as far as the engine is concerned.

Measured **0.4% fan-out** on the current production table.

## Evidence

Two queries over `price_ohlcv_1h`, same span (202402–202607), same
`volume_quote > 0` filter, run minutes apart during [[0114]]'s verification:

| query | total rows | total zeros |
|---|---|---|
| no join (group by month) | 82,335,897 | 47,804,264 |
| `INNER JOIN assets FINAL ON a.asset_id = c.quote_asset_id` | 82,671,727 | 48,140,094 |
| **difference** | **+335,830** | **+335,830** |

The deltas being *identical* is the tell: this is row duplication, not a
filtering difference. An `INNER JOIN` that dropped unmatched rows would make the
joined count **smaller**, not larger.

The un-joined 47,804,264 is independently corroborated — it is the exact figure
`coarse-repair` printed as its `no_reference` floor — so the un-joined side is
the trustworthy one.

## Why it matters beyond 0114

0114's conclusion is unaffected (0.4% cannot move a 100%-vs-0.9% split). The
exposure is everywhere else:

- the read API joins `current_prices` → `assets` on `asset_id`
  (`queries_ch.rs`, both the listing and the batch path) — a duplicated
  `asset_id` would emit the same asset twice in `GET /assets` and corrupt the
  keyset cursor, which uses `(sort_value, asset_id)` as its tie-break
- `mv_current_prices` `LEFT JOIN`s `asset_supply` on `asset_id`
- any analytics query grouping by quote/base asset over-counts by the fan-out

## Investigation

1. **Find the duplicates first — the fan-out may be one bad row or thousands:**
   ```sql
   SELECT asset_id, count() AS n, groupArray(asset_code) AS codes,
          groupArray(issuer_address) AS issuers,
          groupArray(contract_address) AS contracts
   FROM prices.assets FINAL
   GROUP BY asset_id HAVING n > 1
   ORDER BY n DESC LIMIT 50;
   ```
2. Classify them. Plausible causes, cheapest first:
   - the same asset registered twice under slightly different natural keys (e.g.
     a SAC contract address filled in later, creating a second row that kept the
     original surrogate id)
   - two genuinely different assets colliding on one surrogate id — far more
     serious, and would mean the id allocator can repeat
   - a `sac_address` / `contract_address` migration that rewrote one column
3. Decide the fix by cause. Options, in rough order of preference:
   - **repair the rows** (merge duplicates onto one natural key) if it is a small
     fixed set
   - **make `asset_id` uniqueness enforceable** — e.g. a dedicated
     `ReplacingMergeTree` keyed by `asset_id` that reads as the canonical
     id→identity map, so joins target a table where `FINAL` actually guarantees
     one row per id
   - fix the allocator if it can repeat ids
4. Add a cheap invariant check so this cannot silently regress — a query
   asserting `count() = countDistinct(asset_id)` on `assets FINAL`, run either in
   the schema integration tests or as a scheduled probe.

## Acceptance Criteria

- [ ] The duplicate `asset_id` rows are enumerated and their cause identified —
      registration artifact vs. a repeating id allocator (these have very
      different severities)
- [ ] `SELECT count(), countDistinct(asset_id) FROM prices.assets FINAL` returns
      equal values in production
- [ ] The same two-query cross-check from §Evidence agrees to the row
- [ ] An invariant test or probe guards `asset_id` uniqueness going forward
- [ ] `GET /assets` verified to emit no duplicate asset across a full cursor walk
      (extends 0074's pagination test, and relevant to [[0120]]'s conformance
      pass)

## Notes

- ADR 0003 fixed the OHLCV PK to include the quote leg; this task is about the
  `assets` surrogate id, a different key. No ADR change is expected unless the
  fix introduces a new canonical id table.
- The natural-key sort order is not itself wrong — it is right for identity
  lookups, which is what §3.1 of the design doc wants. The bug is the *assumption*
  that `FINAL` therefore makes `asset_id` unique.
