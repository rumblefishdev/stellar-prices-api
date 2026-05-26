---
title: 'volume_quote_usd enrichment pass — design spec (trigger, SQL, idempotency, missing-oracle)'
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [ohlcv, enrichment, oracle, usd-volume, cron-lambda, design]
links:
  - '../README.md'
  - '../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md'
  - '../../archive/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/notes/S-recommendation.md'
  - '../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md'
  - '../../../../docs/database-schema/database-schema-overview.md'
  - '../../../../docs/prices-api-general-overview.md'
history:
  - date: 2026-05-13
    status: mature
    who: okarcz
    note: >
      Design-only first phase. Captures the trigger choice, SQL
      shape, idempotency contract, minute-bucket alignment, and
      missing-oracle behaviour. Implementation lands as a
      follow-up task once 0012's RDS + backfill schema is up.
---

# `volume_quote_usd` enrichment pass — design spec

This note specifies the enrichment pass that fills
`price_ohlcv.volume_quote_usd` for rows the backfill (task 0012)
and live writers leave at 0. ADR 0003's `quote_asset_id` PK column
makes the join clean; the SQL is straightforward and the
remaining design choices are operational (trigger, idempotency,
missing-oracle behaviour).

## TL;DR

| Concern          | Decision                                                                                                                         |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Trigger          | **EventBridge cron Lambda**, hourly cadence. Decoupled from backfill + live writers.                                             |
| Source rows      | `price_ohlcv WHERE volume_quote_usd = 0 AND volume_quote IS NOT NULL`                                                            |
| Join key         | `quote_asset_id` (from ADR 0003) → `oracle_prices.asset_id`                                                                      |
| Minute alignment | Forward-fill the oracle 5m bar onto the OHLCV 1m bars within the same 5m window.                                                 |
| Idempotency      | The `volume_quote_usd = 0` WHERE filter is the idempotency gate. Re-runs are no-ops on enriched rows.                            |
| Missing-oracle   | Leave the row at `volume_quote_usd = 0`; emit `oracle_miss` CloudWatch metric per quote asset; retry on next pass.               |
| Two-hop quotes   | Out of scope for v1 (e.g. AQUA-quoted pairs). Phase 2 follow-up task. v1 enriches when the quote has direct USD oracle coverage. |

## 1. Trigger architecture

Three candidates considered:

| Option                         | Pros                                                                                                     | Cons                                                                                                      |
| ------------------------------ | -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| **A. EventBridge cron Lambda** | Decoupled. Idempotent. Backfill / live writers never block on oracle availability. Operationally simple. | Hourly lag on USD-denominated volume visibility (acceptable per design-doc §5.6 cadence).                 |
| B. End-of-backfill one-shot    | Single batch of computation; no rolling state.                                                           | USD volumes invisible during backfill (12–16 days). Doesn't cover live mode.                              |
| C. Live writer extension       | Real-time USD volume.                                                                                    | Couples backfill correctness to oracle availability. Violates ADR 0003's "defer to enrichment" principle. |

**Adopt Option A.** This matches the existing Oracle Fetcher Lambda
pattern (§5.3 of design doc — cron-driven, decoupled). The hourly
cadence aligns with the Current Price Updater Lambda's own 1-min
cadence: the worst-case lag from trade → USD-denominated
visibility is `1h (enrichment) + 1m (price update)`, which is
adequate for SDEX trades that are non-real-time by definition.

### 1.1 Lambda specifics

- **Runtime**: Rust (`lambda_runtime`), matching the other Lambdas (§5.1).
- **Trigger**: EventBridge `rate(1 hour)`.
- **Memory**: 256 MB (low; this is a SQL-driven batch).
- **Timeout**: 5 minutes (well above the worst-case batch size).
- **Concurrency**: 1 reserved (single-task; no point in parallelism — the work is DB-bound, not CPU-bound).
- **IAM**: read+write on `price_ohlcv`, read on `oracle_prices` and `assets`.

