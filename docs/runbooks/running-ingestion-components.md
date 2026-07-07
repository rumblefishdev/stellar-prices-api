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
                 ┌────────────────────────────────────┐
 Galexie S3  ──► │ sdex-backfill (one CLI, history)   │──┐  source='sdex'
 (public)        │  --mode combined  [activation,tip]  │  │  + phoenix|soroswap|aquarius
                 │  --mode sdex-only [1,activation)    │  │  (combined pass only)
                 └────────────────────────────────────┘  │
                 ┌─────────────────────────┐              ▼
 S3 doorbell ──► │ prices-ledger-processor  │─►┌──────────────────────────┐
 (live tip)      │   (live, both sources)   │  │ prices.price_ohlcv_1m    │
                 └─────────────────────────┘  │  ReplacingMergeTree(ver) │
                                               └──────────┬───────────────┘
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

| Component                 | Kind                 | Writes                                                                                                                                                | How it runs                               | Built?       |
| ------------------------- | -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- | ------------ |
| `sdex-backfill`           | operator CLI         | `price_ohlcv_1m` (`sdex` + AMM sources), `assets`, `oracle_prices`, `backfill_progress`, `pool_registry`, `unresolved_pools`, `backfill_sdex_ledgers` | manual, `--mode`/`--start`/`--end`        | ✅           |
| `prices-ledger-processor` | Lambda / fixture CLI | `price_ohlcv_1m` (all sources), `oracle_prices`, `assets`                                                                                             | SQS doorbell (prod) or `--cursor` (local) | ✅           |
| `enrichment-worker`       | Lambda / CLI         | `price_ohlcv_1m` (`_usd` cols)                                                                                                                        | scheduled (prod) or CLI (local)           | ✅ prototype |
| `asset-discovery`         | scheduled Lambda     | `assets`, `discovery_state`                                                                                                                           | EventBridge `rate(1h)`                    | ✅           |
| `oracle-worker`           | scheduled Lambda     | `oracle_prices`                                                                                                                                       | EventBridge schedule                      | ✅           |
| `supply-worker`           | scheduled Lambda     | `asset_supply`                                                                                                                                        | EventBridge schedule                      | ✅           |
| `cleanup-worker`          | scheduled Lambda     | TTL/partition maintenance                                                                                                                             | EventBridge schedule                      | ✅           |
| `prices-api`              | HTTP Lambda          | reads only                                                                                                                                            | API Gateway (prod) or `serve` bin (local) | ✅           |

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

### 1. SDEX + Soroban AMM historical backfill (`sdex-backfill`)

**One CLI, two disjoint range invocations of the same single-pass engine.**
Each ledger's XDR is downloaded once (download is the bottleneck, not parsing),
so the two runs never download a ledger twice. **Full SDEX deep-dive:
[`backfill-sdex.md`](backfill-sdex.md).**

- **Soroban era — combined.** `--mode combined --start <activation> --end <tip>`
  extracts **SDEX + AMM + oracle** from one download per ledger
  (`source='sdex'` plus `phoenix`/`soroswap`/`aquarius`).
- **Pre-Soroban tail — sdex-only.** `--mode sdex-only --start 1 --end <activation-1>`
  extracts SDEX only — no AMM pools exist before activation.

Union coverage: SDEX = `[1, tip]`, AMM = `[activation, tip]`, no ledger
downloaded twice. Activation is pinned (`--activation-ledger`, default
`50457424` — Protocol 20, 2024-02-20).

```bash
cargo build --release -p sdex-backfill                        # local plaintext CH
cargo build --release -p sdex-backfill --features aws-mtls    # to direct-write Hetzner
```

