---
title: "Per-AMM Soroban swap event shapes — consumer spec for prices-api Tranche 1"
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: [soroban, amm, soroswap, aquarius, phoenix, consumer-spec, stream-1]
links:
  - "evidence/soroswap_pair_swap_decode.json"
  - "R-be-storage-format.md"
  - "../../archive/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
  - "../../archive/0002_RESEARCH_amm-venue-attribution/notes/R-soroswap-registry.md"
  - "https://github.com/soroswap/core"
  - "../../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
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
   ScVal)`. This is the *protocol* shape the AMM contract emits.
2. **CH storage level** — how BE persists the event into
   `soroban_events.topics_xdr` / `.data_xdr`. This is the *consumer
   read* shape, which is BE's custom tagged-JSON encoding, not raw
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

| Role | `topic[0]` value | Authority for trade extraction? |
|---|---|---|
| `SoroswapPair` | pool-level swap | **YES** — emitted by the pool contract per hop, has all the on-chain amounts |
| `SoroswapRouter` | user-facing routed swap | No (convenience event; the pool event is the source of truth and a single user-facing swap = N pair events for N hops) |
| `SoroswapAggregator` | cross-protocol aggregated swap | No (same reasoning; pool event(s) are the truth) |

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

| Position | Type | Value | XDR base64 (the full Vec) |
|---|---|---|---|
| `topics[0]` | `ScVal::String` | `"SoroswapPair"` | `AAAAAgAAAA4AAAAMU29yb3N3YXBQYWlyAAAADwAAAARzd2Fw` |
| `topics[1]` | `ScVal::Symbol` | `"swap"` |  |

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

| Key | Type | Value |
|---|---|---|
| `amount_0_in` | `i128` | `6289308176` |
| `amount_0_out` | `i128` | `0` |
| `amount_1_in` | `i128` | `0` |
| `amount_1_out` | `i128` | `1001363207` |
| `to` | `Address` | `GDKBWJP7DC2WWSNAVYMF2VJQGFOJMRX6PEHB3L33APCHOIYXBCFRNSQV` (G-account, the trader / recipient) |

**Data XDR base64**:
`AAAAEQAAAAEAAAAFAAAADwAAAAthbW91bnRfMF9pbgAAAAAKAAAAAAAAAAAAAAABdt86EAAAAA8AAAAMYW1vdW50XzBfb3V0AAAACgAAAAAAAAAAAAAAAAAAAAAAAAAPAAAAC2Ftb3VudF8xX2luAAAAAAoAAAAAAAAAAAAAAAAAAAAAAAAADwAAAAxhbW91bnRfMV9vdXQAAAAKAAAAAAAAAAAAAAAAO6+XBwAAAA8AAAACdG8AAAAAABIAAAAAAAAAANQbJf8YtWtJoK4YXVUwMVyWRv55Dh2vewPEdyMXCIsW`

### 1.2 CH storage-level shape (what the consumer actually reads)

Per `R-be-storage-format.md`, BE writes a **custom tagged JSON**, not
raw XDR. For this event:

`soroban_events.topics_xdr` (one ZSTD-coded JSON string in the cell):

```json
[
  { "type": "string", "value": "SoroswapPair" },
  { "type": "sym",    "value": "swap" }
]
```

`soroban_events.data_xdr`:

```json
{
  "type": "map",
  "value": [
    { "key": { "type": "sym", "value": "amount_0_in"  }, "value": { "type": "i128", "value": "6289308176" } },
    { "key": { "type": "sym", "value": "amount_0_out" }, "value": { "type": "i128", "value": "0"          } },
    { "key": { "type": "sym", "value": "amount_1_in"  }, "value": { "type": "i128", "value": "0"          } },
    { "key": { "type": "sym", "value": "amount_1_out" }, "value": { "type": "i128", "value": "1001363207" } },
    { "key": { "type": "sym", "value": "to"           }, "value": { "type": "address", "value": "GDKBWJP7DC2WWSNAVYMF2VJQGFOJMRX6PEHB3L33APCHOIYXBCFRNSQV" } }
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

| Token in this sample | SAC contract | Decimals | Sample raw → human |
|---|---|---|---|
| XLM (native SAC) | `CAS3J7GYLGX…` | 7 | `6289308176` → 628.9308176 XLM |
| USDC (SAC of `USDC:GA5ZSEJY…`) | `CCW67TSZV3S…` | 7 | `1001363207` → 100.1363207 USDC |

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
> SwapEvent { ... })` call site verbatim to lock the field names and
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

> **TODO** — next iteration of this task.

Prior surface: archive task 0002 `R-aquarius-registry.md` and
`R-swap-topic-shapes.md` §`Symbol("trade")`. The `Symbol("trade")`
event with 3-element `Vec<i128>` data payload is the candidate
pool-level event; the `Symbol("swap")` event from
`CBQDHNBFBZYE…` (verified Aquarius
`soroban-liquidity-pool-router-contract`) is router-level.

---

## 3. Phoenix

> **TODO** — next iteration of this task.

Prior surface: archive task 0002 `R-phoenix-registry.md`. Phoenix
emits `Symbol("swap")` from its pool contracts (per public reading)
but did not appear under that name in archive task 0001's wider
sample — venue attribution will need WASM-hash matching against
Phoenix's published pool / factory binaries.

---

## Appendix A — Recommendation: shared vs per-AMM extractor

(To be finalised once all three AMM sections land. Initial reading
from prior work and Soroswap: at least three distinct decoders are
needed — Aquarius `trade` 3-element Vec, Aquarius router `swap`
5-element Vec, Soroswap `swap` 5-key Map — plus a Phoenix decoder
TBD. A trait-based dispatch keyed by `(contract_id → venue)` is the
likely shape.)
