# dump-swap-events

Investigative Rust tool for parsing zstd-compressed
`LedgerCloseMetaBatch` files (Galexie history-archive layout) and printing
Soroban contract event topics + data as JSON. Created for lore task 0001
to answer §11.1 of `docs/database-schema/amm-trades-schema.md`.

Self-contained — depends only on `stellar-xdr 26`, `stellar-strkey`,
`zstd`, and `serde_json`. No coupling to the soroban-block-explorer
workspace.

## Build

```
cargo build --release
```

First build pulls ~80 crates and compiles in 20–30 s on a warm cache.

## Run

```
./target/release/dump-swap-events --dir <PATH> [options]
```

| Flag | Default | Effect |
|---|---|---|
| `--dir <PATH>` | required | Directory of `.xdr.zst` files |
| `--symbol <SUBSTR>` | `swap` | Substring match against `topic[0]` (string form) |
| `--no-filter` | off | Disable the topic filter |
| `--histogram` | off | Suppress per-event output; print only the `topic_0` histogram. Implies `--no-filter`. |
| `--limit <N>` | unlimited | Stop after N emitted events |
| `--include-diagnostic` | off | Include events from the V3/V4 `diagnostic_events` container (otherwise dropped — they hold byte-identical mirrors of consensus events) |
| `--pretty` | off | Pretty-print JSON instead of one-line-per-event |

Output is JSONL on stdout, summary + histogram on stderr.

## Examples

Survey topic symbols across a ledger range:

```
./target/release/dump-swap-events --dir ../../.temp/FC4DB5FF--62016000-62079999 --histogram
```

Dump all `swap`-tagged events:

```
./target/release/dump-swap-events --dir ../../.temp/FC4DB5FF--62016000-62079999
```

Dump `trade`-tagged events with pretty output:

```
./target/release/dump-swap-events --dir <PATH> --symbol trade --pretty
```

## Output schema

```jsonc
{
  "ledger_seq": 62079999,
  "tx_hash": "f8b9...",          // hex, lowercase
  "event_index": 0,              // 0-based, per-tx, monotonic across sources
  "source": "TxLevel"            // | "PerOp" | "Diagnostic"
                                 // (see CAP-67 / soroban-block-explorer task 0182)
  "inner_type": "Contract",      // | "System" | "Diagnostic"
  "contract_id": "C…",           // strkey C-address, or null for system events
  "topic_0": "swap",             // best-effort string form (Symbol/String/Bytes)
  "topics": [/* ScVal JSON */],
  "data":   /* ScVal JSON */
}
```

ScVal JSON is the natural serde form of `stellar-xdr` types — e.g.
`{"symbol":"swap"}`, `{"address":"G…"}`, `{"u128":"100"}`,
`{"vec":[…]}`, `{"map":[{"key":…,"val":…}]}`.
