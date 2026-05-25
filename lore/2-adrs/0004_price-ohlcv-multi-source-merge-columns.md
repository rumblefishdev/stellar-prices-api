---
id: "0004"
title: "price_ohlcv carries first_trade_at, last_trade_at, sources_seen for deterministic multi-source merge"
status: accepted
deciders: [okarcz]
related_tasks: ["0012", "0022", "0023", "0024", "0025"]
related_adrs: ["0003"]
tags: [architecture, schema, ohlcv, multi-source, merge, live-ingestion]
links:
  - "../1-tasks/archive/0025_RESEARCH_live-multi-source-merge-contract/notes/G-merge-contract-spec.md"
  - "../1-tasks/archive/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/notes/S-recommendation.md"
  - "../1-tasks/archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "./0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../docs/database-schema/database-schema-overview.md"
history:
  - date: 2026-05-13
    status: proposed
    who: okarcz
    note: >
      Drafted by task 0025 to record three schema additions that
      the live multi-source merge contract requires. Extends ADR
      0003 (which fixed PK identity); these columns sit on the
      same row but are non-PK.
  - date: 2026-05-13
    status: accepted
    who: okarcz
    note: >
      Accepted alongside the merge of task 0025's research PR (#12).
      Task 0012's pre-backfill schema migration lands all three
      columns alongside ADR 0003's quote_asset_id. The merge
      formula lives in a shared Rust library consumed by every
      writer (live + backfill).
---

# ADR 0004: `price_ohlcv` carries `first_trade_at`, `last_trade_at`, `sources_seen` for deterministic multi-source merge

**Related:**

- [ADR 0003: `price_ohlcv` PK includes `quote_asset_id`](./0003_price-ohlcv-pk-includes-quote-asset-id.md) — fixed the row identity; this ADR adds non-PK columns to the same row.
- [Task 0025 (active): Live multi-source merge contract](../1-tasks/active/0025_RESEARCH_live-multi-source-merge-contract/README.md) — research that produced this ADR.
- [Task 0025 G-note: merge-contract spec](../1-tasks/active/0025_RESEARCH_live-multi-source-merge-contract/notes/G-merge-contract-spec.md) — spec backing this ADR.
- [Task 0012 (backlog): SDEX + AMM backfill on Prices-owned Fargate](../1-tasks/backlog/0012_FEATURE_design-prices-owned-backfill-fargate.md) — pre-impl migration lands these columns alongside ADR 0003's `quote_asset_id`.
- [Task 0022 (archived): SDEX filter + extraction spec](../1-tasks/archive/0022_RESEARCH_sdex-filter-and-extraction-spec/README.md) — backfill populates `first_trade_at`/`last_trade_at` from the in-memory accumulator's lex tuple.
- [Task 0024 (archived): `volume_quote_usd` enrichment](../1-tasks/archive/0024_FEATURE_volume-quote-usd-enrichment/README.md) — enrichment SQL untouched by this ADR (joins on `quote_asset_id`).

---

## Context

`price_ohlcv` carries `open` / `high` / `low` / `close` /
`volume_base` / `volume_quote_usd` / `vwap` / `trade_count` /
`source`. ADR 0003 fixed the PK so that one row exists per
`(timestamp, asset_id, quote_asset_id, granularity)`.

Multiple live writers — the Prices Ledger Processor (SDEX
live), the Soroswap consumer, the Aquarius consumer, future
sources — will each write the same row when they all touch the
same native pair in the same minute. The schema doc already
commits to `source = 'aggregated'` when ≥2 sources contribute
(see `database-schema-overview.md` §3.2 "Source attribution").

Task 0025's merge-contract spec identified three gaps that the
current schema can't close:

1. **Deterministic `open` / `close` across sources.** The row
   stores `open` and `close` as `NUMERIC(28,14)` prices but
   carries no timestamp identifying which trade those prices
   came from. With two sources contributing to the same minute,
   "what's the canonical open?" is ambiguous — first arrival
   wins, which is order-dependent rather than
   chronologically-correct.

2. **Per-source attribution under `source = 'aggregated'`.**
   Once `source` flips to `'aggregated'`, the row no longer
   carries which constituent sources contributed how much. But
   `current_prices.sources` JSONB (the API-visible per-source
   breakdown over 24h) requires that information at the 1m row
   level so the Current Price Updater Lambda can sum it across
   the 24h window per source.

