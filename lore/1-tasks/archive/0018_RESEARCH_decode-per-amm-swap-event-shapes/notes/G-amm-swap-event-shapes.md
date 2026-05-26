---
title: 'Per-AMM Soroban swap event shapes — consumer spec for prices-api Tranche 1'
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, soroswap, aquarius, phoenix, consumer-spec, stream-1]
links:
  - 'evidence/soroswap_pair_swap_decode.json'
  - 'R-be-storage-format.md'
  - '../../archive/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md'
  - '../../archive/0002_RESEARCH_amm-venue-attribution/notes/R-soroswap-registry.md'
  - 'https://github.com/soroswap/core'
  - '../../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md'
history:
  - date: 2026-05-15
    status: developing
    who: claude
    note: >
      Draft started with the Soroswap section, decoded fresh from a real
      mainnet swap (tx 21bb150d… ledger 62460506) via the stellar-xdr
      parser crate. Aquarius and Phoenix sections to follow. The
      authoritative storage shape lives in companion note
      R-be-storage-format.md (BE's tagged-JSON encoding, not raw XDR).
---

# Per-AMM Soroban swap event shapes — consumer spec

## Purpose

Pin the precise on-chain event shape for each Soroban AMM the
prices-api Tranche 1 consumer targets, so the swap-extraction logic
can be coded against a real spec instead of a guess. The spec is at
two levels:

1. **ScVal level** — the decoded event as `(topics: Vec<ScVal>, data:
ScVal)`. This is the _protocol_ shape the AMM contract emits.
2. **CH storage level** — how BE persists the event into
   `soroban_events.topics_xdr` / `.data_xdr`. This is the _consumer
   read_ shape, which is BE's custom tagged-JSON encoding, not raw
   XDR (see `R-be-storage-format.md`).

Each section below covers both levels.

---

## 1. Soroswap

Soroswap is a Uniswap-V2-style constant-product AMM on Soroban.
Canonical mainnet contract registry: archive task 0002
`R-soroswap-registry.md` (sources: `soroswap/core` and
`soroswap/aggregator` `public/mainnet.contracts.json`, retrieved
2026-05-08).

Three Soroswap event "roles" exist, with distinct `topic[0]` strings:

| Role                 | `topic[0]` value               | Authority for trade extraction?                                                                                        |
| -------------------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| `SoroswapPair`       | pool-level swap                | **YES** — emitted by the pool contract per hop, has all the on-chain amounts                                           |
| `SoroswapRouter`     | user-facing routed swap        | No (convenience event; the pool event is the source of truth and a single user-facing swap = N pair events for N hops) |
| `SoroswapAggregator` | cross-protocol aggregated swap | No (same reasoning; pool event(s) are the truth)                                                                       |

The Tranche 1 consumer **must extract from `SoroswapPair` swap events
only** to avoid double-counting multi-hop routes.

### 1.1 ScVal-level shape (the canonical Soroswap pool swap event)

Fresh decoded sample: `evidence/soroswap_pair_swap_decode.json`
(event_index 5).

- **tx**: `21bb150d1274aff1e233c76aef36ba052eefbc7e9b41c5330f5cdc213e2ff350`
- **ledger**: `62460506`
- **emitter**: `CAM7DY53G63XA4AJRS24Z6VFYAFSSF76C3RZ45BE5YU3FQS5255OOABP`
  (canonical Soroswap pair, WASM hash matches `soroswap/core`
  `hashes.pair = 18051456…f73e`)
- **pair tokens** (inferred from the SAC `transfer` events in the same
  tx — event_index 2/3 of the evidence file):
  - `token_0 = CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` (native XLM SAC)
  - `token_1 = CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75` (USDC SAC)

**Topics** (`Vec<ScVal>`, length 2):

| Position    | Type            | Value            | XDR base64 (the full Vec)                          |
| ----------- | --------------- | ---------------- | -------------------------------------------------- |
| `topics[0]` | `ScVal::String` | `"SoroswapPair"` | `AAAAAgAAAA4AAAAMU29yb3N3YXBQYWlyAAAADwAAAARzd2Fw` |
| `topics[1]` | `ScVal::Symbol` | `"swap"`         |                                                    |

**Data** (`ScVal::Map`, 5 entries):

```
ScVal::Map(Some([
    Entry { key: ScVal::Symbol("amount_0_in"),  val: ScVal::I128(<u64 hi/lo parts>) },
    Entry { key: ScVal::Symbol("amount_0_out"), val: ScVal::I128(...) },
    Entry { key: ScVal::Symbol("amount_1_in"),  val: ScVal::I128(...) },
    Entry { key: ScVal::Symbol("amount_1_out"), val: ScVal::I128(...) },
    Entry { key: ScVal::Symbol("to"),           val: ScVal::Address(ScAddress::Account(...)) },
]))
```

Sample values from the decoded evidence:

| Key            | Type      | Value                                                                                          |
| -------------- | --------- | ---------------------------------------------------------------------------------------------- |
| `amount_0_in`  | `i128`    | `6289308176`                                                                                   |
| `amount_0_out` | `i128`    | `0`                                                                                            |
| `amount_1_in`  | `i128`    | `0`                                                                                            |
| `amount_1_out` | `i128`    | `1001363207`                                                                                   |
| `to`           | `Address` | `GDKBWJP7DC2WWSNAVYMF2VJQGFOJMRX6PEHB3L33APCHOIYXBCFRNSQV` (G-account, the trader / recipient) |

**Data XDR base64**:
`AAAAEQAAAAEAAAAFAAAADwAAAAthbW91bnRfMF9pbgAAAAAKAAAAAAAAAAAAAAABdt86EAAAAA8AAAAMYW1vdW50XzBfb3V0AAAACgAAAAAAAAAAAAAAAAAAAAAAAAAPAAAAC2Ftb3VudF8xX2luAAAAAAoAAAAAAAAAAAAAAAAAAAAAAAAADwAAAAxhbW91bnRfMV9vdXQAAAAKAAAAAAAAAAAAAAAAO6+XBwAAAA8AAAACdG8AAAAAABIAAAAAAAAAANQbJf8YtWtJoK4YXVUwMVyWRv55Dh2vewPEdyMXCIsW`

### 1.2 CH storage-level shape (what the consumer actually reads)

Per `R-be-storage-format.md`, BE writes a **custom tagged JSON**, not
raw XDR. For this event:

`soroban_events.topics_xdr` (one ZSTD-coded JSON string in the cell):

```json
[
  { "type": "string", "value": "SoroswapPair" },
  { "type": "sym", "value": "swap" }
]
```

`soroban_events.data_xdr`:

```json
{
  "type": "map",
  "value": [
    {
      "key": { "type": "sym", "value": "amount_0_in" },
      "value": { "type": "i128", "value": "6289308176" }
    },
    {
      "key": { "type": "sym", "value": "amount_0_out" },
      "value": { "type": "i128", "value": "0" }
    },
    {
      "key": { "type": "sym", "value": "amount_1_in" },
      "value": { "type": "i128", "value": "0" }
    },
    {
      "key": { "type": "sym", "value": "amount_1_out" },
      "value": { "type": "i128", "value": "1001363207" }
    },
    {
      "key": { "type": "sym", "value": "to" },
      "value": {
        "type": "address",
        "value": "GDKBWJP7DC2WWSNAVYMF2VJQGFOJMRX6PEHB3L33APCHOIYXBCFRNSQV"
      }
    }
  ]
}
```

`soroban_events.signature` = **NULL** for this event (topic[0] is
`type=string`, not `type=sym` — see `R-be-storage-format.md`
Consequence 1).

### 1.3 Token-in / token-out direction convention

Token addresses are **not in the event payload** (pair-level event,
not router-level). The consumer must look up `token_0` / `token_1`
from the pair contract once and cache (factory `new_pair` events
provide both, see archive task 0002 `R-soroswap-registry.md` §1.2).

Direction is inferred from which `amount_*_in` is non-zero:

```python
if amount_0_in > 0 and amount_1_in == 0:
    # user is selling token_0 for token_1
    (token_in, amount_in)   = (token_0, amount_0_in)
    (token_out, amount_out) = (token_1, amount_1_out)
elif amount_1_in > 0 and amount_0_in == 0:
    # user is selling token_1 for token_0
    (token_in, amount_in)   = (token_1, amount_1_in)
    (token_out, amount_out) = (token_0, amount_0_out)
else:
    # both zero or both non-zero → malformed (should not occur)
```

The matching `_out` field is always non-zero on the opposite side.
The non-trading-side `_in` and `_out` are both zero.

Sample-validated: `amount_0_in = 6289308176` (>0), `amount_0_out = 0`,
`amount_1_in = 0`, `amount_1_out = 1001363207` (>0). User sold
token_0 (XLM); received token_1 (USDC). The SAC `transfer` events
in the same tx confirm the direction (event_index 2 = XLM
G→pair, event_index 3 = USDC pair→G).

### 1.4 Amount denomination

`i128` raw contract units, **not** stroops universally. The unit per
token is the contract's published `decimals()`:

| Token in this sample           | SAC contract   | Decimals | Sample raw → human              |
| ------------------------------ | -------------- | -------- | ------------------------------- |
| XLM (native SAC)               | `CAS3J7GYLGX…` | 7        | `6289308176` → 628.9308176 XLM  |
| USDC (SAC of `USDC:GA5ZSEJY…`) | `CCW67TSZV3S…` | 7        | `1001363207` → 100.1363207 USDC |

USDC at 7 decimals is the Stellar Classic convention (SAC inherits the
classic asset's precision; USDC issued by `GA5ZSEJY…` uses 7
decimals at the Classic level). Tokens with non-Stellar-Classic
origin may use different decimals — the consumer **must** read the
contract's `decimals()` once per token and cache, **not** assume 7.

### 1.5 Cross-reference against `soroswap/core` source

Source on GitHub: `soroswap/core` repo, branch `main`. Pair contract
swap event emit site to be confirmed (next-turn task) — pending
fetch of `contracts/pair/src/event.rs` or equivalent. The four-amount
"Uniswap V2 Pair.sol Swap event" layout matches the reference
implementation pattern.

> **TODO** (this task, next iteration): fetch
> `contracts/pair/src/event.rs` (or the actual filename in
> `soroswap/core`) and quote the `publish(("SoroswapPair", swap),
SwapEvent { ... })` call site verbatim to lock the field names and
> their order in the contract's emit code.

### 1.6 Filter recipe for the Tranche 1 consumer

Until `signature` can be relied on (see §1.2 above), the per-AMM
filter against the BE CH `soroban_events` table is:

```sql
SELECT contract_id, transaction_id, ledger_sequence, event_index,
       topics_xdr, data_xdr
FROM   soroban_events
WHERE  ledger_sequence BETWEEN <range_start> AND <range_end>
  AND  JSONExtractString(topics_xdr, '$[0].value') = 'SoroswapPair'
  AND  JSONExtractString(topics_xdr, '$[1].value') = 'swap'
```

(Note: CH `JSONExtractString` syntax to be validated at impl time —
ZSTD-coded `String` columns are still readable as JSON via the
JSON-family functions, but the exact path syntax `$[0].value`
vs alternative forms should be confirmed against CH version on the
local pilot.)

A second-pass filter once `signature` semantics are revisited
upstream (e.g. BE extending the hoisted-signature logic to also
hoist `String` topics, or a synthetic `protocol` column added) may
simplify to a single column predicate.

---

## 2. Aquarius

Aquarius is a multi-pool-type AMM (constant-product, stableswap,
concentrated) on Soroban. Canonical mainnet contract registry:
archive task 0002 `R-aquarius-registry.md` (sources: `AquaToken/soroban-amm`
GitHub master, stellar.expert verified-contract metadata for the
router `CBQDHNBFBZYE…`).

Aquarius emits two distinct swap-shaped events:

| Role                    | `topic[0]` value        | Authority for trade extraction?                                                                                                                        |
| ----------------------- | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Pool `Symbol("trade")`  | per-hop pool swap       | **YES** — emitted by every Aquarius pool (constant-product, stable, concentrated share this shape) with inline token_in / token_out / trader addresses |
| Router `Symbol("swap")` | user-facing routed swap | No (duplicates the pool event; use only for user-intent reconstruction or for non-pool-attributed flows)                                               |

Reasoning is the same as Soroswap: a multi-hop user swap fires N pool
`trade` events plus 1 router `swap` event, and double-counting must be
avoided. The pool event is also where the **fee** is reported, which
is needed for the OHLCV downstream.

### 2.1 ScVal-level shape (the canonical Aquarius pool `trade` event)

Fresh decoded sample: `evidence/aquarius_pool_trade_decode.json`
(event_index 4).

- **tx**: `7f785bf7d275dba8827517a1a04b4e1e65c62bd82660982ca92602e636902e53`
- **ledger**: `62079996`
- **emitter**: `CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE`
  (canonical Aquarius constant-product pool — WASM hash
  `ae0da5a84b15805c5c7931ac567a8d1b34be3f26b483993d9ff80cb2c3de9852`,
  per archive task 0002 `R-aquarius-registry.md`)

**Topics** (`Vec<ScVal>`, length 4 — inline tokens + trader):

| Position    | Type             | Value (sample)                                                                    |
| ----------- | ---------------- | --------------------------------------------------------------------------------- |
| `topics[0]` | `ScVal::Symbol`  | `"trade"`                                                                         |
| `topics[1]` | `ScVal::Address` | `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` (token_in — XLM SAC)   |
| `topics[2]` | `ScVal::Address` | `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75` (token_out — USDC SAC) |
| `topics[3]` | `ScVal::Address` | `GDCRZPZYBZ24RHRO3WBPJGFDL7NDFKUQBS3ZDB6YGBJB3TGKMFYBQ3LD` (trader — G-account)   |

**Topics XDR base64**:
`AAAABAAAAA8AAAAFdHJhZGUAAAAAAAASAAAAASW0/NhZrsL6Y0hDjEibPDwQyYttIb5P08swy2iVPvl3AAAAEgAAAAGt785ZruUpaPdgYdSUwlJbdWWfpClqZfSZ7ynlZHfklgAAABIAAAAAAAAAAMUcvzgOdcieLt2C9JijX9oyqpAMt5GH2DBSHczKYXAY`

**Data** (`ScVal::Vec`, 3 entries — positional, not keyed):

```
ScVal::Vec(Some([
    ScVal::I128(amount_in),
    ScVal::I128(amount_out),
    ScVal::I128(fee),     // denominated in token_in
]))
```

Sample values:

| Index     | Type   | Value (raw)   | Meaning                                                                                                         |
| --------- | ------ | ------------- | --------------------------------------------------------------------------------------------------------------- |
| `data[0]` | `i128` | `25761941491` | `in_amount` (= ~2576.19 XLM at 7 decimals)                                                                      |
| `data[1]` | `i128` | `3901204480`  | `out_amount` (= ~390.12 USDC at 7 decimals)                                                                     |
| `data[2]` | `i128` | `12880971`    | `fee_amount` (= ~1.29 XLM ≈ 0.05% of in_amount, consistent with the Aquarius constant-product 5-bps fee config) |

**Data XDR base64**: `AAAAEAAAAAEAAAADAAAACgAAAAAAAAAAAAAABf+IB/MAAAAKAAAAAAAAAAAAAAAA6IeoAAAAAAoAAAAAAAAAAAAAAAAAxIxL`

### 2.2 CH storage-level shape

Per `R-be-storage-format.md`, BE writes the custom tagged JSON.
For this event:

`soroban_events.topics_xdr`:

```json
[
  { "type": "sym", "value": "trade" },
  {
    "type": "address",
    "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"
  },
  {
    "type": "address",
    "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"
  },
  {
    "type": "address",
    "value": "GDCRZPZYBZ24RHRO3WBPJGFDL7NDFKUQBS3ZDB6YGBJB3TGKMFYBQ3LD"
  }
]
```

`soroban_events.data_xdr`:

```json
{
  "type": "vec",
  "value": [
    { "type": "i128", "value": "25761941491" },
    { "type": "i128", "value": "3901204480" },
    { "type": "i128", "value": "12880971" }
  ]
}
```

`soroban_events.signature` = **`"trade"`** for this event
(topic[0] is `type == "sym"`, so BE's `extract_event_signature`
populates it normally — unlike the Soroswap §1.2 case).

### 2.3 Token-in / token-out direction convention

Trivial — token addresses are **inline in the topic vector**:

- `token_in` = `topics[1]`
- `token_out` = `topics[2]`
- `trader` = `topics[3]`
- `amount_in` = `data[0]` (in `token_in` raw units)
- `amount_out` = `data[1]` (in `token_out` raw units)
- `fee` = `data[2]` (in `token_in` raw units)

No per-pool `token_0` / `token_1` lookup required (the meaningful
contrast with Soroswap §1.3). No zero-vs-non-zero inference logic.
This is materially simpler to extract.

### 2.4 Amount denomination

Same convention as Soroswap §1.4: `i128` raw contract units, with
per-token decimals read from the token contract's `decimals()` —
**do not assume 7**. In this sample both tokens are SACs of
7-decimal Stellar Classic assets (XLM native + USDC issued by
`GA5ZSEJY…`), so the raw values divided by `10^7` give the
human-readable amounts cited above. A pool whose tokens include a
contract-issued (non-Classic) token with `decimals() = 18` would
not.

### 2.5 Cross-reference against `AquaToken/soroban-amm` source

Source on GitHub: `AquaToken/soroban-amm`, branch `master`,
`liquidity_pool_events/src/lib.rs` (shared by `liquidity_pool`
constant-product, `liquidity_pool_stableswap`, and
`liquidity_pool_concentrated` pool modules). The canonical emit
signature, transcribed in archive task 0002 `R-aquarius-registry.md`
§"Event formats":

```rust
fn trade(user, token_in, token_out, in_amount, out_amount, fee_amount) {
    // topics: ("trade", token_in: Address, token_out: Address, user: Address)
    // body:   (in_amount as i128, out_amount as i128, fee_amount as i128)
}
```

Decoded sample matches exactly: `Symbol("trade")` + three `Address`
topics in the documented order, three `I128` in the body in the
documented order. The router-side `swap` emit signature is
documented in the same registry note (`liquidity_pool_router/src/events.rs`)
and matches the wider-sample observations in archive task 0001
`R-swap-topic-shapes.md`. No source-vs-empirical drift on either
side.

### 2.6 Filter recipe for the Tranche 1 consumer

Aquarius is the simple case — the hoisted `signature` column works:

```sql
SELECT contract_id, transaction_id, ledger_sequence, event_index,
       topics_xdr, data_xdr
FROM   soroban_events
WHERE  ledger_sequence BETWEEN <range_start> AND <range_end>
  AND  signature = 'trade'
```

To restrict to **Aquarius** pools specifically (vs. any other
protocol that happens to emit `Symbol("trade")` — currently none
observed at scale, but possible in the future), join against the
authoritative pool registry built from the Aquarius router
(`CBQDHNBFBZYE…`) `Symbol("add_pool")` events:

```sql
SELECT t.*
FROM   soroban_events AS t
WHERE  t.signature = 'trade'
  AND  t.contract_id IN (
         SELECT JSONExtractString(data_xdr, '$.value[0].value')   -- body[0] = pool address
         FROM   soroban_events
         WHERE  contract_id = '<aquarius_router_strkey>'
           AND  signature = 'add_pool'
       )
```

(The second predicate's CH JSON-extract syntax is again to be
validated against the local pilot CH version once 0017 lands.)

---

## 3. Phoenix

Phoenix DeFi Hub is a multi-pool-type AMM (XYK + stable) on Soroban.
Canonical mainnet contract registry: archive task 0002
`R-phoenix-registry.md` (sources: `Phoenix-Protocol-Group/phoenix-contracts`
`scripts/upgrade_mainnet.sh`, stellar.expert directory entries
labelled "Phoenix Pool" / `phoenix-hub.io`). Eleven XYK pools are
confirmed by exact-WASM membership in the upgrade script's
`pools=()` array.

Phoenix's swap event shape is the **structural outlier** of the
three AMMs: a single swap fires **eight separate events** from the
pool contract (six for the stable-pool variant), each carrying one
named field of the swap as `topic[1]` and the corresponding value
as `data`. The decoder must group `(tx_hash, contract_id)` and fold
the eight events into one logical trade row.

### 3.1 ScVal-level shape (XYK pool — 8-event grouping)

Fresh decoded sample: `evidence/phoenix_pool_swap_decode.json`
(event_index 5 through 12 of the same tx).

- **tx**: `559498bdf567340c0780b80f2bfa07bcc58713fc328e659ef72461849a326aa8`
- **ledger**: `62460522`
- **emitter**: `CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX`
  (canonical Phoenix XYK pool, pair XLM/USDC — stellar.expert
  directory label "Phoenix Pool", `phoenix-hub.io`, per archive
  task 0002 `R-phoenix-registry.md`)

Each of the eight events has the same two-topic shape:

```
topics = [ScVal::String("swap"), ScVal::String("<field>")]
data   = <field-typed ScVal>
```

The **eight fields**, in their emission order (matching
`contracts/pool/src/contract.rs:1172-1185`):

| event_index (relative) | `topic[1]`                           | data type | sample value                                               | Meaning                                                                                                                                                                                      |
| ---------------------- | ------------------------------------ | --------- | ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| +0 (= 5)               | `String("sender")`                   | `Address` | `GDCRZPZYBZ24RHRO3WBPJGFDL7NDFKUQBS3ZDB6YGBJB3TGKMFYBQ3LD` | trader (G-account)                                                                                                                                                                           |
| +1 (= 6)               | `String("sell_token")`               | `Address` | `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` | token_in (XLM SAC)                                                                                                                                                                           |
| +2 (= 7)               | `String("offer_amount")`             | `i128`    | `11659417676` (≈ 1165.94 XLM)                              | amount_in (raw, token_in units)                                                                                                                                                              |
| +3 (= 8)               | `String("actual received amount")` ⚠ | `i128`    | `11659417676`                                              | actual `token_in` received by the pool — diverges from `offer_amount` only for fee-on-transfer tokens; equals `offer_amount` for vanilla SAC tokens. **Field name contains literal spaces.** |
| +4 (= 9)               | `String("buy_token")`                | `Address` | `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75` | token_out (USDC SAC)                                                                                                                                                                         |
| +5 (= 10)              | `String("return_amount")`            | `i128`    | `1857322909` (≈ 185.73 USDC)                               | amount_out (raw, token_out units)                                                                                                                                                            |
| +6 (= 11)              | `String("spread_amount")`            | `i128`    | `503808`                                                   | slippage (not the protocol fee — see §3.4)                                                                                                                                                   |
| +7 (= 12)              | `String("referral_fee_amount")`      | `i128`    | `0`                                                        | referral fee — currently disabled in the contract (`FIXM:` flag in source), always 0                                                                                                         |

> ⚠ The `actual received amount` field name contains literal
> spaces. Preserve verbatim when matching — do **not** normalise
> to snake_case.

Topics XDR base64 (example, for the `sender` event): `AAAAAgAAAA4AAAAEc3dhcAAAAA4AAAAGc2VuZGVyAAA=`

Stable-pool variant emits **six events**
(`contracts/pool_stable/src/contract.rs:1182-1189`): drops
`actual received amount` and `referral_fee_amount`. Same topic
shape, same `String` typing. No mainnet stable-pool address is
currently known per `R-phoenix-registry.md` — the decoder should
still handle the 6-event case for future-proofing.

### 3.2 CH storage-level shape

Each of the 8 events is a separate `soroban_events` row. For the
first event (`sender`):

`soroban_events.topics_xdr`:

```json
[
  { "type": "string", "value": "swap" },
  { "type": "string", "value": "sender" }
]
```

`soroban_events.data_xdr`:

```json
{
  "type": "address",
  "value": "GDCRZPZYBZ24RHRO3WBPJGFDL7NDFKUQBS3ZDB6YGBJB3TGKMFYBQ3LD"
}
```

`soroban_events.signature` = **NULL** for every Phoenix swap event
(`topic[0]` is `type == "string"`, not `"sym"` — same root cause
as Soroswap §1.2). Per `R-be-storage-format.md`, the consumer
filter on `signature` does not work for Phoenix any more than for
Soroswap.

### 3.3 Token-in / token-out direction convention

Token addresses come from the dedicated `sell_token` and `buy_token`
events:

- `token_in` = data of the event with `topic[1] = String("sell_token")`
- `token_out` = data of the event with `topic[1] = String("buy_token")`
- `trader` = data of the event with `topic[1] = String("sender")`
- `amount_in` = data of the event with `topic[1] = String("offer_amount")` (raw units of `token_in`)
- `amount_out` = data of the event with `topic[1] = String("return_amount")` (raw units of `token_out`)

The eight events for a single swap **all carry the same
`contract_id` and the same `tx_hash`**, and are guaranteed
contiguous within the tx in emission order (sender → sell_token →
offer_amount → actual_received → buy_token → return_amount →
spread → referral_fee). The decoder can rely on either:

- **By tx_hash + contract_id**: group all rows where
  `(transaction_id, contract_id)` match and `topic[0] = String("swap")`;
  pick out each field by `topic[1]`. Most robust if event ordering
  ever changes.
- **By contiguous event_index window**: take any
  `String("swap")` row at offset _k_ and read offsets _k..k+7_ in
  order. Faster if the emission order is stable (it has been
  since launch per the source, but is in principle a contract
  upgrade away from changing).

### 3.4 Amount denomination — and the fee gap

Amounts are `i128` raw contract units, decimals per token from the
token contract's `decimals()` — same convention as Soroswap §1.4
and Aquarius §2.4. Both tokens in this sample are 7-decimal SACs.

**Fee is NOT emitted as an event.** Phoenix's source (per
`R-phoenix-registry.md`) computes the protocol-fee debit inside
`do_swap` and pays it directly to `fee_recipient` without publishing
it. `spread_amount` is slippage (the pricing error vs. the constant-
product curve), not the fee. Consumer options for the trade-row's
`fee` column:

1. **Leave NULL.** Lossy but simplest — recommended for MVP.
2. **Reconstruct from pool config.** Call the Phoenix pool's
   `query_config()` once per pool, cache the `commission_bps`,
   compute `fee = offer_amount * commission_bps / 10_000` per
   trade. Adds a per-pool one-shot lookup; requires Soroban RPC
   access from the consumer side.

This is a meaningful schema asymmetry vs. Aquarius (where the fee
is `data[2]` of the trade event). The Tranche 1 consumer should
adopt option 1 unless the downstream OHLCV requires per-trade fee
revenue tracking.

### 3.5 Cross-reference against `Phoenix-Protocol-Group/phoenix-contracts` source

Source already transcribed and confirmed in archive task 0002
`R-phoenix-registry.md` §"Pool swap event format":

```rust
// contracts/pool/src/contract.rs:1172-1185
env.events().publish(("swap", "sender"), sender);
env.events().publish(("swap", "sell_token"), sell_token);
env.events().publish(("swap", "offer_amount"), offer_amount);
env.events().publish(("swap", "actual received amount"), actual_received_amount);
env.events().publish(("swap", "buy_token"), buy_token);
env.events().publish(("swap", "return_amount"), compute_swap.return_amount);
env.events().publish(("swap", "spread_amount"), compute_swap.spread_amount);
env.events().publish(("swap", "referral_fee_amount"), compute_swap.referral_fee_amount);
```

