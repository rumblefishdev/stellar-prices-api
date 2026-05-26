---
id: "0003"
title: "price_ohlcv PK includes quote_asset_id: store one OHLCV row per (asset, quote, minute) native pair"
status: accepted
deciders: [okarcz]
related_tasks: ["0012", "0022", "0023", "0024", "0025"]
related_adrs: ["0001", "0002"]
tags: [architecture, schema, ohlcv, primary-key, sdex, backfill, multi-quote]
links:
  - "../1-tasks/archive/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/notes/S-recommendation.md"
  - "../1-tasks/archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../../docs/database-schema/database-schema-overview.md"
  - "../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-13
    status: proposed
    who: okarcz
    note: >
      Drafted by task 0023 to resolve the PK ambiguity surfaced
      in task 0022's decode-and-bucket spec §6 item 1. Replaces
      the implicit "asset_id = base; quote is implied" convention
      with explicit per-native-pair row identity.
  - date: 2026-05-13
    status: accepted
    who: okarcz
    note: >
      Accepted alongside the merge of task 0023's research PR (#10).
      Option A (add `quote_asset_id` to the PK) is the committed
      design. Task 0012 lands the DDL in its pre-backfill schema
      migration. Tasks 0024 and 0025 are unblocked.
---

# ADR 0003: `price_ohlcv` PK includes `quote_asset_id`

**Related:**

- [ADR 0002: Stream 2 SDEX historical backfill is fully independent of Block Explorer](./0002_stream2-sdex-archive-backfill-independent-of-be.md) — motivates the SDEX backfill that surfaced the PK ambiguity
- [Task 0022 (archived): SDEX filter + extraction spec](../1-tasks/archive/0022_RESEARCH_sdex-filter-and-extraction-spec/README.md) — decode spec §6 item 1 flagged this
- [Task 0023: OHLCV row identity research](../1-tasks/active/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/README.md) — research backing this ADR
- [Task 0024 (backlog): `volume_quote_usd` enrichment pass](../1-tasks/backlog/0024_FEATURE_volume-quote-usd-enrichment.md) — consumes the new `quote_asset_id` column
- [Task 0025 (backlog): live multi-source merge contract](../1-tasks/backlog/0025_RESEARCH_live-multi-source-merge-contract.md) — collides per-pair under this PK shape
- [Task 0012 (backlog): SDEX + AMM backfill on Prices-owned Fargate](../1-tasks/backlog/0012_FEATURE_design-prices-owned-backfill-fargate.md) — implements the migration as part of its schema-bootstrap step

---

## Context

`price_ohlcv` as documented in `database-schema-overview.md` §3.2 is
keyed `PRIMARY KEY (timestamp, asset_id, granularity)`. `asset_id`
is implicitly the **base asset** of an asset pair; the quote is not
captured on the row.

This works under a single-quote assumption (one asset trades against
one canonical quote). SDEX violates that: USDC trades against XLM,
USDT, EURT, and many others, all in the same minute. With the
current PK, those native-pair candles collide on the row key.

The four candidate resolutions were laid out in task 0022's
decode-and-bucket spec §6 item 1 and analysed in task 0023:

- **A**: add `quote_asset_id` to the PK (`+ INT` column).
- **B**: introduce an `asset_pairs` surrogate table; PK becomes
  `(timestamp, asset_pair_id, granularity)`.
- **C**: normalise quote to `quote_kind ∈ {USD, XLM}` at write
  time; one row per (asset, kind, minute).
- **D**: keep base-only PK; accept the collision (broken).

The API surface (`GET /assets/{id}/ohlcv?base_currency=USD|XLM`)
projects to USD or XLM at read time — so storage **must** retain
per-native-pair distinction to support per-source attribution and
to defer USD conversion to the enrichment pass (task 0024).

Task 0023's [S-recommendation note](../1-tasks/active/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/notes/S-recommendation.md) carries
the full option matrix and trade-off analysis.

---

## Decision

**Adopt Option A.** Change `price_ohlcv` PK to:

```sql
PRIMARY KEY (timestamp, asset_id, quote_asset_id, granularity)
```

where `asset_id` is the canonical-base asset's surrogate id and
`quote_asset_id` is the canonical-quote asset's surrogate id. Both
reference the `assets` table.