| Flag                  | Env                 | Default                 | Meaning                                                                                               |
| --------------------- | ------------------- | ----------------------- | ----------------------------------------------------------------------------------------------------- |
| `--start`             | —                   | required                | First ledger, inclusive                                                                               |
| `--end`               | —                   | required                | Last ledger, inclusive                                                                                |
| `--mode`              | —                   | `combined`              | `combined` (SDEX+AMM+oracle, for `[activation,tip]`) or `sdex-only` (for `[1,activation)`)            |
| `--activation-ledger` | —                   | `50457424`              | Soroban activation — the range split point + `--mode` sanity check                                    |
| `--tip`               | —                   | `--end`                 | Chain tip = `backfill_progress.target_ledger` denominator; pass the **live** tip on the sdex-only run |
| `--transport`         | —                   | `local`                 | `local` (plaintext `--clickhouse-url`) or `hetzner` (mTLS direct-write; needs `--features aws-mtls`)  |
| `--clickhouse-url`    | `CLICKHOUSE_URL`    | `http://localhost:8123` | CH HTTP endpoint (transport=local)                                                                    |
| `--ch-domain`         | `CH_DOMAIN`         | —                       | Caddy host fronting Hetzner CH (transport=hetzner)                                                    |
| `--ch-database`       | `CH_DATABASE`       | `prices`                | Target database (transport=hetzner)                                                                   |
| `--mtls-cert-path`    | `MTLS_CERT_PATH`    | —                       | PEM client cert (transport=hetzner)                                                                   |
| `--mtls-key-path`     | `MTLS_KEY_PATH`     | —                       | PEM client key — read straight into rustls, never logged (transport=hetzner)                          |
| `--mtls-ca-path`      | `MTLS_CA_PATH`      | —                       | PEM CA bundle (transport=hetzner)                                                                     |
| `--temp-dir`          | `BACKFILL_TEMP_DIR` | `.temp/sdex-backfill`   | Partition scratch dir                                                                                 |
| `--keep-partitions`   | —                   | false                   | Keep downloads after indexing                                                                         |
| `--verbose` / `-v`    | —                   | false                   | Per-partition logs                                                                                    |

**Direct-write to Hetzner (ADR 0009).** `--transport hetzner` writes `prices.*`
straight to the shared cluster over the task-0052 mTLS client — no local mirror,
no separate push step — so `/backfill/status` updates in real time. Needs the
`aws-mtls` build and the operator's client bundle:

```bash
CH_DOMAIN=ch.sorobanscan.rumblefish.dev \
MTLS_CERT_PATH=… MTLS_KEY_PATH=… MTLS_CA_PATH=… \
target/release/sdex-backfill --transport hetzner \
    --mode combined --start 50457424 --end <TIP> --verbose
```

`--transport local` (default) writes plaintext to `--clickhouse-url` — for
testing against a stand-in CH, not a real run.

**Recommended run order.** Both orders are correct (the ranges are disjoint);
the order only sets what `/backfill/status` shows during the run:

- **Soroban-era first** (combined `[activation,tip]`) → `soroban_amm` completes
  and `sdex_archive` jumps to `current=activation` immediately (recent + AMM
  data first); the pre-Soroban SDEX tail follows as a later milestone.
- **Chronological** (sdex-only `[1,activation)` first, then combined) →
  `sdex_archive` advances monotonically `1→tip` (simplest honest progress), but
  you grind the long pre-Soroban tail before any recent/AMM data lands.

**What `/backfill/status` shows.** The run advances **both** `backfill_progress`
rows live: the combined run drives `soroban_amm` forward to `completed` **and**
sets `sdex_archive.current = activation` (so recent SDEX is not under-reported),
then leaves `sdex_archive` **`paused`** at completion — the resting "between the
two runs" state, set automatically so the §"freshness" alarm does not false-fire
while you prepare the tail run (no manual status flip needed). The sdex-only run
un-pauses `sdex_archive`, walks it toward genesis, and marks it `completed` only
when it started at ledger 1 **and** reached the activation boundary (a chunk that
stops short stays `running`/`paused`). `current_ledger` is merged monotonically
and a stored `completed` is never downgraded, so the two runs are safe in either
order. Both rows also carry the covered `[earliest, newest]_data_available` time
window. (The read-side `progress_pct` and timestamp exposure are finished in
**task 0073**.)

**The activation split is a seam.** The two SDEX ranges meet at activation —
treat it like any backfill/live seam (see the seam section): make the sdex-only
run's `--end` the **last ledger of a completed minute**, or accept one
undercounted boundary minute (healed by **task 0065**). The backfill checks this
for you: on the partition straddling the split it decodes the two ledgers on
either side and **logs a loud `WARN` naming the straddled minute** if the split
is not minute-aligned (and an `INFO` confirming a clean boundary otherwise), so a
boundary undercount is never silent. Set `--tip` to the live chain tip on the
sdex-only run so `sdex_archive`'s progress is measured against the whole chain,
not just the pre-Soroban range.

**Discovered pool registry.** A combined run persists every pool it classifies
to `prices.pool_registry` at run end and preloads it at run start — so a partial
re-backfill of a mid-history window still resolves pools created earlier. A
clean forward run from activation leaves `prices.unresolved_pools` **empty**; a
row there is an extractor gap to investigate (its `sample_topics` carries the
event shape), not silently dropped volume.