Decoded sample matches the source signature bit-for-bit, including
the unusual `actual received amount` literal. The `(swap, <field>)`
tuple in Rust source uses `&str` literals which `IntoVal` compiles
to `ScVal::String` (a `ScVal::Symbol` would require
`symbol_short!()` / `Symbol::new()`) — explains the otherwise-
surprising `String` topic kind.

### 3.6 Filter recipe for the Tranche 1 consumer

Phoenix is the harder case — `signature` is NULL and the 8-event
fan-out means a naive `WHERE topic[0]='swap'` query returns 8 rows
per logical trade.

**Recommended approach: contract_id whitelist + GROUP BY**.

```sql
WITH phoenix_swap_events AS (
  SELECT transaction_id, contract_id, event_index, ledger_sequence,
         JSONExtractString(topics_xdr, '$[1].value') AS field,
         data_xdr
  FROM   soroban_events
  WHERE  ledger_sequence BETWEEN <range_start> AND <range_end>
    AND  contract_id IN (<11 known phoenix pool ids>)
    AND  JSONExtractString(topics_xdr, '$[0].value') = 'swap'
)
SELECT transaction_id, contract_id, ledger_sequence,
       MIN(event_index)                                                     AS first_event_index,
       maxIf(JSONExtractString(data_xdr, '$.value'), field = 'sender')      AS trader,
       maxIf(JSONExtractString(data_xdr, '$.value'), field = 'sell_token')  AS token_in,
       maxIf(JSONExtractString(data_xdr, '$.value'), field = 'offer_amount') AS amount_in,
       maxIf(JSONExtractString(data_xdr, '$.value'), field = 'buy_token')   AS token_out,
       maxIf(JSONExtractString(data_xdr, '$.value'), field = 'return_amount') AS amount_out,
       maxIf(JSONExtractString(data_xdr, '$.value'), field = 'spread_amount') AS spread
FROM   phoenix_swap_events
GROUP BY transaction_id, contract_id, ledger_sequence
```