### 1.2 Configuration

| Env var                                 | Default     | Purpose                                               |
| --------------------------------------- | ----------- | ----------------------------------------------------- |
| `ENRICHMENT_BATCH_SIZE`                 | 10 000      | UPDATE batch row limit (avoid long locks).            |
| `ENRICHMENT_MAX_BATCHES`                | 20          | Per-invocation cap; remaining rows roll to next hour. |
| `ENRICHMENT_ORACLE_NAME`                | `reflector` | Which `oracle_prices.oracle_name` row to read.        |
| `ENRICHMENT_QUOTE_FORWARDFILL_WINDOW_S` | 300         | Max staleness of an oracle bar when forward-filling.  |

## 2. SQL contract

The core join, expressed as one UPDATE statement per batch:

```sql
-- Per-batch enrichment UPDATE.
-- Idempotent: WHERE volume_quote_usd = 0 filters out already-enriched rows.
-- Bounded: LIMIT keeps the lock window short for live-writer concurrency.

WITH candidates AS (
    SELECT timestamp, asset_id, quote_asset_id, granularity
      FROM price_ohlcv
     WHERE volume_quote_usd = 0
       AND volume_quote IS NOT NULL
       AND volume_quote > 0
       AND timestamp >= NOW() - INTERVAL '30 days'  -- window guard (see §2.3)
     ORDER BY timestamp DESC, asset_id, quote_asset_id, granularity
     LIMIT $1
       FOR UPDATE SKIP LOCKED                       -- concurrency-safe (see §2.4)
),
priced AS (
    SELECT
        c.timestamp,
        c.asset_id,
        c.quote_asset_id,
        c.granularity,
        -- Forward-fill the oracle bar onto the OHLCV minute.
        -- Pick the most-recent oracle quote at or before the OHLCV
        -- minute, within a forward-fill window (default 5 min).
        (
            SELECT o.price_usd
              FROM oracle_prices o
             WHERE o.asset_id     = c.quote_asset_id
               AND o.oracle_name  = $2
               AND o.timestamp   <= c.timestamp
               AND o.timestamp   >  c.timestamp - $3::interval
             ORDER BY o.timestamp DESC
             LIMIT 1
        ) AS quote_oracle_price_usd
      FROM candidates c
)
UPDATE price_ohlcv p
   SET volume_quote_usd = priced.quote_oracle_price_usd * p.volume_quote
  FROM priced
 WHERE p.timestamp      = priced.timestamp
   AND p.asset_id       = priced.asset_id
   AND p.quote_asset_id = priced.quote_asset_id
   AND p.granularity    = priced.granularity
   AND priced.quote_oracle_price_usd IS NOT NULL
RETURNING p.timestamp, p.quote_asset_id;
```

Parameters: `$1` = batch size, `$2` = oracle_name, `$3` =
forward-fill window.

### 2.1 Why `FOR UPDATE SKIP LOCKED`

If a live writer (or another enrichment invocation due to misfire)
is mid-UPSERT into the same row, the enrichment Lambda would
otherwise block on the row lock. `SKIP LOCKED` makes the Lambda
ignore contended rows for this invocation; they'll be picked up
on the next hourly run. Net: no concurrency hangs, no deadlocks.
(See PG manual on `FOR UPDATE SKIP LOCKED`.)

### 2.2 Why the `timestamp >= NOW() - INTERVAL '30 days'` guard

Two purposes:

1. **Index efficiency**. Partition pruning on `price_ohlcv` is by
   month; the WHERE constraint hands the planner the partitions
   it needs.
2. **Backfill independence**. Historical rows from the SDEX
   backfill (12–16 days running) will land outside the 30-day
   window. A separate **historical enrichment pass** handles
   those: same SQL with no recency guard, invoked as a one-shot
   when the backfill task signals completion (see §4).

