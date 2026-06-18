# prices-clickhouse

Schema + connection layer for the `prices` ClickHouse database. Mirrors BE's
`crates/db-clickhouse` layout: a single embedded `schema/init.sql` is the source
of truth, applied idempotently by the `prices-clickhouse-init` binary.

This crate stands up the schema and hands out a configured `clickhouse::Client`.
It deliberately owns **no** row structs or writers — the backfill / extractor
crates (`sdex-backfill`, the venue extractors) own their own write path.

## Layout

```
packages/prices-clickhouse/
├── Cargo.toml
├── README.md
├── schema/
│   ├── init.sql       # DATABASE + all prices.* tables (the source of truth)
│   ├── seed.sql       # canonical backfill_progress streams (idempotent INSERT)
│   ├── views.sql      # read-surface views (price_usd_series, usd_reference)
│   ├── rollups.sql    # production refreshable-MV rollup chain (applied separately)
│   └── preroll.sql    # deterministic full-range _1m → _15m…_1M re-aggregate (measurement)
└── src/
    ├── lib.rs                         # Config, client(), apply_init_sql / apply_seed / apply_sql, embedded SQL
    ├── mtls.rs                        # mTLS transport (feature `aws-mtls`) — see below
    └── bin/prices-clickhouse-init.rs  # CLI schema applier (tables + seed + views)
```

## Tables (`prices.*`)

| Table | Engine | Partition | Written by |
|-------|--------|-----------|-----------|
| `assets` | `ReplacingMergeTree(updated_at)` | — | backfill registry / Asset Discovery |
| `price_ohlcv_1m` | `ReplacingMergeTree(version)` | `toYYYYMM(timestamp)` | backfill (per-source) / Ledger Processor |
| `price_ohlcv_15m`…`_1M` | `ReplacingMergeTree(version)` | `toYYYYMM(timestamp)` | rollup chain / backfill pre-roll |
| `current_prices` | `ReplacingMergeTree(updated_at)` | — | Current Price Updater (not backfilled) |
| `oracle_prices` | `ReplacingMergeTree` | `toYYYYMM(timestamp)` | backfill REFLECTOR/REDSTONE / Oracle Fetcher |
| `backfill_sdex_ledgers` | `ReplacingMergeTree` | — | backfill (resume cursor) |
| `backfill_progress` | `ReplacingMergeTree(updated_at)` | — | backfill streams |

The `assets`, `price_ohlcv_1m`, and `backfill_sdex_ledgers` column layouts are a
**contract** with `sdex-backfill/src/sink.rs` (positional `clickhouse::Row`
inserts) — do not retype/reorder without updating those structs.

## Quick start (local)

```bash
# 1. Bring up local ClickHouse (prices-api docker-compose) — db 'prices'
docker compose up -d clickhouse

# 2. Apply the schema
export CLICKHOUSE_URL=http://localhost:8123
export CLICKHOUSE_USER=default
export CLICKHOUSE_PASSWORD=clickhouse
cargo run -p prices-clickhouse --bin prices-clickhouse-init

# 2b. (optional) also create the production refreshable-MV rollup chain
cargo run -p prices-clickhouse --bin prices-clickhouse-init -- --rollups

# 3. Verify (expect 12 tables in db 'prices')
docker exec <ch-container> clickhouse-client -q \
  "SELECT count() FROM system.tables WHERE database='prices'"
```

## Remote (mTLS) connection

The default build is plaintext-only (`Config` + `client()`, for local Docker /
tests). Talking to the production Hetzner ClickHouse goes over HTTPS + mTLS to
the Caddy reverse proxy, behind the **`aws-mtls`** cargo feature (task 0052):

```toml
prices-clickhouse = { path = "...", features = ["aws-mtls"] }
```

Caddy validates the client-cert chain, maps the cert CN to a CH user via
`CLICKHOUSE_CN_USER_MAP`, and re-applies that identity upstream — so no
`X-ClickHouse-User` / Basic Auth is set client-side (Caddy strips both). The
CH user (and thus privileges) is whatever your cert CN maps to; see ADR 0007
§3.5 and the cert-issuance procedure in task 0063.

### Env-var contract

`client_from_lambda_env(database)` — the cold-start convenience — reads:

| Var | Set by | Purpose |
|-----|--------|---------|
| `MTLS_SECRET_NAME` | CDK (task 0011) | Secrets Manager secret holding the bundle JSON `{cert, key, ca}` (all PEM) |
| `CH_DOMAIN` | CDK (task 0011) | Caddy hostname; the client connects to `https://{CH_DOMAIN}` |
| `AWS_SESSION_TOKEN` | Lambda runtime | Auth header for the Parameters & Secrets Extension fetch |

Both `MTLS_SECRET_NAME` and `CH_DOMAIN` are required; a missing/empty value
fails at init with `MtlsError::MissingEnv` (surfaces as a CW `Init Errors`
metric, never a half-configured client). The bundle is fetched from the **AWS
Parameters & Secrets Lambda Extension** (`http://localhost:2773`), not the SDK,
so warm containers hit its in-process cache — no Secrets Manager API call on the
hot path. Set `PARAMETERS_SECRETS_EXTENSION_CACHE_ENABLED=true` on the function.

> **Off-Lambda use:** the extension endpoint only exists inside a Lambda runtime.
> From a workstation, fetch the bundle yourself (AWS CLI/SDK), build an
> `MtlsBundle { cert_pem, key_pem, ca_pem }`, and call `client_with_mtls(domain,
> &bundle, database)` directly.

### Build once, reuse

Build the client **once in Lambda global init and clone it per invocation** — do
not rebuild per request. The returned `clickhouse::Client` is cheap to clone:
under the hood `hyper_util`'s legacy client owns an `Arc`-shared connection pool,
so cloning shares warm, pooled TLS connections and amortises the ~80–130 ms
cross-cloud TLS handshake. Rebuilding per request throws that away and
re-handshakes every time.

Note the amortisation only holds while a pooled socket survives: the pool's idle
timeout (8 s) closes quiet connections, so back-to-back invocations reuse the
warm socket but invocations spaced further apart than the idle window pay a fresh
handshake. Sharing the client is still the right default — it removes per-call
connector setup and keeps the pool warm under load — but it is not a guarantee of
zero handshakes on a sparse traffic pattern.

```rust
// global init (cold start) — once
let ch = prices_clickhouse::mtls::client_from_lambda_env("prices").await?;
// per invocation — clone the warm handle
let ch = ch.clone();
```

## Rollups: MV chain vs. pre-roll

- **Live path** — `rollups.sql` defines 6 refreshable MVs (`_1m → _15m → … →
  _1M`), each re-aggregating a bounded recent window from the previous
  granularity's `FINAL`. Mechanism finalised in task 0051; needs CH ≥ 23.12.
- **Backfill / historical** — `preroll.sql` re-aggregates the whole range from
  `_1m FINAL` into each coarser table (one row per bucket). Used by the task
  0060 sizing measurement so the coarse tables are populated deterministically,
  independent of the live MVs' time window.

## Notes

- `init.sql` is intentionally MV-free so schema-apply stays version-agnostic.
- Schema design: `docs/database-schema/database-schema-overview.md`, ADRs 0003 /
  0004 / 0007.
