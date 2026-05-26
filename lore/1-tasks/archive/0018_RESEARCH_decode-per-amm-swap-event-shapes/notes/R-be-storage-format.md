---
title: "BE soroban_events JSON storage format — tagged ScVal encoding, not raw XDR"
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [clickhouse, scval, json-encoding, signature-column, stream-1-consumer]
links:
  - "../../../../soroban-block-explorer/crates/xdr-parser/src/scval.rs"
  - "../../../../soroban-block-explorer/crates/xdr-parser/src/event.rs"
  - "../../../../soroban-block-explorer/crates/db-clickhouse/src/persist/stage.rs"
  - "../../../../soroban-block-explorer/lore/2-adrs/0044_clickhouse-pilot-parallel-store.md"
  - "evidence/soroswap_pair_swap_decode.json"
history:
  - date: 2026-05-15
    status: developing
    who: claude
    note: >
      Recorded during the Soroswap sample decode (task 0018 first AMM).
      The column-name `topics_xdr` / `data_xdr` and ADR 0044's narrative
      both pointed the consumer toward `stellar-xdr` deserialization;
      reading the BE source revealed a different reality.
---

# BE soroban_events JSON storage format

## Headline

The CH `soroban_events.topics_xdr` and `.data_xdr` columns despite their
names **do not** contain XDR bytes (base64 or otherwise). They contain
**BE's custom tagged-JSON encoding** of the decoded `ScVal` tree,
produced by `scval_to_typed_json` in
`crates/xdr-parser/src/scval.rs`.

Implication for the prices-api Tranche 1 consumer: the planned
"decode via the `stellar-xdr` parser crate" decode path **does not
apply directly**. Two specific consequences are spelled out below.

## The actual encoding

`scval_to_typed_json` emits every `ScVal` as
`{ "type": "<tag>", "value": <val> }`. The tag set is fixed by the
function, not by `stellar-xdr`:

| ScVal variant | tag | value shape |
|---|---|---|
| `Bool(b)` | `"bool"` | JSON bool |
| `Void` | `"void"` | JSON null |
| `Error(e)` | `"error"` | `e.name()` (string) |
| `U32(x)` / `I32(x)` | `"u32"` / `"i32"` | JSON number |
| `U64(x)` / `I64(x)` | `"u64"` / `"i64"` | JSON number |
| `Timepoint(t)` | `"timepoint"` | u64 |
| `Duration(d)` | `"duration"` | u64 |
| `U128` / `I128` | `"u128"` / `"i128"` | **decimal string** (i128 fits a Rust string, not a JSON number) |
| `U256` / `I256` | `"u256"` / `"i256"` | 64-char hex string |
| `Bytes(b)` | `"bytes"` | base64 string |
| `String(s)` | `"string"` | raw UTF-8 |
| `Symbol(s)` | **`"sym"`** | raw UTF-8 (note: 3-char tag, not `"symbol"`) |
| `Vec(Some(v))` | `"vec"` | array of tagged values |
| `Map(Some(m))` | `"map"` | array of `{ "key": <tagged>, "value": <tagged> }` |
| `Address(a)` | `"address"` | G-address or C-address strkey |
| `ContractInstance` | `"contract_instance"` | `{ "executable": … }` |
| `LedgerKeyContractInstance` | `"ledger_key_contract_instance"` | null |
| `LedgerKeyNonce(k)` | `"ledger_key_nonce"` | i64 nonce |

`scval_to_typed_json` is **not the inverse** of `stellar-xdr`'s default
`#[derive(Serialize)]` for `ScVal` — that derive produces e.g.
`{ "symbol": "swap" }` (single key per variant, snake_case). Round-
tripping CH content through `serde_json::from_str::<ScVal>(...)` will
fail.

## Worked example — Soroswap pair swap (this task's decoded sample)

Source: `evidence/soroswap_pair_swap_decode.json` event_index 5,
contract `CAM7DY53G63XA4AJRS24Z6VFYAFSSF76C3RZ45BE5YU3FQS5255OOABP`
(WASM-verified canonical Soroswap pair, per archive task 0002
`R-soroswap-registry.md`).

That evidence file shows the `stellar-xdr` default-serde form (because
`dump-swap-events` uses it). The same event written into CH by BE
becomes:

`soroban_events.topics_xdr` (one JSON string in the cell):

