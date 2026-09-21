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
  go-live are learned by the live processor from factory events and, since task
  0291, written back to `prices.pool_registry` (new rows only). ⚠️ Before 0291
  nothing persisted them: the asset-discovery worker was meant to (task 0069),
  but its ledger scan has never run on production (task 0256), so the table
  stopped growing on 2026-07-06. To fill a gap, prefer
  [the factory-event route below](#discover-missing-pools-from-factory-events-task-0291),
  which writes only the missing rows.
- **Not a substitute for the historical OHLCV backfill.** For historical AMM
  price _candles_, use the 0053 backfill; this only seeds the registry (live
  pricing).
- **Secret hygiene.** The key is read from `SOROSWAP_API_KEY` and only sent in the
  `Authorization` header — never logged. Keep `.env.local` local and uncommitted.

---

## Discover missing pools from factory events (task 0291)

`events-backfill --discover-pools` reads the AMM factory events in a ledger
range from BE's `default.soroban_events` (Aquarius `add_pool`, Phoenix `create`,
Soroswap `new_pair`, SushiSwap V3 `pool_created` — task 0290), runs them through the same `learn_factory` the live
processor uses, and writes **only the pools `prices.pool_registry` does not
already hold**. No API key, no rewrite of existing rows, no candles. A re-run
writes nothing.

Use it:

- **once, for task 0291**, to add the pools created after the history backfill
  ended (2026-07-06) that the live processor forgot on its cold starts. Run it
  **before** deploying the 0291 ledger-processor: the tool does not depend on
  that deploy, and the deploy's cold start then loads the new rows;
- **before any reprice that `DROP`s a partition** ([task 0286 phase 3](0286-reingest-history.md)):
  a dry run over the month that reports `to_write=0` shows the registry covers
  it; anything else means the drop would delete candles the reprice cannot put
  back;
- **when `prices-production-ledger-processor-unregistered-pool` fires**, over
  the range holding the pool's factory event.

**Run identity** is the same as the reprice
([events-sourced-amm-reprice.md](events-sourced-amm-reprice.md), precondition
2): on the Hetzner host, as ClickHouse `default`, against `localhost:8123`. The
host needs a static build, because its glibc is older than a local toolchain's:

```bash
# Local machine, repo root:
cargo build --release -p events-backfill --target x86_64-unknown-linux-musl
scp target/x86_64-unknown-linux-musl/release/events-backfill <prod-host>:~/events-backfill
```

**Keep the range tight.** The read parses `topics_xdr` for the string-topic
factories (Phoenix and Soroswap leave `signature` NULL), 2-4 s per 320k-ledger
chunk on the shared box. As of 2026-09-17 every missing pool was created after
ledger 63,000,000 (checked per venue over the whole Soroban era), so the
catch-up only needs `63000000` to the tip. **Task 0290 is the exception:**
SushiSwap V3's pools go back to ledger 60,147,305, so its run starts at
`60000000` — the command is [below](#task-0290--sushiswap-v3s-wider-range).

```bash
# On the prod host, under tmux:
read -rs CH_PW
CLICKHOUSE_PASSWORD="$CH_PW" ~/events-backfill --discover-pools \
  --start 63000000 --end <TIP> \
  --clickhouse-url http://localhost:8123 --dry-run
```

Expected on 2026-09-17 (grows if pools are created meanwhile): one
`pool not in prices.pool_registry` line per pool, every one `change="new"`,
then `to_write=42 per_venue={"aquarius": 27, "phoenix": 1, "soroswap": 14}`.
A `change="changed"` line means an existing row would be rewritten — stop and
investigate before the write.

Then drop `--dry-run` to write, and run the dry run once more: it must report
`to_write=0`.

#### Task 0290 — SushiSwap V3's wider range

SushiSwap V3 is the one venue whose pools predate ledger 63,000,000: the first
`pool_created` is at 60,147,305 and the live factory's pools start at ~61.49M,
so the 63M catch-up above misses them. Run it over its own range **once**, then
the 63M catch-up covers it like every other venue:

```bash
# On the prod host, under tmux:
read -rs CH_PW
CLICKHOUSE_PASSWORD="$CH_PW" ~/events-backfill --discover-pools \
  --start 60000000 --end <TIP> \
  --clickhouse-url http://localhost:8123 --dry-run
```

That is ~4.5M ledgers, so ~14 chunks at the default `--chunk-size 320000` and
2-4 s of `topics_xdr` parsing each — a couple of minutes, well inside a tmux
session. It reads the same factory events as the run above; only `--start`
differs.

Expect `per_venue` to carry a `"sushiswap"` entry. The read has **no emitter
filter**, so it learns every generation's pools, not just the live factory's —
which is what this wider range is for: 99 SushiSwap pools have traded all-time
and three of them come from an earlier factory generation that is still trading.
Confirm with the same `FINAL` count below, then drop `--dry-run` to write.

### Verify

```sql
SELECT venue, count() FROM prices.pool_registry FINAL GROUP BY venue ORDER BY venue;
-- 2026-09-17 before: aquarius 488, phoenix 19, soroswap 221
-- expected after:    aquarius 515, phoenix 20, soroswap 235
```

The live processor reads the table only at cold start, so a warm container
keeps pricing without the new rows until its next one. Any code or
configuration deploy of the function forces it, which is why the task-0291
order is discover first, deploy second.
