# Runbook — Events-sourced AMM reprice (CH-to-CH, no ledger re-download)

**Task:** 0097. **Audience:** operator running against `ch-prod-01` (or a local
Docker ClickHouse loaded with BE tables). Follow top to bottom.

## What this does

Repices historical **AMM candles** (Soroswap / Phoenix / Aquarius) by reading
BE's ClickHouse `default.soroban_events` and running the events through the
**same live extraction pipeline** the ledger-processor uses — a ClickHouse-to-
ClickHouse reprice. **No ledger archive is downloaded.** Use it to recover AMM
candles a live extractor bug dropped (the ~824k Soroswap swaps from task 0096),
or any AMM coverage gap where BE still holds the events.

The tool is the `events-backfill` binary in this repo. It writes
`prices.price_ohlcv_1m` per source, idempotently (ReplacingMergeTree by
`version`), so a re-run over any range only replaces — never double-counts.

> **⚠ Writing `1m` is only half the job.** `prices.price_ohlcv_1m` is a transient
> 7-day feeder — the nightly cleanup drops its partitions and the live rollup MVs
> ignore backfilled rows. The store of record is the coarse tables
> (`1h/4h/1d/1w/1M`), filled for historical data by an explicit **pre-roll**. A
> repriced range isn't done until it's pre-rolled. Same rule and sequence as the
> SDEX/Soroban backfill — see task 0090 and
> [`fix-backfill-history-loss-and-rerun.md`](fix-backfill-history-loss-and-rerun.md).

## Preconditions

1. **`prices.pool_registry` is seeded.** The reprice reads events only for the
   AMM contracts in the registry (that filter is what makes it fast and what
   preserves each extractor's full event group). An empty registry reprices
   nothing — the tool errors out. Seed it first:
   [`seed-pool-registry.md`](seed-pool-registry.md).
2. **Run identity.** The single ClickHouse client reads `default.*` (BE tables)
   **and** writes `prices.*`. The prices mTLS user cannot read `default.*`, so
   run on the Hetzner host as the `default` user against `localhost:8123`.
   Connect: [[hetzner-ch-prod-ssh-access]].
3. **Cleanup rule `prices-production-cleanup` is DISABLED** and stays disabled
   until after the pre-roll (step 4) — otherwise the 02:00 UTC cleanup drops the
   `1m` rows before they're rolled up.

## Steps

### 1. Dry-run to sanity-check coverage

```bash
CLICKHOUSE_USER=default CLICKHOUSE_PASSWORD=<pw> \
cargo run --release -p events-backfill -- \
  --start 50457424 --end 63352611 \
  --clickhouse-url http://localhost:8123 \
  --dry-run --verbose
```

Prints per-source tick counts without writing. Compare the `soroswap` tick count
against the raw swap count in `soroban_events` for the same range (hand the
per-source query to the operator — [[feedback-user-runs-prod-ch-queries]]).

### 2. Run the reprice

Drop `--dry-run`. Bound the range to the AMM era (`--start` at/above the Soroban
activation ledger `50457424`; `--end` below the SDEX live floor to respect the
operator's disjoint-range rule). Chunks default to 320k ledgers, each read →
classified → flushed → written before the next.

```bash
CLICKHOUSE_USER=default CLICKHOUSE_PASSWORD=<pw> \
cargo run --release -p events-backfill -- \
  --start 50457424 --end 63352611 \
  --clickhouse-url http://localhost:8123 --verbose
```

Idempotent — safe to re-run the whole range or any sub-range after an
interruption. New surrogate `asset_id`s are written to `prices.assets` per chunk.

### 3. Verify `1m`

Confirm non-zero `soroswap` (and `phoenix`/`aquarius`) candles landed for the
range, per source:

```sql
SELECT source, count() AS candles, min(timestamp), max(timestamp)
FROM prices.price_ohlcv_1m
WHERE source IN ('soroswap','phoenix','aquarius')
  AND timestamp BETWEEN <start_ts> AND <end_ts>
GROUP BY source ORDER BY source;
```

### 4. Pre-roll into the coarse tables, then re-enable cleanup

Roll the repriced `1m` up to `1h/4h/1d/1w/1M` with the **incremental,
non-truncating** pre-roll scoped to this reprice:
[`schema/preroll-amm-reprice.sql`](../../packages/prices-clickhouse/schema/preroll-amm-reprice.sql).

Neither other script fits, and both are actively wrong here:

| Script                    | Why NOT this one                                                                                                                                                                     |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `preroll.sql`             | Full rebuild expecting TRUNCATE-d coarse — wipes every already-pre-rolled row (the 0090 history-loss incident).                                                                      |
| `preroll-incremental.sql` | Bounded to the pre-Soroban SDEX tail `[genesis, activation)` — the _complement_ of the range this reprice touches. Rolls the wrong side of the boundary; touches nothing 0097 wrote. |

`preroll-amm-reprice.sql` scopes every statement to
`source IN ('aquarius','phoenix','soroswap')` and the repriced window. Because
`source` is part of the table key (`ORDER BY (asset_id, quote_asset_id, source,
timestamp)`), SDEX coarse — including the expensive pre-Soroban tail — cannot be
touched by it.

> ⚠️ The script is a **draft that has never been run against prod**, and it
> carries two `OPEN QUESTION` blocks (RMT version ties on corrected
> aquarius/phoenix rows; buckets straddling `{end_ts}`). Work its §0 pre-flight
> first and settle both before executing — do not run it on the strength of this
> runbook alone.

Only after the pre-roll verifies (script §5): **re-enable
`prices-production-cleanup`.**

## How it works (the reuse guarantee)

`events-backfill` does **not** reimplement extraction. It maps each
`soroban_events` row into a `RawSorobanEvent` (the `topics_xdr` / `data_xdr`
columns are already the typed-JSON SCVal shape the live decoder emits) and calls
`prices_ingest_core::process_soroban_event_rows` — the same
`classify_amm_groups` → `dispatch` → `amm_trade_to_tick` chain the live
processor runs. Repriced candles are therefore byte-identical to what live would
have written from the original ledger (same `asset_id`s, SAC collapse, canonical
orientation, decimals, RMT `version`). This is the reusable "reprice from BE
events" path for any future extractor gap.
