# Seed `prices.pool_registry` from the Soroswap API — Runbook

One-off operator tool (task 0079) that fills `prices.pool_registry` with the
current AMM pool set fetched from the Soroswap `/pools` API — the fast way to
give the live ledger-processor (task 0078) the pool→token classification it needs
to price live AMM swaps, instead of a multi-day historical ledger replay (0053).

**What it writes:** one table, `prices.pool_registry` — ~500 small classification
rows (`contract_id, venue, token0, token1, pool_type`). It does **not** write any
price data; it's a metadata seed. Once seeded, the live processor resolves AMM
pools and writes real candles to `prices.price_ohlcv_1m` itself.

**Idempotent:** `pool_registry` is a `ReplacingMergeTree` on `contract_id`, so
re-running replaces rather than duplicates. Safe to re-run any time.

## Prerequisites

- **Rust toolchain** (stable, matches the workspace).
- **`SOROSWAP_API_KEY`** — a Soroswap API bearer key, stored in the gitignored
  `.env.local` at the repo root as `SOROSWAP_API_KEY=…`. Get one via the API's
  self-serve auth (register → login → `POST /api-keys/generate`). **Secret —
  never print, echo, or commit it.**
- **For the prod (Hetzner) write only:** the `prices_writer` mTLS bundle as three
  PEM files on the workstation — client cert, client key, CA cert (the same
  writer credential the ledger-processor Lambda uses; fetch from Secrets Manager
  / the cert-issuance setup). For local runs, none of this is needed.

## Step 1 — dry run (no ClickHouse, always do this first)

Confirms the fetch + venue-aware mapping without touching any database:

```bash
# Load the key into this command's env only (nothing echoes the value).
set -a; . ./.env.local; set +a

cargo run -q -p pool-registry-seed -- --dry-run --network mainnet
```

Expected (counts vary as pools are created):

```
DRY RUN — would write 521 pool_registry rows:
  aquarius: 310
  phoenix: 12
  soroswap: 199
  (20 pool(s) skipped for unknown poolType)
```

The skipped pools are Aquarius `concentrated` (held pending task 0080) — see
**Notes**. `INFO`/`WARN` lines list each skipped pool; add `RUST_LOG=info` for
per-venue fetch counts.

## Step 2 — seed a local ClickHouse (optional sanity check)

```bash
docker compose up -d clickhouse            # schema applies via init.sql
set -a; . ./.env.local; set +a
cargo run -q -p pool-registry-seed -- --network mainnet --ch-url http://localhost:8123

# Verify
docker compose exec clickhouse clickhouse-client \
  --query "SELECT venue, count() FROM prices.pool_registry FINAL GROUP BY venue ORDER BY venue"
```

## Step 3 — seed the Hetzner production ClickHouse (mTLS direct-write)

> **Prod write.** This inserts into the shared prod CH `prices.*` tenant over the
> `prices_writer` credential (BE's `default.*` is untouched). No container
> restart, no schema change. Run Step 1 first; do this deliberately.

```bash
set -a; . ./.env.local; set +a

cargo run --release -p pool-registry-seed -- \
  --network mainnet \
  --ch-domain ch.sorobanscan.rumblefish.dev \
  --mtls-cert-path /path/to/prices_writer.crt \
  --mtls-key-path  /path/to/prices_writer.key \
  --mtls-ca-path   /path/to/ca.crt
```

The CLI opens an mTLS connection to Caddy:443 (which CN-maps the cert to the
`prices_writer` CH user), runs a `SELECT 1` preflight, then `INSERT INTO
prices.pool_registry`. On success it prints the per-venue row counts.

### Verify

Connect to the prod ClickHouse host over the operator's SSH access (see the
internal access notes — host + key are not committed here), then run:

```bash
docker exec -i app-clickhouse-1 clickhouse-client -q \
  "SELECT venue, count() FROM prices.pool_registry FINAL GROUP BY venue ORDER BY venue"
```

Expect ~`soroswap 199 / phoenix 12 / aquarius 310` (numbers drift over time).

## Spot-check (optional, recommended before the first prod seed)

Independently confirm the API's pool addresses are genuine on-chain AMM pool
contracts by matching a sample pool's WASM hash against the known-good per-venue
hash (task 0018/0034). Requires the [Stellar CLI](https://developers.stellar.org/docs/tools/cli).

```bash
PP="Public Global Stellar Network ; September 2015"
RPC="https://mainnet.sorobanrpc.com"
# <POOL> = any pool address from the API for that venue
stellar contract fetch --id <POOL> --rpc-url "$RPC" --network-passphrase "$PP" \
  --out-file /tmp/pool.wasm
sha256sum /tmp/pool.wasm
```

Expected on-chain WASM by venue:

| Venue                         | Expected WASM sha256                                               |
| ----------------------------- | ------------------------------------------------------------------ |
| soroswap                      | `18051456816b66f12e773a56f77c5794fac1b1fb7ab6e22d4fad5a412770f73e` |
| phoenix (xyk)                 | `167ab414a226427de34c19947ef9c5cf38c6c0ed91ecf9392f7cef3278ff506c` |
| aqua (xyk / constant-product) | `ae0da5a84b15805c5c7931ac567a8d1b34be3f26b483993d9ff80cb2c3de9852` |

A match proves the address is the expected pool contract; a mismatch means the
API returned a contract of an unexpected type — investigate before seeding it.
(Verified 2026-07-03 for one live pool per venue.)

## Notes

- **Venue-aware seeding.** `pool_type` is only used by Phoenix dispatch, so:
  Phoenix seeds `xyk` only (stable extractor unimplemented); Soroswap seeds all;
  Aquarius seeds `xyk`+`stable` (its extractor reads tokens inline). Aquarius
  `concentrated` pools are **held back** pending swap-event-shape verification —
  see task 0080. They land only once that task confirms the extractor handles
  them.
- **Ongoing coverage.** This seeds _pre-existing_ pools. Pools created after
  go-live are picked up by the live processor's own factory-event stream and
  persisted by the asset-discovery worker (task 0069). Re-run this seeder any
  time to reconcile against the API's current set.
- **Not a substitute for the historical OHLCV backfill.** For historical AMM
  price _candles_, use the 0053 backfill; this only seeds the registry (live
  pricing).
- **Secret hygiene.** The key is read from `SOROSWAP_API_KEY` and only sent in the
  `Authorization` header — never logged. Keep `.env.local` local and uncommitted.
