# SDEX Historical Backfill — Runbook

Local workstation tool for backfilling SDEX trade history into ClickHouse.
Mirrors the block explorer's `backfill-runner` pattern (BE ADR 0010).

## Prerequisites

- **Rust toolchain** (stable, ≥ 1.85)
- **AWS CLI** (`aws --version`) — no credentials needed; reads an anonymous public bucket
- **Docker** + **Docker Compose** — for local ClickHouse

## First-time setup

```bash
# 1. Start local ClickHouse (schema applies automatically via init.sql).
docker compose up -d clickhouse

# 2. Verify ClickHouse is healthy.
docker compose exec clickhouse clickhouse-client --query "SELECT count() FROM system.tables WHERE database = 'prices'"
# Expected: 4 (assets, price_ohlcv_1m, backfill_sdex_ledgers, backfill_progress)

# 3. Build the backfill CLI in release mode.
cargo build --release -p sdex-backfill
```

## Phase 1 — recent 6 months (Tranche 1)

```bash
# ~1.1M ledgers ≈ 6 months at ~5s ledger cadence.
# Replace TIP with the current ledger sequence from Horizon.
TIP=$(curl -s 'https://horizon.stellar.org/ledgers?order=desc&limit=1' \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['_embedded']['records'][0]['sequence'])")

target/release/sdex-backfill \
    --start $((TIP - 1100000)) \
    --end $TIP \
    --verbose
```

Wall-clock estimate: ~1 hour decode + archive sync overhead → half to full day.

## Subsequent phases

Run older ranges in any order. The resume logic short-circuits
already-processed ledgers, so overlapping invocations are safe.

```bash
# Phase 2: next ~12M ledgers (~1 year older)
target/release/sdex-backfill \
    --start $((TIP - 12000000)) \
    --end $((TIP - 1100001)) \
    --verbose

# Phase N: from ledger 1 to wherever the previous phase started
target/release/sdex-backfill \
    --start 1 \
    --end $((TIP - 12000001)) \
    --verbose
```

## Stop / pause / resume

`Ctrl-C` interrupts the CLI. The next invocation resumes from the
last fully-indexed partition. Completed ledger sequences are tracked
in the `prices.backfill_sdex_ledgers` ClickHouse table.

Re-running the same `--start`/`--end` range skips partitions that
are fully in the DB. Partially-processed partitions are re-downloaded
and re-indexed (ReplacingMergeTree deduplicates re-inserted rows).

## Inspect progress

```bash
# Count completed ledgers
docker compose exec clickhouse clickhouse-client --query \
    "SELECT count(), min(sequence), max(sequence) FROM prices.backfill_sdex_ledgers"

# Count OHLCV rows
docker compose exec clickhouse clickhouse-client --query \
    "SELECT count(), min(timestamp), max(timestamp) FROM prices.price_ohlcv_1m"

# Count discovered assets
docker compose exec clickhouse clickhouse-client --query \
    "SELECT count() FROM prices.assets"
```

## Configuration

| Flag                | Env var             | Default                 | Description                               |
| ------------------- | ------------------- | ----------------------- | ----------------------------------------- |
| `--start`           | —                   | required                | First ledger (inclusive)                  |
| `--end`             | —                   | required                | Last ledger (inclusive)                   |
| `--clickhouse-url`  | `CLICKHOUSE_URL`    | `http://localhost:8123` | ClickHouse HTTP endpoint                  |
| `--temp-dir`        | `BACKFILL_TEMP_DIR` | `.temp/sdex-backfill`   | Scratch directory for partitions          |
| `--keep-partitions` | —                   | false                   | Keep downloaded partitions after indexing |
| `--verbose` / `-v`  | —                   | false                   | Enable per-partition info logs            |

## Writing to Hetzner (direct-write, ADR 0009)

The earlier stage-then-push model (a separate task-0028 cloud-push tool) is
**superseded**. `sdex-backfill` now writes `prices.*` **directly** to the
Hetzner cluster over the task-0052 mTLS client with `--transport hetzner` (no
local mirror, no separate push step), so `/backfill/status` updates in real
time. See §1 "SDEX + Soroban AMM historical backfill" in
[`running-ingestion-components.md`](running-ingestion-components.md) for the
`--transport` / `--mode` / `CH_DOMAIN` + `MTLS_*_PATH` invocation.

## Troubleshooting

**`aws s3 sync` fails repeatedly:** Check network connectivity.
The CLI retries 3 times with exponential backoff. The Stellar
public archive bucket is anonymous — no credentials needed.

**ClickHouse connection refused:** Ensure the Docker container is
running (`docker compose ps`). The HTTP port is 8123.

**Partition "S3 incomplete" warnings:** The latest partition at the
tip of the chain may not be fully uploaded yet. The CLI skips it
and logs a warning. Re-run the same range later once the archive
catches up.
