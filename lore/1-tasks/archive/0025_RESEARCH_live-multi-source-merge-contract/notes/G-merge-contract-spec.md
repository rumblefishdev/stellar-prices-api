---
title: "Live multi-source merge contract — writer-side detection + per-column merge rules"
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [ohlcv, multi-source, live-ingestion, merge, design, schema]
links:
  - "../README.md"
  - "../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../../archive/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/notes/S-recommendation.md"
  - "../../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md"
  - "../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../../docs/database-schema/database-schema-overview.md"
history:
  - date: 2026-05-13
    status: mature
    who: okarcz
    note: >
      Design spec answering the three open questions from task
      0025's README: (1) who initiates merge, (2) numeric merge
      rules, (3) source attribution recoverability. Surfaces one
      schema-change need: add `first_trade_at` / `last_trade_at`
      timestamp columns for deterministic open/close.
---

# Live multi-source merge contract — writer-side detection + per-column merge rules

This note specifies what happens when two or more live writers
(SDEX Ledger Processor, Soroswap, Aquarius, …) target the same
`price_ohlcv` row in the same minute. The contract is **embedded
in each writer's UPSERT statement**, with a shared library
encapsulating the merge formula. The backfill contract (task
0022) writes single-source rows; this spec handles what happens
when later writes touch them.

## TL;DR

| Concern                                | Decision                                                                                      |
| -------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Who initiates the merge                | **Each live writer**, on UPSERT conflict. Embedded in the `ON CONFLICT DO UPDATE` clause. No separate aggregator process. |
| Open / close determinism               | Requires two new columns: `first_trade_at TIMESTAMPTZ`, `last_trade_at TIMESTAMPTZ`. Writers populate; merge picks `MIN(first_trade_at)` and `MAX(last_trade_at)` to choose the canonical open/close. **Small schema-change ADR addendum to 0003.** |
| Per-column merge rules                 | `high` = `GREATEST`; `low` = `LEAST`; `volume_base`, `volume_quote_usd`, `trade_count` = `SUM`; `vwap` = recomputed from accumulated `volume_quote_usd / volume_base`; `open`/`close` tied to `first_trade_at`/`last_trade_at`. |
| Source label transition                | `source = 'aggregated'` once ≥ 2 distinct sources have contributed. Single-source rows keep their original source label (`'sdex'`, `'soroswap'`, etc.). |
| Per-row source breakdown               | **Not stored on `price_ohlcv`.** Per-source visibility lives in `current_prices.sources` JSONB (already specced in the schema doc). If a per-1m-row breakdown becomes useful later, a `sources_seen JSONB` column can be added. |
| Backfill interaction                   | Backfill writes single-source whole-row replacement; live writer arriving later runs the merge formula. The backfill never needs to know about other sources — its row is "first to land" in its minute. |

## 1. Who initiates the merge

### 1.1 The three options

The README listed three candidates:

| Option | Description                                                                 |
| ------ | --------------------------------------------------------------------------- |
| (a)    | Each live writer detects on UPSERT conflict and rewrites with merged values. |
| (b)    | Separate aggregator process polls and consolidates multi-touched rows.       |
| (c)    | Rollup Lambda merges when it re-aggregates 1m → 15m / 1h / etc.              |

### 1.2 Why (a) — writer-side detection

- **Latency.** (a) commits the consistent row in the same
  transaction as the conflict; (b) introduces a polling lag; (c)
  doesn't fix 1m at all — it just propagates inconsistency.
- **Atomicity.** PG's `INSERT ... ON CONFLICT DO UPDATE` is
  row-level atomic. Multiple writers serialise on the row lock,
  so each one sees a consistent existing-row state before
  applying its merge. No race window.
- **Locality.** The merge formula lives where the row is being
  modified. (b) needs a separate process that re-reads and
  re-writes the row, doubling the IO.
- **Failure isolation.** If the merge formula has a bug, only
  affected writes fail; (b)'s separate process would touch every
  multi-source row in the system.