### 2.3 Special case: USDC/USDT-quoted rows

USDC and USDT are quote assets where the "USD oracle" path is
either identity or a small depeg correction. Two options:

- **A (this spec)**: treat USDC and USDT identically to any other
  asset — read their `oracle_prices.price_usd` (which the Oracle
  Fetcher Lambda writes via Reflector or another oracle). Result:
  during normal operation `oracle_price ≈ 1.0` and the enrichment
  is approximately the identity. During a depeg, the oracle price
  reflects the depeg and the USD-denominated volume is corrected.
- B: hard-code `volume_quote_usd = volume_quote` for USDC/USDT
  quotes. Simpler but loses the depeg correction.

Adopt A. Uniform code path; the oracle's already responsible for
USD reference; depeg handled.

### 2.4 Concurrency model

Three writers may touch `price_ohlcv`:

1. **Backfill task** (task 0012) — writes whole-row candles, sets
   `volume_quote_usd = 0`.
2. **Live writers** (Prices Ledger Processor, Soroban AMM live) —
   per-ledger incremental UPSERTs, may also write `volume_quote_usd = 0`
   (subject to live writer's own enrichment choice).
3. **Enrichment Lambda** (this spec) — UPDATEs `volume_quote_usd`
   from 0 to a real value.

The PG `INSERT ... ON CONFLICT DO UPDATE` from writers (1) and (2)
serialises on the row lock with the enrichment's UPDATE. Two
collision scenarios:

| Writer race                                                                       | Outcome                                                                                              |
| --------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Writer commits first, then enrichment                                             | Writer sets `volume_quote_usd = 0`; enrichment overwrites with real value. ✅                        |
| Enrichment commits first, then writer's UPSERT replaces `volume_quote_usd` with 0 | Lost work. Next enrichment pass re-fills (idempotent recovery). 🟡 minor inefficiency, no data loss. |

The minor inefficiency in the second case is bounded — at most
one wasted enrichment per affected row per hour. Tolerable; not
worth adding write-side coordination.

## 3. Missing oracle behaviour

When the `priced.quote_oracle_price_usd` is NULL (no oracle bar
in the forward-fill window for that quote asset at that minute):

- The UPDATE `WHERE` clause skips the row (predicate fails).
- Row stays at `volume_quote_usd = 0`.
- A CloudWatch metric increments:
  `EnrichmentOracleMiss { quote_asset_id, oracle_name, granularity }`.
- Next hourly pass re-tries (the row still matches `volume_quote_usd = 0`).

This is the right default: the row stays correctable indefinitely;
no permanent state is recorded. Once the Oracle Fetcher Lambda
backfills the missing oracle bar (e.g. recovered from a Reflector
outage), the next enrichment pass picks it up.

### 3.1 Quote assets without any direct USD oracle coverage

Some exotic quote assets (AQUA, esoteric DEX tokens) may have **no**
USD oracle bar at all — never any rows in `oracle_prices` for that
`asset_id`. Rows with such quotes accumulate at
`volume_quote_usd = 0` indefinitely.

For v1, **leave them at 0**. The Current Price Updater computes
asset USD prices from rows with `volume_quote_usd > 0`, so exotic-
quote rows are silently excluded from the USD VWAP — which is
correct: we don't know what they're worth.

A v2 two-hop enrichment (use `(quote / XLM)` price from
`price_ohlcv` itself plus `XLM / USD` from `oracle_prices`) can
extend coverage. Spawn as follow-up; not v1 scope.

## 4. Historical (backfill) enrichment

The hourly Lambda covers the rolling 30-day window. The historical
backfill (12–16 days of SDEX history per task 0022 spec §4)
writes rows outside that window. Two options for backfill USD
enrichment:

- **B1**: After the backfill task signals completion
  (`backfill_progress.status = 'completed'`), trigger a one-shot
  historical pass: same SQL, no recency guard, runs to completion.
  Estimated runtime: ~30–60 minutes for ~25M rows depending on
  oracle row density.
- **B2**: Extend the hourly Lambda's window guard to "all rows
  where `volume_quote_usd = 0` and `oracle_prices` covers the
  minute". Simpler but each hourly invocation scans the full
  table.

Adopt B1. One-shot is cleaner operationally; the hourly Lambda
remains lean. The trigger can be an EventBridge rule on the
`backfill_progress` status (via a Lambda that watches the column
on a separate cadence — or surfaces the completion via SNS).
Details deferred to task 0012's CDK + task 0024-impl when it
lands.

## 5. Telemetry

CloudWatch metrics emitted per invocation:

| Metric                                | Dimensions                                     | Use                                                    |
| ------------------------------------- | ---------------------------------------------- | ------------------------------------------------------ |
| `EnrichmentRowsEnriched`              | `granularity`                                  | How much work the Lambda did this hour.                |
| `EnrichmentOracleMiss`                | `quote_asset_id`, `oracle_name`, `granularity` | Identifies quote assets with thin oracle coverage.     |
| `EnrichmentRowsRemainingAtVolumeZero` | `granularity`                                  | Saturation indicator — rising = oracle / coverage gap. |
| `EnrichmentBatchDurationMs`           | (none)                                         | Performance / lock-contention indicator.               |

Alarm on `EnrichmentRowsRemainingAtVolumeZero` if it grows
monotonically over 24 hours — that's a sign of a structural
oracle-coverage gap, not a transient missing bar.

## 6. Failure modes

| Failure                                                                     | Behaviour                                                                               |
| --------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| DB unavailable                                                              | Lambda fails; EventBridge retries per its default. Next invocation picks up same rows.  |
| Oracle Fetcher Lambda hasn't run recently                                   | `priced.quote_oracle_price_usd` NULL → rows skipped → retry on next pass. Self-healing. |
| Backfill writes wholesale `volume_quote_usd = 0` over already-enriched rows | Next hourly pass re-enriches. ~1h lost data on enriched USD volume. Logged at INFO.     |
| Schema drift (column renamed, etc.)                                         | UPDATE fails fast at startup. Lambda alarms; manual intervention.                       |

## 7. Acceptance criteria for the implementation task (post-0012)

When the actual Lambda + integration lands as a follow-up impl
task, it should satisfy:

1. EventBridge cron Lambda exists with the schema in §2 wired up.
2. CDK + IAM matches §1.1 / §1.2.
3. Re-running on already-enriched rows produces zero changes
   (idempotency test).
4. Rows with missing oracle stay at `volume_quote_usd = 0`,
   `EnrichmentOracleMiss` metric increments.
5. After full SDEX backfill + a one-shot historical enrichment
   pass, `current_prices.volume_24h_usd` for at least 3
   XLM-quoted assets reflects SDEX-sourced volume (>0 and
   credible against Horizon's historical aggregates).
6. CloudWatch metrics from §5 are emitted and visible in the
   dashboard.

These are the implementation task's acceptance criteria, not
this design task's. This task is complete when this spec is
reviewed and the follow-up implementation task is spawned.

## 8. Open items

- **Oracle source selection** — §1.2 defaults to `reflector`. If
  the Oracle Fetcher Lambda writes multiple oracle sources (e.g.
  also Chainlink), the enrichment Lambda picks one. Could
  alternatively prefer-then-fallback. Decide when the Oracle
  Fetcher's full source list is finalised; defaulting to
  `reflector` is safe for v1.
- **`forward-fill window` vs `linear interpolation`** — §1.2's
  forward-fill default is simple. Interpolation might be more
  accurate for sparse oracle cadences. Defer to operational
  experience; default to forward-fill.
- **Two-hop enrichment** — §3.1 mentions a v2 path for exotic
  quotes. Will need its own task when it's prioritised; not
  blocking 0024's main acceptance criteria.