**Teardown.** Direct-write leaves nothing local to tear down beyond the
per-partition scratch (`--temp-dir`, auto-cleaned unless `--keep-partitions`).
If you exercised a local stand-in CH via `docker compose up -d clickhouse`, run
`docker compose down` when done (its data lives in a named volume).

**Stopping & resuming.** The run is safe to stop and restart — it
auto-resumes, no special flag.

- **How it checkpoints.** Progress is committed **per partition**: after a
  partition's candles/oracle rows are written, its ledger sequences are recorded
  into `backfill_sdex_ledgers` (`ingest.rs`, end of `index_partition`). On
  startup `load_completed(start, end)` skips any partition whose ledgers are all
  recorded (`partition_fully_done`), so completed partitions are **not
  re-downloaded and not re-decoded**.
- **How to stop.** `Ctrl-C` / `tmux kill` — there is no graceful-shutdown
  handler, and none is needed: completion is committed per partition, and candle
  writes are idempotent (`ReplacingMergeTree(version)`, `version =
ledger_seq*1000 + op_index` → re-writing a ledger replaces with identical
  values, never duplicates/undercounts). The **one partition in flight** when
  killed was not yet recorded → on restart it is simply re-downloaded and
  re-decoded from scratch. Only that single partition's partial work is lost.
- **How to resume.** Re-run the **same command verbatim** (same `--start`/`--end`
  /`--mode`). Local scratch is cleaned per partition, so there is nothing to tidy
  first.

**Caveat — AMM pool discovery on a resumed `combined` run.** The discovered pool
registry (`reg`) is held in memory across partitions and only flushed to
`prices.pool_registry` at **clean run-end** (`run.rs`, after the loop) — the
per-partition `backfill_sdex_ledgers` checkpoint advances independently. So if a
pool's **factory-create** event landed in a partition that completed _before_ an
interrupt, that discovery was only in memory and is lost; on restart that
partition is skipped (marked done) and the pool is **not re-discovered**. A later
**swap** for it then hits the unresolved path → recorded to
`prices.unresolved_pools` (volume unresolved), not resolved into a candle.
Exposure is limited to pools _created in the backfilled window that are not in the
preloaded seed_ (prod `pool_registry` is seeded with ~521 pools via task 0079, so
most historical pools resolve regardless).

- **Post-resume check** — after any resumed `combined` run, inspect the
  safety-net table:
  ```sql
  SELECT venue, count() AS pools, sum(swap_count) AS missed_swaps
  FROM prices.unresolved_pools FINAL GROUP BY venue ORDER BY missed_swaps DESC
  ```
  Empty / unchanged → nothing missed. Grew after the resume → those pools' volume
  was skipped; reconcile by re-seeding `pool_registry` and re-running just the
  affected ledger range (idempotent, safe to repeat).
- **Cleanest planned pause:** stop **between the two runs** (after `combined`
  completes, before the `sdex-only` tail) — the tail carries no AMM discovery
  state, so it resumes with zero caveat. A mid-`combined` pause is fine too; just
  run the `unresolved_pools` check above afterward.

### 2. AMM historical backfill — superseded (folded into §1)

The separate `soroban-amm-backfill` binary planned in early 0053 was
**superseded** by the combined single-pass model (task 0053 / ADR 0009). Soroban
AMM swaps are extracted by `sdex-backfill --mode combined` in the **same
download pass** as SDEX (§1) — there is no second binary and no second pass over
the Soroban era, and backfilled AMM candles are byte-identical to the live
processor's (same code, replayed from history).

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

| Task     | Why it matters here                                                       |
| -------- | ------------------------------------------------------------------------- |
| **0053** | Combined single-pass SDEX+AMM backfill — this runbook's §1 (`--mode`)     |
| **0073** | Read-side `/backfill/status`: expose the data-window + fix `progress_pct` |
| **0064** | Durable ClickHouse-backed cursor (replaces the `/tmp` stub)               |
| **0065** | Periodic OHLCV re-aggregation — heals boundary/seam undercounts           |
| **0066** | Ledger-processor `RustFunction` + lag metric (prod deploy)                |
| **0067** | `assets` column clobber by concurrent writers                             |
| **0070** | Production rollout — where live+backfill coordination gets nailed down    |

---

_Golden rule: give the live processor and each backfill **disjoint,
minute-aligned ledger ranges** per shared `source`. ClickHouse guarantees
no duplicates; only you can guarantee no undercounted seam._
