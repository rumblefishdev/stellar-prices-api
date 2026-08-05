---
id: "0150"
title: "Materialize price_usd_series* as an identity-keyed table (BE 0199 §6 request)"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0144", "0139", "0147", "0142", "0095", "0090", "0143", "0061"]
tags:
  ["priority-medium", "effort-large", "clickhouse", "performance", "be-interop", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 8) — BE 0199 finding 2. A valid
      request that our own schema header pre-authorized, deliberately ordered
      last because materializing before [[0139]] and [[0147]] bakes two defects
      into a physical table.
---

# Materialize `price_usd_series*` — identity-keyed

## Summary

BE measured a 104-week chart window at **70.7M read rows / 4.6 s / 2.1 GiB per
uncached request**. Bucket-range pushdown works (1.89M of 19.6M rows for a
90-day window) but identity cannot push down, because the key columns are
computed by the view. They request a materialized table
`ORDER BY (asset_kind, asset_code, issuer_address, contract_address, bucket)`.

**The pre-authorization is real and we should honour it** — both our schema
header and the design note say so in as many words:

- `views.sql:197-198` — "promote to a materialized table only if measured read
  latency demands it (design note §6)".
- `R-historical-usd-close-design.md` §6.3 — same, for `price_usd_1d`.

BE has now supplied the measurement that trips the trigger.

## Three things must be settled first

### 1. Roughly half the scan is [[0139]], not physics

Their phrase "**scans every asset's daily candles twice**" is the tell. Both
views join

```sql
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
```

`prices.assets` is `ReplacingMergeTree(updated_at) ORDER BY (asset_code,
issuer_address, contract_address)`, so `FINAL` dedups on **natural identity,
not `asset_id`** — and [[0139]] measured **3,275 `asset_id`s mapped to two or
more natural identities** on prod.

Reproduced ([[0144]] TEST D): 2× read amplification **and** a second identity
publishing a price series for a candle it never traded, because the fan-out
feeds a `GROUP BY` on identity. Materializing first turns that from a live view
artifact into stored data.

### 2. It must be ordered behind [[0147]]

A table built from today's `close_usd > 0` population inherits finding 3i and
makes it durable — a dust-print bucket becomes a stored fact. Settle the
coverage gate first, then materialize under it.

### 3. The refresh mode is the dangerous part, not the DDL

- [[0095]]/[[0090]] — a refreshable MV with a `TO` table refreshes as an atomic
  **REPLACE** over its window; that is what wiped the coarse tables. APPEND +
  `sum(version)` was the fix.
- But plain APPEND is wrong here: a bucket's `close_usd` legitimately *changes*
  as enrichment lands, so a naive append leaves both versions and lets RMT
  version arithmetic decide — which is [[0149]]'s collision.
- [[0142]] — `rollups.sql`-style `IF NOT EXISTS` MV edits silently no-op on a
  provisioned target. Whatever 0142 settles on is the delivery mechanism.
- [[0143]] — no `DEPENDS ON` anywhere in the cascade; a new tier reading a
  rollup inherits that race.

**A plain scheduled rebuild of a bounded recent window may be the cheaper,
safer answer than an MV.** Decide explicitly; do not default.

## Acceptance Criteria

- [ ] [[0139]] fixed and confirmed on prod before any table is built.
- [ ] [[0147]]'s population rule settled and the table built under it.
- [ ] Identity-keyed exactly as BE requested.
- [ ] Refresh mode chosen with a written justification against [[0095]], and
      the [[0142]] no-op trap accounted for so the DDL actually lands on prod.
- [ ] BE re-measures the 104-week window and confirms the seek.
- [ ] Storage cost measured and recorded against the [[0061]] footprint.
