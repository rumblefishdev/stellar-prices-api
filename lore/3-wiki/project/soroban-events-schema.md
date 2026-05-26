# `soroban_events` — schema, signatures, and event shapes

Reference for the ClickHouse `soroban_events` table maintained by `soroban-block-explorer/backfill-runner`. Use this when designing consumers, enrichment Lambdas, or queries that ingest Soroban contract events for the Stellar prices API.

The live samples backing this doc are in `lore/4-notes/samples/soroban-events/*.jsonl` (50 rows per signature) and were captured from a local ClickHouse populated by the backfill range **ledgers `62078346`–`62079999`** (~1.65k ledgers, ~1.54M events).

## How to refresh the samples

ClickHouse runs in `soroban-block-explorer/docker-compose.yml`. From that repo:

```bash
docker compose stop clickhouse db-clickhouse-init
docker compose rm -f clickhouse db-clickhouse-init
docker volume rm soroban-block-explorer_clickhouse-data
docker compose up -d clickhouse db-clickhouse-init

cargo run --release -p backfill-runner -- \
  --target clickhouse --keep-partitions --verbose \
  run --start 62078346 --end 62079999
```

Then run the queries below against `localhost:8123` (user `default`, password `clickhouse`) or via the ClickHouse UI on `localhost:3488`.

## Table schema

```sql
CREATE TABLE default.soroban_events (
    contract_id      Int64,
    transaction_id   Int64,
    ledger_sequence  Int64,
    event_index      Int16,
    event_type       Int16,
    signature        LowCardinality(Nullable(String)),
    topics_xdr       String CODEC(ZSTD(3)),
    data_xdr         String CODEC(ZSTD(3))
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (contract_id, ledger_sequence, transaction_id, event_index)
```

### Gotchas