(CH `JSONExtractString` + `maxIf` syntax to be validated against
the local pilot CH version once task 0017 lands. The
`maxIf(..., field = '<name>')` pattern works on string values too
because at most one event per group carries each field.)

The 11-pool whitelist comes from
`R-phoenix-registry.md` §"Pool contracts". For new pools, the
Phoenix factory `CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI`
emits `(Symbol("create"), Symbol("liquidity_pool"))` with
`data = Address(<new pool>)` — the consumer should periodically
re-scan the factory to keep the whitelist current.

If avoiding a 50%-overhead `contract_id` lookup is desired, an
alternative filter is `JSONExtractString(topics_xdr, '$[0].value') =
'swap' AND JSONExtractString(topics_xdr, '$[1].type') = 'string'`
— but that doesn't distinguish Phoenix from any other contract
that happens to use `String("swap")` topic, so the whitelist is
the safer path.

---

## Appendix A — Recommendation: per-AMM extractor with shared dispatch

**Verdict: three independent decoders are required.** The pool-level
swap event shapes have no overlap usable for a single extractor:

| Venue              | Topic[0] kind            | Topic[1..]                                  | Data shape                                                                    | Token inline?                                  | Fee in event?                    | Event count per swap      |
| ------------------ | ------------------------ | ------------------------------------------- | ----------------------------------------------------------------------------- | ---------------------------------------------- | -------------------------------- | ------------------------- |
| Soroswap (Pair)    | `String("SoroswapPair")` | `Symbol("swap")`                            | `Map<5>` (`amount_0_in`, `amount_0_out`, `amount_1_in`, `amount_1_out`, `to`) | **No** — `token_0`/`token_1` lookup needed     | No (computable from pool config) | 1                         |
| Aquarius (pool)    | `Symbol("trade")`        | `Address × 3` (token_in, token_out, trader) | `Vec<3, i128>` (`amount_in`, `amount_out`, `fee`)                             | **Yes**                                        | **Yes** (`data[2]`)              | 1                         |
| Phoenix (XYK pool) | `String("swap")`         | `String("<field>")` × 8                     | scalar per event                                                              | Yes (across `sell_token` / `buy_token` events) | No (computable from pool config) | **8** (6 for stable pool) |