(c) is rejected outright: the Rollup re-derives 15m/1h/etc.
from 1m by SQL aggregation. If 1m is inconsistent, 15m/1h/etc.
inherit the inconsistency. Rollup is downstream of the merge
contract, not its custodian.

(b) is a viable fallback if (a) proves too complex. The merge
formula is simple enough (see §3) that this seems unlikely.

### 1.3 The shared merge library

To avoid every writer reimplementing the formula:

- **One shared Rust crate** (likely `crates/ohlcv-merge` or
  similar in task 0012's workspace) exposes a function that
  builds the `ON CONFLICT DO UPDATE` clause for `price_ohlcv`.
- Each writer (Prices Ledger Processor, Soroswap consumer,
  Aquarius consumer, future sources) calls this function. The
  source identifier is the only parameter.
- The merge SQL is a single string template; runtime cost is
  negligible.

```rust
// Sketch.
pub fn upsert_with_merge(source: &str) -> &'static str {
    // Returns the full INSERT...ON CONFLICT DO UPDATE statement
    // template with $1..$N bind slots. Same logical statement for
    // every writer; only the values bound (and the source label)
    // differ at call time.
    UPSERT_MERGE_SQL
}
```

This puts the merge formula in one place, gradeable against this
spec.

## 2. Schema change required: `first_trade_at` / `last_trade_at`

### 2.1 The problem

`price_ohlcv` carries `open` / `close` as `NUMERIC(28,14)` prices
but **no timestamp** indicating which trade those prices came
from. With two sources contributing to the same minute, "what's
the open?" is ambiguous: each source has its own
chronologically-first trade.

Without these columns, the merge has to fall back to one of:

- **Arrival order** — first writer's `open` wins, last writer's
  `close` wins. Order-dependent, not chronologically correct.
  Bad.
- **Tie-break by source label** — pick a canonical source
  (e.g. SDEX always wins for open). Arbitrary; loses the actual
  price of the chronologically-first trade.
- **Don't merge open/close** — keep first arrival's values, only
  merge high/low/volumes. Defensible but ugly.

### 2.2 The fix

Add two columns:

```sql
ALTER TABLE price_ohlcv ADD COLUMN first_trade_at TIMESTAMPTZ;
ALTER TABLE price_ohlcv ADD COLUMN last_trade_at  TIMESTAMPTZ;
-- nullable initially; live writers populate. Backfill task 0022
-- can also populate from the in-memory accumulator (which has
-- the (ledger_seq, op_idx, claim_idx) lex tuple — first/last
-- tuple's wall-clock comes from `LedgerCloseMeta.scp_value.close_time`).
```

Then the merge picks the chronologically-earliest open and the
chronologically-latest close:

```sql
ON CONFLICT (timestamp, asset_id, quote_asset_id, granularity) DO UPDATE SET
    open = CASE
             WHEN EXCLUDED.first_trade_at < price_ohlcv.first_trade_at THEN EXCLUDED.open
             ELSE price_ohlcv.open
           END,
    close = CASE
             WHEN EXCLUDED.last_trade_at > price_ohlcv.last_trade_at THEN EXCLUDED.close
             ELSE price_ohlcv.close
           END,
    first_trade_at = LEAST(price_ohlcv.first_trade_at, EXCLUDED.first_trade_at),
    last_trade_at  = GREATEST(price_ohlcv.last_trade_at, EXCLUDED.last_trade_at),
    -- ... (other column merges below)
    ...
```

### 2.3 Schema-change ADR

This needs an ADR. Options:

- **Addendum to ADR 0003** — add a §"Open / close determinism"
  section to the existing accepted ADR. Pro: one ADR for the
  `price_ohlcv` row identity + open/close handling. Con: amends
  accepted ADR.
- **New ADR 0004** — "`price_ohlcv` carries `first_trade_at` /
  `last_trade_at` for deterministic multi-source merge". Pro:
  one ADR per decision. Con: more ADR overhead for a small
  schema delta.

Recommend **ADR 0004** for cleanliness. Task 0025 spawns the
ADR-acceptance step; either this task accepts it on completion
or it lands as part of task 0012's pre-impl schema work. Defer
the choice to the user's acceptance review.

### 2.4 Impact on tasks 0012 / 0022 / 0024

- **Task 0012**: pre-backfill schema migration gains 2 column
  adds (alongside ADR 0003's `quote_asset_id` change).
- **Task 0022 (archived)**: decode-and-bucket spec §5 needs a
  spec edit — the in-memory accumulator already tracks
  `(ledger, op_idx, claim_idx)` lex order; that maps to
  `closed_at` (the ledger's `scp_value.close_time`) as the
  per-tick wall-clock. Backfill populates `first_trade_at` from
  the lowest-lex tick's `closed_at`, `last_trade_at` from the
  highest. Spec edit lands when task 0012 implements.
- **Task 0024 (archived)**: enrichment SQL unaffected — it
  joins on `quote_asset_id` and ignores the new columns.

## 3. Per-column merge rules

### 3.1 Full merge formula

Given an incoming candle `EXCLUDED` for a `(timestamp, asset_id,
quote_asset_id, granularity)` row that already has `price_ohlcv`
values, the merge is:

| Column             | Merge rule                                                                                                            |
| ------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `open`             | `EXCLUDED.open` if `EXCLUDED.first_trade_at < price_ohlcv.first_trade_at` else `price_ohlcv.open`                      |
| `high`             | `GREATEST(price_ohlcv.high, EXCLUDED.high)`                                                                            |
| `low`              | `LEAST(price_ohlcv.low, EXCLUDED.low)`                                                                                 |
| `close`            | `EXCLUDED.close` if `EXCLUDED.last_trade_at > price_ohlcv.last_trade_at` else `price_ohlcv.close`                      |
| `volume_base`      | `price_ohlcv.volume_base + EXCLUDED.volume_base`                                                                       |
| `volume_quote_usd` | `price_ohlcv.volume_quote_usd + EXCLUDED.volume_quote_usd`                                                             |
| `vwap`             | `(price_ohlcv.volume_quote_usd + EXCLUDED.volume_quote_usd) / NULLIF(price_ohlcv.volume_base + EXCLUDED.volume_base, 0)` |
| `trade_count`      | `price_ohlcv.trade_count + EXCLUDED.trade_count`                                                                       |
| `source`           | `CASE WHEN price_ohlcv.source = EXCLUDED.source THEN price_ohlcv.source ELSE 'aggregated' END`                          |
| `first_trade_at`   | `LEAST(price_ohlcv.first_trade_at, EXCLUDED.first_trade_at)`                                                           |
| `last_trade_at`    | `GREATEST(price_ohlcv.last_trade_at, EXCLUDED.last_trade_at)`                                                          |

### 3.2 VWAP recomputation

VWAP is volume-weighted; merging two candles requires re-deriving
it from the new total volume:

```text
vwap_merged = (volume_quote_usd_a + volume_quote_usd_b)
            / (volume_base_a + volume_base_b)
```

The SQL above implements this. Note that if either contributor
has `volume_quote_usd = 0` (e.g. SDEX backfill before enrichment
runs), the merged VWAP is undercounted by that contribution.
That's correct: VWAP is in USD-denominated terms; if one source
hasn't been enriched yet, the merged VWAP is just the other
source's VWAP. Task 0024's enrichment pass eventually fills the
zero rows, but the merge formula doesn't need to wait — it can
recompute correctly with whatever values are present at each
write moment.

### 3.3 `volume_base` consistency check

A consistency invariant: `volume_base` should be the same unit
across all sources for the same `(asset_id, quote_asset_id)`
pair. Since `asset_id` is the canonical base and all sources
report trades in base asset stroops normalised to 7-decimal
(see task 0022 decode spec §4.1), this holds by construction.
No additional unit-handling needed in the merge.

### 3.4 Source label transition

Three states are possible for `source` post-merge:

```text
existing_source  +  incoming_source  =>  result
─────────────────────────────────────────────────
'sdex'           +  'sdex'           =>  'sdex'           (same source — UPSERT replace from same source, no merge)
'sdex'           +  'soroswap'       =>  'aggregated'     (first multi-source touch)
'aggregated'     +  'soroswap'       =>  'aggregated'     (already multi-source)
```

The SQL `CASE WHEN price_ohlcv.source = EXCLUDED.source THEN
price_ohlcv.source ELSE 'aggregated' END` implements this. Note:
once `'aggregated'`, it stays `'aggregated'` — there is no
de-aggregation operation (would need to know which source contributed
what, which the row doesn't track per §4).

## 4. Per-row source attribution: not stored

### 4.1 Three options revisited

The README listed three options:

| Option | Approach                                                            |
| ------ | ------------------------------------------------------------------- |
| (a)    | Add `sources_seen JSONB` column with per-source breakdown per row.   |
| (b)    | Rely on `current_prices.sources` JSONB at the asset+24h level only.  |
| (c)    | Reconstruct from a separate per-source rows table.                  |

### 4.2 Why (b)

- The **API surface** doesn't expose per-1m-row source breakdown.
  `GET /assets/{id}/ohlcv` returns merged OHLCV; `GET /assets/{id}/price`
  returns `sources` JSONB at the 24h level.
- The **Current Price Updater** reads `price_ohlcv` rows and builds
  the 24h `sources` JSONB by joining... actually wait. Let me
  re-check: how does the Current Price Updater know per-source
  breakdown if it's not on the row?

Re-reading `database-schema-overview.md` §3.3 Current Prices and
the design doc §5.5 VWAP: the per-source breakdown in
`current_prices.sources` JSONB requires the Current Price Updater
to know **which sources contributed how much** over the 24h window.

If `price_ohlcv` rows are merged to `'aggregated'` with no source
breakdown, the Current Price Updater can't reconstruct this.

This is a **real gap**.

### 4.3 Resolution

Two sub-options surface:

- **(b-i)** Current Price Updater reads from a **pre-merge view**.
  Maintain separate per-source 1m rows (option c) and aggregate
  at read time. Doubles storage. Doubles write cost.
- **(b-ii)** Each source maintains its own 24h-volume rolling
  state somewhere lightweight (e.g. a per-source rolling-window
  Redis or an additional `source_24h_volume` table). The Current
  Price Updater reads from there for the `sources` JSONB, not
  from `price_ohlcv`.
- **(a)** Add `sources_seen JSONB` column to `price_ohlcv`
  carrying `{ "sdex": { "volume_base": X, "volume_quote_usd": Y,
  "trade_count": Z }, "soroswap": {...}, ... }`. Allows the
  Current Price Updater to read directly from `price_ohlcv` and
  reconstruct per-source 24h.

**Recommendation: (a) `sources_seen JSONB`**. Reasoning:

- Adds ~50-200 bytes per multi-source row. For 1m candles, total
  rows are bounded (single-digit million per year per asset
  retention horizon). Storage hit is small.
- Eliminates the duplication of (b-i) and the operational
  surface area of (b-ii).
- Concentrates per-source visibility in one schema location.
- Removes the temptation to re-aggregate from per-source rows
  later — the breakdown is already on the row.

This needs to land in the same ADR as the `first_trade_at` /
`last_trade_at` columns (proposed §2.3 as ADR 0004). One ADR
covering all multi-source-merge-related schema changes:
`first_trade_at`, `last_trade_at`, `sources_seen`.

### 4.4 `sources_seen` shape

```json
{
  "sdex": {
    "volume_base": "12345.6789012",
    "volume_quote_usd": "98765.4321098",
    "trade_count": 47,
    "first_trade_at": "2026-02-10T11:00:03Z",
    "last_trade_at":  "2026-02-10T11:00:54Z"
  },
  "soroswap": {
    "volume_base": "5432.10",
    ...
  }
}
```

Numeric values serialised as strings to preserve `NUMERIC(28,14)`
precision (same convention as `current_prices.sources` per
schema doc §3.3 note).

### 4.5 Merge SQL for `sources_seen`

Per-source slot is overwritten by the incoming source (since the
incoming candle is that source's complete 1m contribution for the
minute). Other sources' slots are preserved. Postgres `jsonb_set`:

```sql
sources_seen = jsonb_set(
    COALESCE(price_ohlcv.sources_seen, '{}'::jsonb),
    array[EXCLUDED.source],
    jsonb_build_object(
        'volume_base',       EXCLUDED.volume_base::text,
        'volume_quote_usd',  EXCLUDED.volume_quote_usd::text,
        'trade_count',       EXCLUDED.trade_count,
        'first_trade_at',    EXCLUDED.first_trade_at,
        'last_trade_at',     EXCLUDED.last_trade_at
    ),
    true                                   -- create slot if absent
)
```

### 4.6 Current Price Updater changes

The Current Price Updater Lambda's job becomes: scan the last 24h
of `price_ohlcv` rows, aggregate `sources_seen` across rows for
each asset, write into `current_prices.sources`. Algorithm
straightforward; no longer reads `source` column for breakdown
purposes.

## 5. Backfill interaction

The SDEX backfill (task 0012 / 0022) writes whole-row 1m candles
once per `(asset, quote, minute)` with `source = 'sdex'`. If a
live writer (live SDEX ingestion or Soroswap consumer) later
touches the same row:

- The live writer's UPSERT runs the merge formula in §3.1.
- If incoming `source = 'sdex'` (same source), the row stays
  `source = 'sdex'` and the values are re-merged (e.g. high
  recomputed). This is the same-source case — covered by §3.4.
- If incoming `source = 'soroswap'`, the row transitions to
  `source = 'aggregated'` and the per-column merge runs.

Backfill never needs to know about other sources. Its write is
just "first to land" in the minute (or last, if a live writer
beat it — UPSERT ordering doesn't matter under the merge
formula).

## 6. Concurrency considerations

The merge runs under PG row-level locking (UPSERT is row-atomic).
Concurrent UPSERTs from N writers targeting the same row
serialise on the row lock; each one sees a consistent existing-
row state and applies its merge.

No deadlocks: the lock is acquired and released within one
`INSERT...ON CONFLICT DO UPDATE` statement. No cross-row
ordering.

Enrichment Lambda (task 0024) runs `UPDATE` with `FOR UPDATE SKIP
LOCKED` and is therefore non-conflicting with concurrent
UPSERTs.

## 7. Acceptance criteria for task 0025 (this task)

- [x] Specification note in this task's `notes/` directory.
      → This file.
- [x] Schema implication documented and routed to a new ADR.
      → §2.3 + §4.3 specify ADR 0004 covering `first_trade_at`,
      `last_trade_at`, `sources_seen` columns.
- [x] Live writer implementation guidance for the Prices Ledger
      Processor (task 0012's eventual sibling).
      → §1.3 sketch + §3.1 merge table.

Acceptance gate: user reviews this spec; if accepted, draft ADR
0004 with the three schema columns and route through the normal
ADR acceptance flow.

## 8. Open items + spawned follow-ups

1. **ADR 0004** — needs to be drafted, recording the three
   schema additions (`first_trade_at`, `last_trade_at`,
   `sources_seen`). Either this task drafts it on acceptance,
   or it spawns as a separate small task. Recommend: spawn as
   a follow-up so 0025 stays scoped to the contract spec.
2. **Current Price Updater spec update** — §4.6 says the Updater
   reads `sources_seen` for the per-source 24h breakdown. The
   Updater's own spec (lives in design doc §5.5) should be
   amended when 0004 lands. Not part of 0025.
3. **Per-source rolling state cache** — not adopted (option b-ii
   in §4.3). Mentioning for completeness; if `sources_seen`
   JSONB becomes too heavy at scale, revisit.
4. **Backfill spec edit for `first_trade_at` / `last_trade_at`**
   — task 0022's archived decode-and-bucket spec §5 needs a
   one-line correction (accumulator tracks the wall-clock of
   first/last tick). Lands in task 0012's worklog when
   implementation begins, not as a retroactive edit to the
   archived spec.