- **`topics_xdr` / `data_xdr` are NOT raw XDR.** Despite the column name, the values are JSON-encoded SCVal trees (the same shape the Soroban RPC `/getEvents` returns when `xdrFormat=json`). The names are historical. Parse them with a JSON parser, not `xdr::ScVal::from_xdr`.
- **`contract_id` is an Int64 internal ID**, not the `C…` strkey address. Resolve via `JOIN soroban_contracts c ON c.id = e.contract_id` → `c.contract_id` is the strkey.
- **`transaction_id` is an Int64 internal ID.** Resolve to the 32-byte hash via `JOIN transactions t ON t.id = e.transaction_id` → `t.hash` is `FixedString(32)`; wrap in `hex(t.hash)` for the canonical uppercase hex form.
- **`signature`** is derived from topic 0 only when topic 0 is `{"type":"sym","value":"…"}`. If topic 0 is a `string` (or any other SCVal type), `signature` is `NULL` even though the event is well-formed. See [Null-signature events](#null-signature-events) below.
- **`event_type`** is always `1` (contract event) in the backfilled range. Other types (system, diagnostic) are not stored.
- The engine is `ReplacingMergeTree` keyed by `(contract_id, ledger_sequence, transaction_id, event_index)`. Duplicate rows within a partition collapse on merge. Use `FINAL` or de-dupe in the query if you need point-in-time consistency before merges run.

## Signature distribution (full backfill window)

Top signatures by count over 1.54M events. Full list in `lore/4-notes/samples/soroban-events/signatures-stats.tsv`.

| Signature                                                |  Events | Contracts |     Txs | Notes                                                                |
| -------------------------------------------------------- | ------: | --------: | ------: | -------------------------------------------------------------------- |
| `fee`                                                    | 792,453 |         1 | 550,122 | Emitted only by the **XLM SAC** (`CAS3J7GY…`). One per Soroban tx.   |
| `transfer`                                               | 638,311 |     2,336 |  48,002 | SAC and token contract transfers. Asset coding varies (see below).   |
| `mint`                                                   |  89,725 |       271 |  35,105 | Token / LP mint.                                                     |
| `burn`                                                   |   8,748 |        54 |   8,276 | Token / LP burn.                                                     |
| `set_authorized` / `clawback`                            |   8,820 |        24 |       — | SAC admin events.                                                    |
| `update_reserves`                                        |     743 |        77 |     426 | **AMM pool reserve snapshot** — pair with `trade`/`swap` on same tx. |
| `trade`                                                  |     725 |        74 |     408 | Phoenix-style AMM trade.                                             |
| _(NULL)_                                                 |     625 |        31 |     101 | Topic 0 is `string`, not `sym` — see below.                          |
| `swap`                                                   |     549 |         4 |     249 | Three distinct shapes: CLMM, simple, router.                         |
| `REDSTONE`                                               |      98 |         1 |      98 | Oracle update — bytes-encoded SCVal.                                 |
| `REFLECTOR`                                              |      96 |         3 |      96 | Oracle update — SCVal map with `update_data` vec.                    |
| `score_submitted`, `supply`, `withdraw`, `vault_*`, etc. |    tail |         — |       — | Misc protocol events.                                                |

## Event shapes — payload reference

All `topics_json` / `data_json` examples below are abbreviated copies of real samples. The full sample row (with contract address, tx hash, ledger) lives in `lore/4-notes/samples/soroban-events/<signature>.jsonl`.

### `fee` (XLM SAC fee)

Only emitted by the XLM SAC (`CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA`). Appears once per Soroban tx at `event_index = 0`.

```json
topics: [{"type":"sym","value":"fee"}, {"type":"address","value":"G…payer"}]
data:   {"type":"i128","value":"100"}   // stroops paid
```

### `transfer` — SAC variant (most common)

For SACs the contract emits `transfer` with a 4-element topic vector. Topic 3 is a Stellar asset descriptor `"<CODE>:<ISSUER>"` (native XLM uses `"native"`).

```json
topics: [
  {"type":"sym","value":"transfer"},
  {"type":"address","value":"G…from"},
  {"type":"address","value":"G…to or L…muxed"},
  {"type":"string","value":"USDC:GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"}
]
data: {"type":"i128","value":"100000"}
```

### `transfer` — custom token contract variant

Non-SAC tokens (`soroban_contracts.is_sac = false`) emit the same topic shape; the asset string is the token's chosen code+issuer or symbol.

```json
topics: [{"type":"sym","value":"transfer"}, from_addr, to_addr,
         {"type":"string","value":"UnicornCat:GAOCCFYA…"}]
data:   {"type":"i128","value":"86785647"}
```

### `mint` / `burn`

Same shape as `transfer` but only one party in topics (the holder), plus the asset string.

```json
topics: [{"type":"sym","value":"mint"}, {"type":"address","value":"G…holder"},
         {"type":"string","value":"USDM:GDHD…USDM"}]
data:   {"type":"i128","value":"406960404"}
```

### `swap` — shape A: Uniswap V3-style CLMM pool

Emitted by individual concentrated-liquidity pool contracts (e.g. `CCR2CH4G…`). Single-symbol topic. Data is a map keyed by symbols.

```json
topics: [{"type":"sym","value":"swap"}]
data: {"type":"map","value":[
  {"key":"amount0","value":{"type":"i128","value":"2871373757"}},
  {"key":"amount1","value":{"type":"i128","value":"-439878710"}},
  {"key":"liquidity","value":{"type":"u128","value":"89251760312657"}},
  {"key":"recipient","value":{"type":"address","value":"G…"}},
  {"key":"sender","value":{"type":"address","value":"G…"}},
  {"key":"sqrt_price_x96","value":{"type":"u256","value":"…hex…"}},
  {"key":"tick","value":{"type":"i32","value":-18732}}
]}
```

`amount0`/`amount1` are signed — negative means the pool paid out that token. Token0/token1 ordering must be looked up from the pool's `token_0`/`token_1` storage (not in the event).

### `swap` — shape B: simple in/out

Often emitted by a wrapping contract on the same tx as shape A.

```json
topics: [{"type":"sym","value":"swap"}]
data: {"type":"map","value":[
  {"key":"amount_in","value":{"type":"i128","value":"2871373757"}},
  {"key":"amount_out","value":{"type":"i128","value":"439878710"}},
  {"key":"recipient","value":{"type":"address","value":"G…"}}
]}
```

### `swap` — shape C: Soroswap Router

Emitted by the Soroswap Router contract (`CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` in the sample window). One event per hop of a multi-hop swap. Topic 1 is the hop's `[token_in, token_out]` pair; topic 2 is the trader.

```json
topics: [
  {"type":"sym","value":"swap"},
  {"type":"vec","value":[{"type":"address","value":"C…token_in"},
                          {"type":"address","value":"C…token_out"}]},
  {"type":"address","value":"G…trader"}
]
data: {"type":"vec","value":[
  {"type":"address","value":"C…pair_contract"},
  {"type":"address","value":"C…token_in"},
  {"type":"address","value":"C…token_out"},
  {"type":"u128","value":"930000000"},        // amount_in
  {"type":"u128","value":"423899086439"}      // amount_out
]}
```

The router shape lets you trace the full path without separate Soroswap pair-pool reads, but the pair-level `trade` + `update_reserves` events also fire in the same tx.

### `trade` — Phoenix-style AMM trade

Emitted by AMM pair-pool contracts. Topic 1 = sold token, topic 2 = bought token, topic 3 = trader (G… or C…).

```json
topics: [
  {"type":"sym","value":"trade"},
  {"type":"address","value":"C…sold_token"},
  {"type":"address","value":"C…bought_token"},
  {"type":"address","value":"G…trader"}
]
data: {"type":"vec","value":[
  {"type":"i128","value":"57624430586"},   // amount_sold
  {"type":"i128","value":"49435406506"},   // amount_bought
  {"type":"i128","value":"288122153"}      // fee
]}
```

### `update_reserves` — AMM reserves snapshot

Always paired with `trade` (and sometimes `swap`) in the same tx. The `contract_id` is the AMM pair pool itself. Reserves are i128 in token-native decimals; ordering is `[reserve_0, reserve_1]` per the pool's token ordering.

```json
topics: [{"type":"sym","value":"update_reserves"}]
data:   {"type":"vec","value":[
  {"type":"i128","value":"56282401209368"},
  {"type":"i128","value":"48722126337247"}
]}
```

### `REFLECTOR` — Reflector oracle update

Three known feed contracts in the sample window:

- `CBKGPWGKSKZF52CFHMTRR23TBWTPMRDIYZ4O2P5VS65BMHYH4DXMCJZC` — FX (EUR, GBP, CAD, BRL, JPY, CNY, …, XAU)
- `CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN` — global crypto by symbol (BTC, ETH, USDT, XRP, SOL, USDC, ADA, …)
- `CALI2BYU2JE6WVRUFYTS6MSBNEHGJ35P4AVCZYF3B6QOE3QKOB2PLE6M` — Stellar assets keyed by **contract address** (so the asset identity is on-chain)

Common shape — topic 2 is the feed timestamp (`u64`, milliseconds), data wraps an `update_data` vec of `[key, i128 price]` pairs. Prices are scaled to **14 decimals** (Reflector's standard).

```json
topics: [{"type":"sym","value":"REFLECTOR"},
         {"type":"sym","value":"update"},
         {"type":"u64","value":1775958000000}]
data: {"type":"map","value":[
  {"key":"update_data","value":{"type":"vec","value":[
    {"type":"vec","value":[{"type":"sym","value":"BTC"},
                            {"type":"i128","value":"7231710463185063718"}]},
    {"type":"vec","value":[{"type":"sym","value":"ETH"},
                            {"type":"i128","value":"225591688710028279"}]}
    // … rest of the symbol/address keyed entries
  ]}}
]}
```

The Stellar-assets feed swaps the inner `sym` key for `address`, so the value pair is `[C…asset_contract, i128 price]`. Use this to price assets that have only on-chain identity (no `code:issuer` string).

### `REDSTONE` — RedStone oracle update

Single feed contract in the sample window (`CA526Y2NQWGWVVQ7RFFPGAZMU66PSYJ3UC2MTVAV4ZU7OM5BOPHDXUSG`). Payload is opaque `bytes` (base64) that decodes to a Soroban-XDR `ScVal` map with `updated_feeds → [{package_timestamp, price, write_timestamp}, …]` plus the updater address. To extract prices you must `from_xdr_base64::<ScVal>` the inner bytes; it is **not** plain JSON.

```json
topics: [{"type":"sym","value":"REDSTONE"}]
data: {"type":"bytes","value":"AAAAEQAAAAEAAAACAAAADwAAAA11cGRhdGVkX2ZlZWRz…"}
```

### Null-signature events

About 0.04% of events have `signature IS NULL`. The pattern in the sample window is a contract (`CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX`) that emits a sequence of micro-events for a single logical swap: topic 0 is the **string** `"swap"` (not a symbol), topic 1 is a string key (`"sender"`, `"sell_token"`, `"offer_amount"`, `"actual received amount"`, `"buy_token"`, …), and the data is the single typed value. Consumers that only filter on `signature` will silently drop these — match on `topics_json` JSON content if you need to capture them.

## Useful queries

### Resolve a sample swap to readable form

```sql
SELECT
  e.ledger_sequence,
  e.event_index,
  c.contract_id    AS contract_addr,
  c.is_sac,
  hex(t.hash)      AS tx_hash,
  e.signature,
  e.topics_xdr,
  e.data_xdr
FROM soroban_events e
LEFT JOIN soroban_contracts c ON c.id = e.contract_id
LEFT JOIN transactions      t ON t.id = e.transaction_id
WHERE e.signature = 'swap'
ORDER BY e.ledger_sequence, e.transaction_id, e.event_index
LIMIT 5
FORMAT JSONEachRow;
```

### All AMM events for one tx (trade + update_reserves + transfer)

```sql
WITH (
  SELECT id FROM transactions WHERE hex(hash) = '2964A4F2FE7A9A484EEC60DD2A60A3B99F36EACF123F83E00423325AAA1287E2'
) AS tx_id
SELECT
  e.event_index,
  c.contract_id AS contract_addr,
  e.signature,
  e.topics_xdr,
  e.data_xdr
FROM soroban_events e
LEFT JOIN soroban_contracts c ON c.id = e.contract_id
WHERE e.transaction_id = tx_id
ORDER BY e.event_index
FORMAT JSONEachRow;
```

### Top emitters per signature

```sql
SELECT signature, c.contract_id, count() AS events
FROM soroban_events e
JOIN soroban_contracts c ON c.id = e.contract_id
WHERE signature IN ('swap','trade','update_reserves')
GROUP BY signature, c.contract_id
ORDER BY events DESC
LIMIT 20;
```

## Sample files

| File                                                        | Contents                                                               |
| ----------------------------------------------------------- | ---------------------------------------------------------------------- |
| `lore/4-notes/samples/soroban-events/signatures-stats.tsv`  | Full per-signature count + contract/tx fan-out for the backfill window |
| `lore/4-notes/samples/soroban-events/swap.jsonl`            | 50 swap rows (covers all three shapes)                                 |
| `lore/4-notes/samples/soroban-events/trade.jsonl`           | 50 Phoenix-style trades                                                |
| `lore/4-notes/samples/soroban-events/update_reserves.jsonl` | 50 reserve snapshots                                                   |
| `lore/4-notes/samples/soroban-events/transfer.jsonl`        | 50 SAC + custom-token transfers                                        |
| `lore/4-notes/samples/soroban-events/mint.jsonl`            | 50 mint events                                                         |
| `lore/4-notes/samples/soroban-events/burn.jsonl`            | 50 burn events                                                         |
| `lore/4-notes/samples/soroban-events/fee.jsonl`             | 50 XLM SAC fee events                                                  |
| `lore/4-notes/samples/soroban-events/REDSTONE.jsonl`        | 50 RedStone oracle updates (bytes-encoded SCVal)                       |
| `lore/4-notes/samples/soroban-events/REFLECTOR.jsonl`       | 50 Reflector oracle updates (rich SCVal map)                           |
| `lore/4-notes/samples/soroban-events/null-signature.jsonl`  | 50 events where `signature IS NULL`                                    |