Concrete schema change (lands in task 0012's pre-backfill
migration; the table is greenfield so no data move is required):

```sql
ALTER TABLE price_ohlcv DROP CONSTRAINT price_ohlcv_pkey;

ALTER TABLE price_ohlcv ADD COLUMN quote_asset_id INT;
-- nullable initially; set NOT NULL once the writer wires up

ALTER TABLE price_ohlcv
  ADD CONSTRAINT price_ohlcv_pkey
  PRIMARY KEY (timestamp, asset_id, quote_asset_id, granularity);

DROP INDEX IF EXISTS idx_ohlcv_asset_gran;

CREATE INDEX idx_ohlcv_asset_gran
  ON price_ohlcv (asset_id, granularity, timestamp DESC, quote_asset_id);

CREATE INDEX idx_ohlcv_pair_gran
  ON price_ohlcv (asset_id, quote_asset_id, granularity, timestamp DESC);

ALTER TABLE price_ohlcv ALTER COLUMN quote_asset_id SET NOT NULL;
```

Native range partitioning by `timestamp` is unaffected (PK gains a
column; partitioning column is unchanged).

---

## Consequences

### Positive

- **SDEX correctness.** Multi-quote pair trades (e.g. USDC/XLM and
  USDC/USDT in the same minute) no longer collide; each gets its
  own row.
- **Per-source attribution preserved.** SDEX and Soroswap and
  Aquarius can each write rows for the same `(asset, quote,
  minute)` triple; the multi-source merge contract (task 0025)
  has well-defined rows to merge.
- **Backfill decoupled from oracle data.** USD conversion is
  deferred to task 0024's enrichment pass — backfill writes the
  native-quote `volume_quote` and `vwap`; USD denomination
  derives later. With Option C this would have been impossible.
- **Cheap to migrate.** Greenfield table; one `ALTER` per index
  + PK + column. No row backfill.
- **Index size tractable.** PK tuple grows by 4 bytes
  (`INT`). For ~8 GB of `price_ohlcv` projected at year 1, PK
  overhead grows by ~5–10% — well inside the design-doc
  storage budget.

### Negative / costs

- **API read path is more work.** Producing the USD or XLM
  projection requires aggregating across multiple per-pair rows
  (one per native pair contributing in the minute). Acceptable —
  this is the standard exchange-aggregator pattern.
- **`current_prices.price_xlm` semantic is now "weighted across
  XLM-quoted pairs".** Trivial in practice (most assets have
  one XLM-quote pair) but worth documenting in the Current Price
  Updater spec.
- **One naming infelicity preserved.** The
  `?base_currency=USD|XLM` query param on the OHLCV endpoint is
  mislabelled (it's the quote). Rename deferred to API v2; not
  a blocker for this ADR.

### Neutral

- **`asset_pairs` table not introduced.** Option B's surrogate
  is not adopted. If a future table needs to key on pair
  identity, a follow-up ADR can introduce `asset_pairs` and
  migrate `price_ohlcv` to it; that migration is cheap and not
  worth pre-paying today.

---

## Implementation hooks

1. **Task 0012** schema-bootstrap migration includes the DDL
   above before any backfill writes. The migration runs once at
   CDK deploy time.
2. **Task 0022 decode-and-bucket spec** receives a one-line
   correction: the in-memory accumulator key gains
   `quote_asset_id` and the UPSERT SQL `conflict target` gains
   the column. Edit lands in 0012's implementation worklog,
   not retroactively in the archived spec (which would mask the
   evolution).
3. **Task 0024 enrichment pass** joins `oracle_prices` on
   `quote_asset_id` to fill `volume_quote_usd`.
4. **Task 0025 multi-source merge** specs the same-row merge
   contract under the new PK shape.

---

## Open questions (not blockers)

- Should `quote_asset_id = asset_id` be allowed? (Self-pair —
  semantically meaningless.) Recommend adding a CHECK constraint:
  `CHECK (asset_id <> quote_asset_id)`. Defer to 0012's
  migration code.
- Ordering of `(asset_id, quote_asset_id)` — should the schema
  enforce canonical orientation (e.g. `asset_id < quote_asset_id`
  via CHECK)? Task 0022 spec §2.2 already picks orientation
  upstream of the write; a CHECK is belt-and-suspenders. Defer.