```json
[
  { "type": "string", "value": "SoroswapPair" },
  { "type": "sym",    "value": "swap" }
]
```

`soroban_events.data_xdr` (one JSON string in the cell):

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

For reference, the raw XDR base64 (what `topics_xdr` *would* hold if
the column actually stored XDR) is in
`evidence/soroswap_pair_swap_decode.json` under the
`topics_xdr_b64` and `data_xdr_b64` fields, captured by
`dump-swap-events --show-xdr`.

## Consequence 1 — `signature = 'swap'` does NOT match Soroswap

`crates/db-clickhouse/src/persist/stage.rs::extract_event_signature`
(snippet):

```rust
fn extract_event_signature(topics: &Value) -> Option<String> {
    let first = topics.as_array()?.first()?.as_object()?;
    if first.get("type").and_then(Value::as_str)? != "sym" {
        return None;     // ← Soroswap's first topic is "string", returns None
    }
    first
        .get("value")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
```

The hoisted `signature` column is populated **only** when
`topics[0].type == "sym"`. Soroswap (`SoroswapPair` / `SoroswapRouter`
/ `SoroswapAggregator`) leads with `type == "string"`, so the column
is NULL for every Soroswap event in CH.

Filter implications:

| Predicate | Soroswap pair `swap` matches? |
|---|---|
| `WHERE signature = 'swap'` | **No** (column is NULL) |
| `WHERE signature IS NULL` | Yes, but also matches every non-Symbol-first-topic event ever (too broad) |
| `WHERE JSONExtractString(topics_xdr, '$[0].value') = 'SoroswapPair' AND JSONExtractString(topics_xdr, '$[1].value') = 'swap'` | Yes — exact (CH path syntax; verify against ZSTD-coded String column at consumer-impl time) |
| `WHERE contract_id IN (<pre-enumerated Soroswap pair ids>)` | Yes (requires factory-event enumeration of pair IDs) |

The smoke query in task 0017 (`SELECT count() FROM soroban_events
WHERE signature = 'swap'`) will undercount Soroswap by exactly the
sum of `SoroswapPair` / `SoroswapRouter` / `SoroswapAggregator` swap
events in the range. For Aquarius pool `trade` events the column is
populated (`'trade'`); for the Aquarius-router `Symbol("swap")`
events the column is populated (`'swap'`). Phoenix — to be confirmed,
but per `R-swap-topic-shapes.md` it likely emits `Symbol("swap")`
from its pools, so populated.

## Consequence 2 — consumer needs a reverse tagged-JSON decoder

The decode path the Tranche 1 consumer actually wants:

```
CH cell (String, ZSTD-coded JSON)
   → serde_json::Value (an array of tagged objects, or a tagged object)
   → walk the {type, value} tree to extract amount_0_in etc.
```

Either of:

1. **Direct walking**: read `topics[0].value` and `topics[1].value`
   from the array; for the data map, iterate `value` entries and
   pattern-match on `key.value` strings. Cheap, no reverse decoder.
2. **Reverse to ScVal**: implement `typed_json_to_scval` mirroring
   BE's `scval_to_typed_json` and then operate on `ScVal`. More
   structured, but adds a maintenance dependency on BE's tag set
   staying stable.

Option 1 is the simpler path for the per-AMM extractors. Option 2
becomes attractive if multiple downstream consumers want the
`ScVal` form.

## Open follow-ups

- Confirm against a live CH instance (gated on task 0017) that
  `signature` is NULL for a `SoroswapPair` event row — the source
  reading is conclusive but a smoke verification is worth the cost
  once CH is queryable.
- Surface this finding to BE: column name `*_xdr` is misleading for
  JSON content. Either renaming or a column comment in the CH
  schema would help future readers. (Spawn as a backlog task once
  task 0018 closes.)
- Consider whether prices-api would benefit from a BE-side helper
  crate exposing `typed_json_to_scval` so multiple downstream
  consumers don't each re-implement it.

## References

- BE encoder: `crates/xdr-parser/src/scval.rs::scval_to_typed_json`
- BE writer call site: `crates/db-clickhouse/src/persist/stage.rs:715-718`
- BE signature extractor: `crates/db-clickhouse/src/persist/stage.rs:1304-1314`
- ADR 0044 codec note (this task's source): the "ScVal-decoded JSON"
  phrasing in the 2026-05-12 history entry is accurate but does not
  pin the tag-set — this note pins it.