The differences are structural, not cosmetic — Soroswap requires
direction inference and token-side lookup; Aquarius is fully
inline; Phoenix requires per-tx event grouping.

**Recommended consumer shape**: a per-venue extractor trait keyed
by `(contract_id) → venue`, with a per-venue input-grouping policy:

```rust
trait SwapExtractor {
    /// Read a contiguous run of CH rows for one tx and produce zero
    /// or more trade rows. Returns the number of CH rows consumed.
    fn extract(rows: &[SorobanEventRow]) -> ExtractResult;
}

// Concrete impls:
struct SoroswapPairExtractor;   // consumes 1 row per swap
struct AquariusPoolExtractor;   // consumes 1 row per swap
struct PhoenixXykPoolExtractor; // consumes 8 contiguous rows per swap
struct PhoenixStablePoolExtractor; // consumes 6 contiguous rows per swap
```

Venue identification at row-level: build a `HashMap<C-strkey, Venue>`
once from the three registries (Soroswap factory `new_pair` events,
Aquarius router `add_pool` events, Phoenix factory
`("create", "liquidity_pool")` events) and look up per event's
`contract_id`. This is more robust than relying on topic shape
alone, which can collide across protocols (e.g. nothing prevents a
new protocol from also using `Symbol("trade")`).

Filter complexity ranking, from simplest to hardest:

1. **Aquarius** — `WHERE signature = 'trade' AND contract_id IN (aquarius_pools)`
2. **Soroswap pair** — `WHERE JSONExtractString(topics_xdr, '$[0].value') = 'SoroswapPair' AND JSONExtractString(topics_xdr, '$[1].value') = 'swap'` (whitelist optional but recommended)
3. **Phoenix** — full SQL group-by as in §3.6 above

The consumer's pipeline executes the three filters concurrently
and feeds the matching row sets into the corresponding extractor.

---

## Appendix B — Open follow-ups (out of scope for this task)

Findings during this decode that should spawn separate backlog
tasks rather than expanding scope here:

1. **BE column naming.** `soroban_events.topics_xdr` /
   `.data_xdr` contain JSON, not XDR bytes. Surface a rename
   request (or a `COMMENT ON COLUMN`) upstream to BE. See
   `R-be-storage-format.md` "Open follow-ups".
2. **`signature` column undercounts non-Symbol-first-topic
   events.** Soroswap and Phoenix both fall in this category.
   Either (a) extend BE's `extract_event_signature` to also hoist
   `String`-typed topic[0]s, or (b) add a parallel
   `LowCardinality(Nullable(String)) protocol` column derived
   from the topic[0]+topic[1] pair. Both options keep the existing
   Aquarius behaviour intact. Task 0017's smoke-query design
   should account for the current behaviour.
3. **Phoenix stable-pool first observation.** When the first
   stable-pool address appears in the wild, capture its WASM hash
   and confirm the 6-event-grouping decoder works against a real
   sample. Currently the registry only carries XYK pool addresses.
4. **Soroswap source-emit-site verbatim quote.** §1.5 still has a
   TODO to fetch `soroswap/core` Pair contract's `event.rs` (or
   equivalent) and lock the field names from the emit call. The
   decoded sample confirms the shape; a verbatim source quote
   closes the loop the way `R-phoenix-registry.md` and
   `R-aquarius-registry.md` did for those two.