3. **Backfill alignment.** The SDEX backfill (task 0022) already
   tracks the `(ledger_seq, op_idx, claim_idx)` lex tuple of
   the first / last `ClaimAtom` in each minute's accumulator —
   the wall-clock of those ticks comes from
   `LedgerCloseMeta.scp_value.close_time`. Persisting that
   wall-clock on the row gives the live writer the same anchor
   when merging in.

Task 0025's G-note ([§2–§4](../1-tasks/active/0025_RESEARCH_live-multi-source-merge-contract/notes/G-merge-contract-spec.md))
recommends three additive columns.

---

## Decision

**Add three nullable columns to `price_ohlcv`:**

```sql
ALTER TABLE price_ohlcv
  ADD COLUMN first_trade_at TIMESTAMPTZ,
  ADD COLUMN last_trade_at  TIMESTAMPTZ,
  ADD COLUMN sources_seen   JSONB DEFAULT '{}'::jsonb;
```

`first_trade_at` is the wall-clock of the earliest trade
contributing to this row's `open`. `last_trade_at` is the
wall-clock of the latest trade contributing to this row's
`close`. Both are populated by writers (live + backfill).

`sources_seen` is a per-source breakdown of the contributions
the row has accumulated. Shape:

```json
{
  "sdex": {
    "volume_base": "12345.6789012",
    "volume_quote_usd": "98765.4321098",
    "trade_count": 47,
    "first_trade_at": "2026-02-10T11:00:03Z",
    "last_trade_at":  "2026-02-10T11:00:54Z"
  },
  "soroswap": { ... },
  ...
}
```

Numeric values serialised as JSON strings to preserve
`NUMERIC(28,14)` precision — same convention as
`current_prices.sources` per schema doc §3.3 note.

**Merge formula** (lives in the shared Rust library per task
0025 G-note §1.3) — the `ON CONFLICT DO UPDATE` clause becomes:

```sql
ON CONFLICT (timestamp, asset_id, quote_asset_id, granularity) DO UPDATE SET
  open  = CASE
            WHEN EXCLUDED.first_trade_at < price_ohlcv.first_trade_at
              THEN EXCLUDED.open
            ELSE price_ohlcv.open
          END,
  high  = GREATEST(price_ohlcv.high,  EXCLUDED.high),
  low   = LEAST   (price_ohlcv.low,   EXCLUDED.low),
  close = CASE
            WHEN EXCLUDED.last_trade_at > price_ohlcv.last_trade_at
              THEN EXCLUDED.close
            ELSE price_ohlcv.close
          END,
  first_trade_at  = LEAST   (price_ohlcv.first_trade_at, EXCLUDED.first_trade_at),
  last_trade_at   = GREATEST(price_ohlcv.last_trade_at,  EXCLUDED.last_trade_at),
  volume_base       = price_ohlcv.volume_base       + EXCLUDED.volume_base,
  volume_quote_usd  = price_ohlcv.volume_quote_usd  + EXCLUDED.volume_quote_usd,
  vwap = (price_ohlcv.volume_quote_usd + EXCLUDED.volume_quote_usd)
       / NULLIF(price_ohlcv.volume_base + EXCLUDED.volume_base, 0),
  trade_count = price_ohlcv.trade_count + EXCLUDED.trade_count,
  source = CASE
             WHEN price_ohlcv.source = EXCLUDED.source THEN price_ohlcv.source
             ELSE 'aggregated'
           END,
  sources_seen = jsonb_set(
    COALESCE(price_ohlcv.sources_seen, '{}'::jsonb),
    array[EXCLUDED.source],
    jsonb_build_object(
      'volume_base',      EXCLUDED.volume_base::text,
      'volume_quote_usd', EXCLUDED.volume_quote_usd::text,
      'trade_count',      EXCLUDED.trade_count,
      'first_trade_at',   EXCLUDED.first_trade_at,
      'last_trade_at',    EXCLUDED.last_trade_at
    ),
    true
  );
```

Migration lands in task 0012's pre-backfill schema bootstrap,
**alongside** ADR 0003's `quote_asset_id` column add. Both ALTERs
are zero-cost on the greenfield table.

---

## Consequences

### Positive

- **Deterministic OHLC under multi-source merge.** `open` and
  `close` resolve to the chronologically-first / last trade
  across all contributing sources, regardless of writer arrival
  order.
