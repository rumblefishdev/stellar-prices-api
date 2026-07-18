# prices-ledger-processor

Live ingestion of Stellar ledgers into `prices.price_ohlcv_1m` (task 0038).

An SQS **doorbell** triggers a **doorbell-cursor reconcile loop** (mirroring BE's
production indexer): read cursor → derive the next Galexie S3 key → fetch →
decode → extract + bucket → write OHLCV candles to the shared Hetzner ClickHouse
over mTLS → advance the cursor **last**.

## What it reuses (no drift)

The decode → extract → canonicalise → bucket → write pipeline is **not**
reimplemented here — it is [`prices-ingest-core`](../prices-ingest-core), the
same tested code the SDEX backfill (`sdex-backfill`) runs. Live and backfill
therefore emit byte-identical `prices.price_ohlcv_1m` rows: same surrogate
`asset_id`s (via the shared `AssetRegistry`, with SAC→classic collapse), same
preferred-quote orientation, same `Decimal(38,14)` scaling, same `version`.

This crate owns only the **transport seams**:

| Seam | Local (default) | Production (`lambda` feature) |
|------|-----------------|-------------------------------|
| `object_fetcher` | `LocalDiskFetcher` (fixtures) | `S3Fetcher` (`aws-sdk-s3` GetObject) |
| `sink` | `ClickHouseSink::plaintext` / `CountingSink` | `ClickHouseSink::from_lambda_env` (mTLS via [`prices-clickhouse::mtls`](../prices-clickhouse), task 0052) |
| `cursor` | `StubFileCursor` | `StubFileCursor` (CH-backed cursor is a follow-up — G-note Part D.1) |

## Cargo features

- `default` — lean: the local fixture runner only. No rustls / lambda / aws SDK.
- `aws-mtls` — the remote ClickHouse-over-mTLS sink.
- `lambda` — full Lambda: `lambda_runtime` SQS runtime + `aws-sdk-s3` fetch +
  `aws-mtls`. Build: `cargo build -p prices-ledger-processor --features lambda`
  (then `cargo lambda` for the `provided.al2023` ZIP, ADR 0006).

## Run it locally

```bash
# parse + bucket only, no DB (uses bundled fixtures)
cargo run -p prices-ledger-processor --bin prices-cli -- --cursor 62460539 --dry-run

# write into a local Docker ClickHouse (apply the prices schema first via
# `prices-clickhouse-init`)
CLICKHOUSE_URL=http://localhost:8123 \
  cargo run -p prices-ledger-processor --bin prices-cli -- --cursor 62460539
```

Fixtures live under `fixtures/ledgers/<derived-key>` and are **gitignored**
(large binary Galexie objects copied locally); the integration test self-skips
when they are absent.

## Event contract (production)

Doorbells reach the Lambda via **SNS fan-out** (2026-06-10 cross-team decision):

```
ledger PutObject → S3 ObjectCreated
                 → SNS topic (BE-owned, on stellar-ledger-data)
                 ├─ SQS  ledger-ingest-{env}  (BE)
                 └─ SQS  prices-ingest-{env}   (prices-api) + DLQ → this Lambda
```

The SQS message **body is ignored** — order comes from the cursor + S3 contents,
not delivery order (so no FIFO needed). `reservedConcurrency = 1` (CDK) keeps
runs serial, which is the ordering guarantee. Adding the prices SNS subscription
on BE's bucket is a cross-team change (tracked by task 0050); the CDK wiring is
already in `infra/` (prepare-only).

## Environment variables

Injected by CDK at deploy from `/platform/{env}/*` SSM (deploy-time handshake —
the Lambda reads only env vars, never SSM at runtime):

| Var | Used by | Meaning |
|-----|---------|---------|
| `BUCKET_NAME` | `S3Fetcher` | BE's `stellar-ledger-data` bucket |
| `CH_DOMAIN` | `prices-clickhouse::mtls` | Caddy host fronting the Hetzner cluster |
| `MTLS_SECRET_NAME` | `prices-clickhouse::mtls` | Secrets Manager bundle (cert+key+ca) name |
| `CURSOR_FILE` / `INITIAL_CURSOR` | `StubFileCursor` | cursor checkpoint path / cold-start seed |
| `MAX_ITERATIONS` | reconcile loop | max contiguous ledgers per invocation (default 16) |
| `CLICKHOUSE_URL` | local CLI only | plaintext local ClickHouse endpoint |

## Known follow-ups

- **Cross-invocation intra-minute aggregation.** Candles aggregate across one
  contiguous run; a minute split across two separate runs lands as two
  `version`-keyed rows (ReplacingMergeTree keeps the latest). Same characteristic
  the backfill has across partition boundaries; a periodic re-aggregation /
  AggregatingMergeTree is the fix.
- **CH-backed cursor** (G-note Part D.1) — replace the file cursor.
- **rustls dedup** — `aws-sdk-s3` pulls an older rustls 0.21 alongside our
  0.23.40; unify to shrink the Lambda ZIP.
