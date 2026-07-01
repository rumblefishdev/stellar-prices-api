# Running the Prices Ingestion Components — Operator Guide

How to run each moving part of the prices pipeline, what its parameters
mean, and — critically — **how to run the live processor and the
backfills together without silently corrupting the OHLCV data.**

If you only read one section, read
[Concurrency & the live ↔ backfill seam](#concurrency--the-live--backfill-seam).

> Scope: this is the operator's how-to. For the _why_ behind the schema
> (engines, sort keys, the enrichment split) see
> [`../database-schema/`](../database-schema/) and the overview doc
> [`../prices-api-general-overview.md`](../prices-api-general-overview.md).
> The deep-dive SDEX runbook is [`backfill-sdex.md`](backfill-sdex.md).

---

## The pipeline at a glance

```
                 ┌─────────────────────────┐
 Galexie S3  ──► │ sdex-backfill (history)  │──┐
 (public)        └─────────────────────────┘  │
                 ┌─────────────────────────┐  │   source='sdex'
 Galexie S3  ──► │ soroban-amm-backfill     │──┤   source=phoenix|soroswap|aquarius
 (public)        │   (history, PLANNED 0053)│  │        │
                 └─────────────────────────┘  ▼        ▼
                 ┌─────────────────────────┐  ┌──────────────────────────┐
 S3 doorbell ──► │ prices-ledger-processor  │─►│ prices.price_ohlcv_1m    │
 (live tip)      │   (live, both sources)   │  │  ReplacingMergeTree(ver) │
                 └─────────────────────────┘  └──────────┬───────────────┘
                                                          │ (rollup MVs → _15m.._1M)
                 ┌─────────────────────────┐              ▼
 oracle-worker ─►│ prices.oracle_prices     │   enrichment-worker fills
                 └─────────────────────────┘   close_usd / volume_quote_usd
                                                          │
                                              mv_current_prices (REFRESH 1m,
                                              last-24h) → prices.current_prices
```

Everything writes to the shared `prices.*` ClickHouse schema. All fact
tables are `ReplacingMergeTree` — the engine collapses duplicate keys, so
you can never create double-counted rows. The one thing you _can_ create
by mis-sequencing writers is a **silently undercounted minute** — see the
seam section.

## Component summary

| Component                 | Kind                 | Writes                                                       | How it runs                               | Built?       |
| ------------------------- | -------------------- | ------------------------------------------------------------ | ----------------------------------------- | ------------ |
| `sdex-backfill`           | operator CLI         | `price_ohlcv_1m` (`sdex`), `assets`, `backfill_sdex_ledgers` | manual, `--start/--end`                   | ✅           |
| `soroban-amm-backfill`    | operator CLI         | `price_ohlcv_1m` (AMM sources)                               | manual (planned)                          | ⛔ task 0053 |
| `prices-ledger-processor` | Lambda / fixture CLI | `price_ohlcv_1m` (all sources), `oracle_prices`, `assets`    | SQS doorbell (prod) or `--cursor` (local) | ✅           |
| `enrichment-worker`       | Lambda / CLI         | `price_ohlcv_1m` (`_usd` cols)                               | scheduled (prod) or CLI (local)           | ✅ prototype |
| `asset-discovery`         | scheduled Lambda     | `assets`, `discovery_state`                                  | EventBridge `rate(1h)`                    | ✅           |
| `oracle-worker`           | scheduled Lambda     | `oracle_prices`                                              | EventBridge schedule                      | ✅           |
| `supply-worker`           | scheduled Lambda     | `asset_supply`                                               | EventBridge schedule                      | ✅           |
| `cleanup-worker`          | scheduled Lambda     | TTL/partition maintenance                                    | EventBridge schedule                      | ✅           |
| `prices-api`              | HTTP Lambda          | reads only                                                   | API Gateway (prod) or `serve` bin (local) | ✅           |

---

## Concurrency & the live ↔ backfill seam

**There is no code-level coordination between the live processor and the
backfills. Keeping their ledger ranges disjoint is entirely on you (the
operator).**

### Why overlap is dangerous (but never duplicates)

`price_ohlcv_1m` is:

```
ENGINE ReplacingMergeTree(version)                       -- version = ledger_seq*1000 + op_index
ORDER BY (asset_id, quote_asset_id, source, timestamp)   -- the dedup key
```

On a key collision the engine keeps the row with `max(version)` and
**replaces — it does not sum.** Each writer builds a minute candle from
only the trades _it_ saw. So if two writers both emit the **same
`(asset, quote, source, minute)`**, each produces a _partial_ candle, only
one survives, and the other's trades vanish → **a silently undercounted
minute.** Not a duplicate, not a hole in the time axis — a wrong number.

Two facts bound the blast radius:

- **`source` is part of the key.** The live processor emits both `sdex`
  and the AMM sources (`phoenix`/`soroswap`/`aquarius`). So
  AMM-backfill-vs-SDEX-live can never collide; only **same-`source`**
  overlaps are the hazard (SDEX-live vs SDEX-backfill, AMM-live vs
  AMM-backfill).
- **Re-processing is idempotent.** Re-running the _exact same_ ledgers in
  full yields byte-identical candles with the same `version`, so a full
  re-run is always safe. The danger is strictly _partial_ coverage of a
  bucket by each writer.

### This gap is wider than the handoff

Because each writer accumulates a whole contiguous run/partition in memory
and flushes once, minutes are whole _within_ a single run. But a minute
split across **two separate runs** is undercounted regardless of who runs
them:

- live invocation → next live invocation (every Lambda run boundary),
- backfill partition → next backfill partition,
- **backfill → live handoff** (the seam).

This is a known, accepted residual (`prices-ledger-processor/src/reconcile.rs`
header). The healer is **task 0065 (periodic-ohlcv-reaggregation)** — not
yet built. Until it ships, boundary minutes stay undercounted, so
minute-aligned handoffs matter.

### Safe operating rules

1. **Disjoint ranges, split on a minute boundary.** Pick a handoff ledger
   `H` that is the _last ledger of a completed minute_. Backfill runs
   `--end H`; the live cursor starts at `H` (so it processes `H+1…`). No
   minute is shared → no seam undercount. (Bonus: pick `H` on a partition
   boundary too and you also avoid the per-partition residual at the join.)
2. **Never let a backfill `--end` reach or exceed the live cursor** for a
   shared `source`. Any ledger overlap = undercounted minutes across the
   whole overlap, not just one seam.
3. **Backfill-vs-backfill re-runs are safe** — the SDEX backfill's
   `backfill_sdex_ledgers` resume set short-circuits done ledgers, and
   ReplacingMergeTree dedups the rest. (This does _not_ protect against
   backfill-vs-live: the live processor does not consult that resume set.)
4. **Don't re-write already-enriched buckets.** Ingest writes
   `close_usd = 0` / `volume_quote_usd = 0` (DEFAULT); enrichment fills
   them later. Re-ingesting an enriched minute (same key, `_usd = 0`) at a
   `version ≥` the enriched row resets it to 0. Same root cause — avoid
   overlapping/re-writing enriched windows.

### What is _not_ at risk

- **`current_prices`** — sole writer is `mv_current_prices`, and it only
  reads the last 24h. Backfilling old data cannot touch it.
- **`oracle_prices`** — only `oracle-worker` writes it; backfills don't.
- **`backfill_progress`** — distinct `task_name` per writer
  (`sdex_archive` vs `soroban_amm`); no collision.

---

## Component catalog

### 1. SDEX historical backfill (`sdex-backfill`)

Backfills classic DEX (`source='sdex'`) trade history from the public
Galexie archive. **Full runbook: [`backfill-sdex.md`](backfill-sdex.md).**

```bash
cargo build --release -p sdex-backfill
target/release/sdex-backfill --start <FIRST_LEDGER> --end <LAST_LEDGER> --verbose
```

| Flag                | Env                 | Default                 | Meaning                       |
| ------------------- | ------------------- | ----------------------- | ----------------------------- |
| `--start`           | —                   | required                | First ledger, inclusive       |
| `--end`             | —                   | required                | Last ledger, inclusive        |
| `--clickhouse-url`  | `CLICKHOUSE_URL`    | `http://localhost:8123` | CH HTTP endpoint              |
| `--temp-dir`        | `BACKFILL_TEMP_DIR` | `.temp/sdex-backfill`   | Partition scratch dir         |
| `--keep-partitions` | —                   | false                   | Keep downloads after indexing |
| `--verbose` / `-v`  | —                   | false                   | Per-partition logs            |

Resumable: `Ctrl-C` and re-run the same range; done partitions are
skipped. Runs older ranges in any order.

### 2. AMM historical backfill (`soroban-amm-backfill`) — PLANNED

Stream-1 backfill of Soroban AMM swaps (`phoenix`/`soroswap`/`aquarius`).
**Not yet built — task 0053** (blocked on 0017 local ClickHouse). When it
lands it will follow the same `--start/--end` shape and the same
disjoint-range rules against the live processor. Until then, AMM history
is empty except for whatever the live processor has seen at the tip.

### 3. Live ledger processor (`prices-ledger-processor`)

The always-on processor. Decodes each new ledger (classic + Soroban),
writes candles for **all** sources, plus oracle samples and newly-seen
assets. Reconcile loop: read cursor → fetch next contiguous ledger → decode
→ bucket → write → **advance cursor last** (crash-safe, idempotent).

**Production (Lambda, `--features lambda`):** driven by an SQS doorbell
(`S3 ObjectCreated → SNS (BE) → prices-ingest-{env} SQS → handler`).
`reservedConcurrency = 1` keeps runs serial (the ordering guarantee).
Configured by env, not flags:

| Env              | Default                  | Meaning                                       |
| ---------------- | ------------------------ | --------------------------------------------- |
| `BUCKET_NAME`    | required                 | Galexie ledger bucket                         |
| `INITIAL_CURSOR` | required (first boot)    | Ledger to resume from; run starts at cursor+1 |
| `CURSOR_FILE`    | `/tmp/prices-cursor.txt` | Checkpoint file (**ephemeral** — see below)   |
| `MAX_ITERATIONS` | 16                       | Max contiguous ledgers per invocation         |

> ⚠️ The cursor is still a `/tmp` file (`StubFileCursor`), lost on cold
> start — the durable ClickHouse-backed cursor is **task 0064**. Until
> then, `INITIAL_CURSOR` re-seeds on a cold container, so set it carefully
> and treat cursor durability as unsolved. Prod deploy of this Lambda is
> **task 0066 / 0070**.

**Local (fixture CLI, `prices-cli`):** runs against on-disk fixtures or a
local plaintext ClickHouse.

```bash
cargo run -p prices-ledger-processor --bin prices-cli -- \
    --cursor 62460539 --max-iterations 16 --fixtures-dir fixtures/ledgers
# add --dry-run to parse+bucket only (no writes), or --clickhouse-url for a real local CH
```

| Flag               | Env              | Default                 | Meaning                                |
| ------------------ | ---------------- | ----------------------- | -------------------------------------- |
| `--cursor`         | —                | required                | Start ledger; run processes cursor+1…  |
| `--max-iterations` | —                | 16                      | Contiguous ledgers per run             |
| `--fixtures-dir`   | —                | `fixtures/ledgers`      | Local Galexie fixture root             |
| `--cursor-file`    | —                | `out/cursor.txt`        | Where the checkpoint is written        |
| `--clickhouse-url` | `CLICKHOUSE_URL` | `http://localhost:8123` | CH endpoint (ignored with `--dry-run`) |
| `--dry-run`        | —                | false                   | Count rows, write nothing              |

### 4. Enrichment worker (`enrichment-worker`)

Fills the deferred USD columns — `close_usd` and `volume_quote_usd` —
by joining OHLCV candles to `oracle_prices` (forward-filled). Ingest writes
these as 0; this pass makes `mv_current_prices` and the USD read paths
meaningful. Prototype CLI:

```bash
cargo run -p enrichment-worker --bin enrichment-cli -- \
    --candidates fixtures/candidates.jsonl --oracle fixtures/oracle_prices.jsonl \
    --oracle-name reflector --window-s 300 --sink sql-file --out-dir out
```

| Flag            | Default                        | Meaning                                   |
| --------------- | ------------------------------ | ----------------------------------------- |
| `--candidates`  | `fixtures/candidates.jsonl`    | Candidate OHLCV rows to enrich            |
| `--oracle`      | `fixtures/oracle_prices.jsonl` | Oracle price bars                         |
| `--oracle-name` | `reflector`                    | Oracle source to match                    |
| `--window-s`    | 300                            | Max oracle-bar staleness for forward-fill |
| `--batch-size`  | 10000                          | Candidates per inner batch                |
| `--max-batches` | 20                             | Inner-loop cap per invocation             |
| `--sink`        | `stdout`                       | `stdout` or `sql-file`                    |
| `--out-dir`     | `out`                          | SQL-file sink output dir                  |

> Ordering: enrich a window **after** its ingest has settled, and don't
> re-ingest an enriched window (rule 4 above).

### 5. Periodic workers (scheduled Lambdas)

`asset-discovery`, `oracle-worker`, `supply-worker`, `cleanup-worker` run
on EventBridge schedules (see `infra/`), not as operator CLIs — there are
no ledger ranges to coordinate, so they don't participate in the seam.
They write disjoint tables (`assets`/`discovery_state`, `oracle_prices`,
`asset_supply`, and maintenance respectively). The one cross-writer caveat
is the **`assets` column clobber (task 0067)**: every writer that upserts
an asset stamps a fresh `updated_at` and can overwrite columns it doesn't
own (e.g. `home_domain` set by discovery). More concurrent writers → more
frequent clobbers; not a data gap, but a source of flapping asset metadata.

### 6. Prices API (`prices-api`)

Read-only. Prod: single axum Lambda behind API Gateway. Local smoke:

```bash
cargo run -p prices-api --features local-server --bin serve   # plaintext local CH
```

See [`../../packages/prices-api/README.md`](../../packages/prices-api/README.md)
and its `loadtest/` for the k6 harness.

---

## Related follow-ups

| Task     | Why it matters here                                                    |
| -------- | ---------------------------------------------------------------------- |
| **0053** | AMM historical backfill CLI — not yet built                            |
| **0064** | Durable ClickHouse-backed cursor (replaces the `/tmp` stub)            |
| **0065** | Periodic OHLCV re-aggregation — heals boundary/seam undercounts        |
| **0066** | Ledger-processor `RustFunction` + lag metric (prod deploy)             |
| **0067** | `assets` column clobber by concurrent writers                          |
| **0070** | Production rollout — where live+backfill coordination gets nailed down |

---

_Golden rule: give the live processor and each backfill **disjoint,
minute-aligned ledger ranges** per shared `source`. ClickHouse guarantees
no duplicates; only you can guarantee no undercounted seam._