- **Per-source 24h breakdown reconstructible.** The Current
  Price Updater Lambda reads `sources_seen` over the rolling
  24h window per asset, sums per-source slots, writes the
  result into `current_prices.sources`. No separate per-source
  rolling state needed.
- **Backfill + live unify on the same row shape.** Backfill
  populates `first_trade_at` / `last_trade_at` from the
  in-memory accumulator's lowest / highest lex-tuple ticks (see
  task 0022 decode spec §5.1 — the accumulator already tracks
  `(ledger_seq, op_idx, claim_idx)`; map to wall-clock via
  `LedgerCloseMeta.scp_value.close_time`). The live writer
  populates the same fields from its trade timestamps. Same
  merge formula handles both.
- **No new tables.** Three additive columns on the existing
  row; no new joins, no new write paths.

### Negative / costs

- **Per-row storage overhead.** `first_trade_at` +
  `last_trade_at` add 16 bytes (2 × TIMESTAMPTZ); `sources_seen`
  adds ~50-200 bytes for multi-source rows, ~30-50 bytes for
  single-source. For ~8 GB of `price_ohlcv` projected at year 1,
  the addition is ~3-8% of the table size. Well inside the
  design-doc storage budget.
- **JSONB read/write cost.** `jsonb_set` per UPSERT is fast in
  PG but non-trivial; the cost is well-amortised per-row and
  scales with row write rate (~10 candles/minute × pairs).
  Negligible at projected loads.
- **Two timestamp columns are nullable initially.** Pre-existing
  rows from before this migration (none in production since
  greenfield) would have NULL; the merge formula's
  `LEAST`/`GREATEST` handle NULL by SQL semantics
  (`LEAST(NULL, x) = x` in PG). Safe.

### Neutral

- **Naming.** `first_trade_at` / `last_trade_at` chosen over
  `min_trade_ts` / `max_trade_ts` for human readability. The
  `_seen` suffix on `sources_seen` distinguishes "sources that
  have written into this row" from `current_prices.sources`
  (which is the API-projected breakdown).
- **No CHECK constraint** on `sources_seen` shape — Postgres
  doesn't natively validate JSONB schemas. Writer-side
  responsibility; the shared merge library is the single source
  of truth.

---

## Implementation hooks

1. **Task 0012** schema-bootstrap migration adds these three
   columns (alongside ADR 0003's `quote_asset_id`).
2. **Task 0022** decode-and-bucket spec §5 needs a one-line
   correction: the in-memory accumulator's per-tick wall-clock
   from `closed_at` becomes `first_trade_at` (lowest-lex tick) /
   `last_trade_at` (highest-lex tick) at flush time. Edit lands
   in task 0012's implementation worklog (matches the same
   convention used for ADR 0003's spec edit).
3. **Task 0025** merge-contract spec §3.1 is the source of
   truth for the merge formula; the shared Rust library
   implements it.
4. **Current Price Updater Lambda** (in design doc §5.5)
   re-spec'd to read `sources_seen` instead of inferring
   per-source breakdown some other way. Spawn as part of the
   live-writer implementation tasks (post-0012).

---

## Open questions (not blockers)

- **Backfill `first_trade_at` / `last_trade_at` granularity.**
  The Stellar protocol's `LedgerCloseMeta.scp_value.close_time`
  is per-ledger (~5–6 s granularity), not per-trade. For 1m
  candles this is fine — multiple trades in the same ledger
  share the same `closed_at`, which is still inside the 1m
  bucket. For sub-second analytics this would be coarse; not
  applicable here.
- **`sources_seen` retention vs `price_ohlcv` retention.** The
  cleanup-worker drops fine-grained rows by granularity
  (`1m` after 7 days, `15m` after 30 days). `sources_seen` ages
  out with the row. No separate retention policy needed.
- **Per-source `volume_quote_usd` and the enrichment Lambda.**
  Task 0024's enrichment fills `volume_quote_usd` for the row
  as a whole; the `sources_seen.*.volume_quote_usd` per-source
  slot is the writer's own contribution at write time. If a
  source writes `volume_quote_usd = 0` (pre-enrichment), its
  slot keeps that value; the row-level enrichment doesn't
  back-propagate into `sources_seen` slots. This is correct
  for the per-source 24h breakdown — each source's USD volume
  is its own responsibility, not an enrichment artefact.
  Document this in task 0024 spec when the live writers come
  online; for now, flagged.
