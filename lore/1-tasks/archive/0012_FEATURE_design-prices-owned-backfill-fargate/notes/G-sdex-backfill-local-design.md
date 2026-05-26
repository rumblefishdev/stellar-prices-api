---
title: 'SDEX backfill on a local workstation — operational design'
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags:
  [sdex, backfill, local, workstation, postgres, cloud-push, stream-2, design]
links:
  - '../../../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md'
  - '../../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md'
  - '../../../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md'
  - '../../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-filter-strategy.md'
  - '../../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md'
  - '../../../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md'
  - '../../../../../../soroban-block-explorer/lore/2-adrs/0010_local-backfill-over-fargate.md'
history:
  - date: 2026-05-13
    status: mature
    who: okarcz
    note: >
      First-pass design, Fargate shape. Consumed ADR 0002.
  - date: 2026-05-14
    status: mature
    who: okarcz
    note: >
      Rewritten to match ADR 0005 (supersedes ADR 0002): local
      workstation CLI mirroring BE's backfill-bench / backfill-runner
      pattern; cloud push of finalised prices tables to RDS is a
      separate post-backfill step. The Rust module split and the
      mapping onto task 0022's filter + decode-and-bucket spec are
      unchanged; everything host-shaped (Fargate task def, IAM,
      CloudWatch, SNS) is gone.
---

# SDEX backfill on a local workstation — operational design

## 0. Scope and non-scope

This note is the **operational design** for the SDEX (Stream 2)
historical backfill, post-supersession of ADR 0002 by ADR 0005. The
backfill runs as a local Rust CLI on the operator's workstation,
mirroring BE's `crates/backfill-bench` and `crates/backfill-runner`
shape. A separate, smaller `sdex-cloud-push` tool streams the
finalised prices tables to cloud RDS once backfill is complete (or
reaches a release-ready threshold).

**In scope here:**

- CLI shape (clap subcommands, args, env).
- Partition-pipelined archive read via `aws s3 sync --no-sign-request`.
- Local Postgres schema additions (`backfill_progress`, ADR 0003 PK migration).
- Resumability contract (per-ledger atomic tx + partition-level skip).
- Observability — stdout tracing JSON, optional p95 latency report.
- Failure-mode taxonomy and operator response.
- Rust module split — 1:1 mapping onto task 0022's spec.
- Cloud-push contract (post-backfill, separate tool).
- Reuse of BE's `xdr-parser` crate via git Cargo dep.

**Out of scope (deferred to task 0027 / task 0028):**

- Concrete Cargo workspace layout — proposed in §10, finalised on impl.
- Cargo lockfile pin to a specific BE git SHA — chosen during impl.
- Cloud RDS provisioning — gated by task 0011 (CDK bootstrap).
- The `sdex-cloud-push` tool implementation — task 0028.
- Multi-laptop parallel backfill — explicitly v2 per ADR 0005 §9.

## 1. Architecture overview

```text
                                 ┌──────────────────────────────────────┐
                                 │ Stellar public history archive (S3)  │
                                 │  s3://aws-public-blockchain/         │
                                 │    v1.1/stellar/ledgers/pubnet/      │
                                 │  (anonymous, --no-sign-request)       │
                                 └─────────────────┬────────────────────┘
                                                   │  aws s3 sync
                                                   ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ OPERATOR WORKSTATION                                                    │
│                                                                          │
│   ┌────────────────────────┐    ┌────────────────────────────────────┐  │
│   │ .temp/backfill-sdex/   │    │ sdex-backfill (Rust CLI)            │  │
│   │   <HEX>--<s>-<e>/       │───▶│  partition prefetch pipeline       │  │
│   │     <hex>--<seq>.xdr.zst│    │  ├─ decompress + decode             │  │
│   │   (local scratch)       │    │  ├─ filter trade-shaped ops         │  │
│   └────────────────────────┘    │  ├─ ClaimAtom → TradeTick           │  │
│                                  │  ├─ TradeTick → 1m price_ohlcv      │  │
│                                  │  └─ UPSERT + advance checkpoint     │  │
│                                  └──────────────┬─────────────────────┘  │
│                                                 │                        │
│                                                 ▼                        │
│                                  ┌────────────────────────────────────┐  │
│                                  │ Local PostgreSQL (Docker)           │  │
│                                  │   • price_ohlcv (ADR 0003 PK)       │  │
│                                  │   • assets                          │  │
│                                  │   • backfill_progress               │  │
│                                  └──────────────┬─────────────────────┘  │
│                                                 │                        │
│                                                 │ POST-BACKFILL          │
│                                                 ▼                        │
│                                  ┌────────────────────────────────────┐  │
│                                  │ sdex-cloud-push (separate Rust CLI) │  │
│                                  │  ├─ stream price_ohlcv + assets     │  │
│                                  │  └─ COPY/INSERT … SELECT batches    │  │
│                                  └──────────────┬─────────────────────┘  │
└─────────────────────────────────────────────────┼────────────────────────┘
                                                  │ public internet
                                                  ▼
                                  ┌────────────────────────────────────┐
                                  │ Cloud Postgres (RDS, per task 0011) │
                                  │   • price_ohlcv (canonical)         │
                                  │   • assets                          │
                                  └────────────────────────────────────┘
```

**No AWS auth on the backfill path.** `aws s3 sync --no-sign-request`
hits the anonymous public bucket — same path BE consumes in
`backfill-bench`. The only AWS-touching component is
`sdex-cloud-push`, which uses standard PG credentials against cloud
RDS (no IAM-DB auth, no IAM role).

**No code in the BE repo.** BE's `xdr-parser` crate is consumed
read-only as a git Cargo dependency. We never PR into the BE repo;
we pin a commit and update the pin when convenient.

## 2. Direction and range strategy

ADR 0005 §9 keeps the ledger-1 → tip coverage from ADR 0002 but
delegates direction to the operator via the CLI's `--start`/`--end`
flags (BE pattern). The Tranche 1 UX gate ("≥ 6 months of recent
history exposed via `GET /backfill/status`") is met by running
**tip-backward chunks** for v1:

| Phase        | Invocation                                       | Wall-clock at 311 ledgers/s                    | Outcome                            |
| ------------ | ------------------------------------------------ | ---------------------------------------------- | ---------------------------------- |
| Phase 1      | `--start=<tip-1_100_000> --end=<tip>`            | ~1 h pure decode (≈ ½–1 day with archive sync) | Tranche 1 ≥ 6 months ready to push |
| Cloud push 1 | `sdex-cloud-push` of Phase 1 range               | minutes (bulk UPSERT)                          | Cloud RDS exposes Tranche 1 window |
| Phase 2      | `--start=<tip-12_000_000> --end=<tip-1_100_001>` | ~10 h decode                                   | ~1 year of older history           |
| Cloud push 2 | as above                                         |                                                | Cloud RDS extended                 |
| Phase N      | `--start=1 --end=<phase-N-1-floor>`              | ~12-16 days (full archive)                     | Full historical complete           |

The CLI itself walks ledgers in `--start..=--end` ascending order
within each invocation (BE's pattern — partitions iterate ascending).
"Tip-backward" is realised by the operator's choice of consecutive
non-overlapping ranges, not by reversing the in-binary walk. This
exactly matches how BE chunks parallel-laptop backfills (BE ADR
0040): each invocation is one disjoint range.

**Why not in-binary tip-backward decrement?** Three reasons:

1. **BE's pattern is ascending-only.** Their `partitions_for_range`
   returns partitions sorted by `start ASC`. Mirroring saves us
   re-implementing the (small but non-zero) prefetch-pipeline logic
   for descending walks.
2. **Operator already controls direction via chunks.** Tranche 1 UX
   is met by picking the right first chunk; no in-binary direction
   knob needed.
3. **Multi-laptop v2 readiness.** When/if v2 lands, every laptop's
   range is ascending; the design stays consistent.

UPSERT correctness is unchanged from the previous design: per task
0022 decode-and-bucket §5.4 the backfill writes whole-row replacement,
so any order — within a chunk, across chunks, across laptops — yields
the same final `price_ohlcv` row content.

## 3. CLI shape

The binary is `sdex-backfill`, modeled directly on BE's `backfill-bench`
(simpler) with optional features from `backfill-runner` (sink
preflight, dashboard). One subcommand for v1; a `status` subcommand
matching BE's runner is a v2 addition.

```rust
// Logical shape — final crate path lands in task 0027.
#[derive(Parser)]
#[command(name = "sdex-backfill", version)]
struct Cli {
    /// First ledger to index (inclusive).
    #[arg(long)]
    start: u32,

    /// Last ledger to index (inclusive).
    #[arg(long)]
    end: u32,

    /// Postgres connection string. Flag > DATABASE_URL env > error.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Local scratch directory for downloaded partitions.
    #[arg(long, env = "BACKFILL_TEMP_DIR", default_value = ".temp/sdex-backfill")]
    temp_dir: PathBuf,

    /// Delete each partition's local folder after indexing succeeds.
    /// Default: delete (caps disk at ≈ 2 × partition_size).
    /// Pass `--keep-partitions` for iteration / debugging.
    #[arg(long)]
    keep_partitions: bool,

    /// Enable per-ledger and per-partition progress logs.
    /// Default: warnings only; final summary always printed.
    #[arg(long, short)]
    verbose: bool,

    /// Fail the run (exit 1) if per-ledger p95 latency exceeds N ms.
    /// Unset = report only. Matches BE backfill-bench's --assert-p95-ms.
    #[arg(long)]
    assert_p95_ms: Option<u128>,
}
```

**Defaults are operator-friendly:**

- `database_url` is required but reads `DATABASE_URL` so the operator
  can `export DATABASE_URL=postgres://... && sdex-backfill --start ... --end ...`.
- `temp_dir` matches BE's `.temp/backfill-runner` shape but namespaces
  to `.temp/sdex-backfill/` so a workstation running both tools doesn't
  collide.
- `keep_partitions` defaults off — same as BE.

**No `--target` flag in v1.** BE's `backfill-runner` has
`--target postgres|clickhouse`; prices-api has only one sink
(Postgres) in v1, so the flag is omitted. v2 can add it if cloud-push
becomes a `--target cloud-rds` variant rather than a separate tool —
but for now keeping the tools separate is simpler.

## 4. Partition pipeline

Identical structure to BE's `crates/backfill-runner/src/run.rs`:

```text
preflight:
    - which `aws --version`  (panic if missing)
    - sink preflight (SELECT 1; panic if unreachable)

resume:
    completed ← SELECT processed ledger set in [start, end] from PG
    partitions ← partitions_for_range(start, end)
    todo ← partitions filter (not fully-done given `completed`)

prime:
    sync_partition(todo[0])     # foreground; required for cold start

main loop (for each partition i in todo):
    next_handle ← if i+1 in todo: spawn sync_partition(todo[i+1])

    index_partition(todo[i], start, end, sink, completed):
        for ledger in clamped(todo[i], start, end):
            if ledger in completed: continue
            bytes ← read local file from temp_dir
            xdr   ← xdr_parser::decompress_zstd(bytes)
            batch ← xdr_parser::deserialize_batch(xdr)
            for lcm in batch.ledger_close_metas:
                ticks ← extract_trade_ticks(lcm)    # task 0022 spec
                rows  ← bucket(ticks)               # task 0022 spec
                BEGIN;
                  UPSERT rows INTO price_ohlcv     # whole-row replacement
                  UPSERT row    INTO backfill_progress  # current_ledger advance
                COMMIT;

    if not --keep-partitions: rm -rf todo[i] local folder

    await next_handle                              # surface sync errors
```

**Partition size:** 64 000 ledgers — matches BE's
`PARTITION_SIZE`. The S3 layout uses `{HEX:08X}--{start}-{end}/`
where `HEX = u32::MAX - start`, mirroring BE's `Partition::from_ledger`
exactly. Per-ledger filename is `{u32::MAX - seq:08X}--{seq}.xdr.zst`.

**Disk bound:** ≈ 2 × partition size — one indexing, one prefetching.
Older partitions average ~50 MB compressed, recent partitions ~12 GB
(per BE's notes); plan for 25-30 GB free scratch on the workstation.

## 5. Resumability

### 5.1 `backfill_progress` table

DDL lands in task 0027. Logical shape:

```sql
CREATE TABLE backfill_progress (
    stream             TEXT PRIMARY KEY,           -- 'sdex_backfill' for v1
    last_processed     BIGINT NOT NULL,            -- highest fully-processed ledger seq in current chunk
    range_start        BIGINT NOT NULL,            -- as passed to --start
    range_end          BIGINT NOT NULL,            -- as passed to --end
    started_at         TIMESTAMPTZ NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL,
    chunk_id           TEXT NOT NULL                -- e.g. 'tip-1100000-tip' for ops debugging
);
```

A single row exists for the chunk currently being processed; a chunk
that completes leaves its row in place as the historical record.
Multiple historical rows are fine — `stream` is the PK so each chunk
must have a unique stream id like `sdex_backfill_2026_05_phase1`, OR
the table can be widened to `(stream, chunk_id)` PK in v2 if the
single-row pattern proves too restrictive. v1 ships with one
`stream='sdex_backfill'` row and overwrites between chunks.

### 5.2 Per-ledger atomicity (unchanged from previous design)

Each ledger's `price_ohlcv` UPSERTs commit together with the
`backfill_progress.last_processed` advance, in one PG transaction.
Crash mid-ledger → restart reprocesses that ledger from the archive
(idempotent under whole-row replacement per 0022 §5.4).

### 5.3 Partition-level skip (BE pattern)

Before downloading a partition, the runner queries `backfill_progress`
to compute the set of ledgers in `[start, end]` already in the local
DB. Any partition whose entire clamped range is already in the
checkpoint set is skipped — no S3 sync, no decode.

This means a `--start=1 --end=tip` invocation against a DB that
already has ledgers `tip-1.1M..tip` from a prior Phase 1 run will
only download / decode the genuinely-missing partitions.

### 5.4 `assets` table writes

The backfill emits new asset rows on first-seen via the existing
asset-discovery contract (per task 0022 G-note's pair canonicalisation
section). `assets` is upsert-by-natural-key with a surrogate `id
BIGSERIAL`. In single-laptop v1 this is fine — sequence allocation
is monotonic. v2 multi-laptop requires the same surrogate-id remap
pattern BE solved in their `db-merge` crate; deferred per ADR 0005.

## 6. Observability — stdout only

No CloudWatch, no SNS. The CLI emits structured tracing logs to
stdout (BE pattern, `tracing_subscriber::fmt`) and prints a summary
block on completion.

### 6.1 Tracing events (stable names)

```json
{
  "ts": "2026-05-14T08:21:14.512Z",
  "level": "info",
  "stream": "sdex_backfill",
  "ledger": 62442947,
  "event": "ledger_processed",
  "trade_ticks": 24,
  "upsert_rows": 12,
  "dur_ms": 3.41
}
```

Stable event names:

| Event                     | Emitted when                                     |
| ------------------------- | ------------------------------------------------ |
| `preflight_ok`            | aws CLI + sink reachable                         |
| `range_resolved`          | partitions enumerated + `completed` set loaded   |
| `partition_sync_complete` | `aws s3 sync` returned for a partition           |
| `partition_indexing`      | switching index focus to next partition          |
| `ledger_processed`        | every ledger (verbose) — primary throughput line |
| `progress_tick`           | every 100 ledgers (default), summary metrics     |
| `partition_done`          | end of partition; counts + duration              |
| `backfill_complete`       | terminal success                                 |
| `archive_fetch_failed`    | `aws s3 sync` returned non-zero                  |
| `pg_write_failed`         | terminal DB error                                |
| `parser_panic`            | XDR decode panic (typically a `stellar-xdr` bug) |

### 6.2 Final summary (always printed, even without `--verbose`)

Modeled on BE `backfill-runner::run::print_run_summary` and
`backfill-bench` end-of-run block:

```text
=== sdex-backfill complete ===
partitions processed:    47
ledgers indexed:         3_008_000
ledgers skipped (in DB): 11_456
trade ticks emitted:     1_204_788
price_ohlcv rows UPSERT: 481_220
total bytes downloaded:  142 GiB
parse total:             8 314 s
persist total:           2 207 s
ledger time (min / p50 / p95 / max): 1 / 3 / 8 / 142 ms
elapsed:                 9_812 s (≈ 2 h 43 m)
```

### 6.3 Optional p95 assert

`--assert-p95-ms 200` matches BE's `--assert-p95-ms` flag — exits
non-zero if p95 exceeds the threshold, useful for CI smoke tests
on small ranges. No SLO defined for v1; flag exists for future use.

## 7. Failure modes

### 7.1 Transient `aws s3 sync` failures

`aws s3 sync` is invoked as a subprocess. If it exits non-zero, the
CLI returns a typed `BackfillError::ArchiveFetch` and exits non-zero.
**No in-binary retry loop for `s3 sync`** — `aws-cli` already does
multipart retries internally with `S3_REQUEST_TIMEOUT` semantics.
Operator response: re-run the same CLI invocation; the resume logic
short-circuits already-downloaded partitions.

### 7.2 Postgres write failure

Treated as fatal. The current ledger's transaction rolls back,
checkpoint is not advanced, binary exits non-zero. On restart the
same ledger is retried (idempotent). If failure is persistent (auth,
schema mismatch, disk full), operator diagnoses PG-side.

### 7.3 Parser panic

`stellar-xdr` / `xdr-parser` panic on malformed XDR. Treated as
fatal — operator files an issue against the BE `xdr-parser` repo
with the offending ledger sequence. v2 may add an opt-in
`--skip-malformed` flag, but pre-emptive skip hides upstream bugs.

### 7.4 Workstation sleep / network drop

The CLI is interrupt-resumable by construction (per-ledger atomic
checkpoint + partition pre-skip). Operator response: re-run the
same invocation; resume picks up from `backfill_progress` and skips
already-downloaded partitions.

### 7.5 Insufficient disk space

`aws s3 sync` fails on partition write; CLI exits per §7.1. Operator
frees space (workspace cleanup, `--keep-partitions` was on?) and
re-runs.

### 7.6 OOM

Streaming decode keeps memory bounded; OOM is not expected on
modern workstations (≥ 8 GB RAM). If observed on legacy hardware,
mitigation is the same as BE's: reduce parallelism (we already use
single-slot prefetch) or run on a beefier machine.

## 8. Local Postgres bootstrap

Docker is the assumed shape, mirroring BE's local-PG pattern. A
`docker-compose.yml` lands in task 0027 alongside the binary:

```yaml
# Logical shape; task 0027 finalises.
services:
  prices-pg-local:
    image: postgres:16
    environment:
      POSTGRES_PASSWORD: postgres
      POSTGRES_DB: prices_api
    ports: ['5432:5432']
    volumes:
      - prices-pg-data:/var/lib/postgresql/data

volumes:
  prices-pg-data:
```

Schema migrations apply via the same `sqlx-migrate` or `refinery`
tool the prices-api API will use (decision deferred to task 0011 /
ADR-to-be when the API crate lands). For v1 of backfill the migrations
required are:

- ADR 0003: `price_ohlcv` PK includes `quote_asset_id` (`uidx_price_ohlcv_pk` redefined as `(timestamp, asset_id, quote_asset_id, granularity)`).
- New: `backfill_progress` table (§5.1).
- Pre-existing: `assets`, `price_ohlcv` tables per `docs/database-schema/`.

## 9. Runbook outline

Full runbook lands at `docs/runbooks/backfill-sdex.md` in task 0027.
Structure:

### 9.1 First-time setup

```bash
# 1. Install aws CLI (no credentials needed — anonymous bucket).
which aws || { echo "install aws-cli"; exit 1; }

# 2. Start local Postgres.
docker compose up -d prices-pg-local

# 3. Apply migrations.
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/prices_api \
    cargo run -p db-migrate

# 4. Build the backfill CLI in release mode.
cargo build --release -p sdex-backfill
```

### 9.2 Phase 1 — recent 6 months (Tranche 1 UX gate)

```bash
TIP=$(curl -s https://horizon.stellar.org/ledgers?order=desc | jq '.[0].sequence')
# 1 100 000 ledgers ≈ 6 months at ~5s ledger cadence.

DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/prices_api \
target/release/sdex-backfill \
    --start $((TIP - 1100000)) \
    --end $TIP \
    --verbose
```

Wall-clock: ≈ 1 h pure decode + archive sync overhead → ½ – 1 day.

### 9.3 Cloud push (post-backfill, requires task 0028)

```bash
DATABASE_URL_LOCAL=postgres://...local... \
DATABASE_URL_CLOUD=postgres://...cloud... \
target/release/sdex-cloud-push \
    --since-chunk Phase1 \
    --tables price_ohlcv,assets
```

### 9.4 Subsequent phases

Repeat §9.2 with older ranges. The resume logic short-circuits
overlapping ledger sets, so concurrent or overlapping operator
invocations are safe (each just no-ops on already-processed ledgers).

### 9.5 Stop / pause

`Ctrl-C` interrupts the CLI. Per-ledger atomic commit means the
next invocation resumes from a clean checkpoint.

### 9.6 Inspect progress

```bash
psql $DATABASE_URL -c "
  SELECT stream, last_processed, range_start, range_end, updated_at, chunk_id
    FROM backfill_progress
   WHERE stream = 'sdex_backfill';"

psql $DATABASE_URL -c "
  SELECT MIN(timestamp), MAX(timestamp), COUNT(*) FROM price_ohlcv;"
```

## 10. Rust module split — mapping to task 0022's spec

Workspace layout (final paths land in task 0027):

```text
packages/
  sdex-backfill/                # Bin crate — CLI entry
    src/
      main.rs
      cli.rs                    # clap definitions, env handling
      run.rs                    # orchestration (modeled on BE run.rs)
      partition.rs              # Partition struct + partitions_for_range
      sync.rs                   # aws s3 sync subprocess wrapper
      ingest.rs                 # per-partition + per-ledger index loop
      filter.rs                 # post-decode OperationResultTr walk (0022 §1.4–1.6)
      tick.rs                   # ClaimAtom → TradeTick (0022 §2–§3)
      canonical.rs              # pair canonicalisation (0022 §1, 0020 G-note)
      price.rs                  # amount_bought / amount_sold → Decimal (0022 §4)
      bucket.rs                 # TradeTick → price_ohlcv UPSERT (0022 §5)
      checkpoint.rs             # backfill_progress read/write
      obs.rs                    # tracing-subscriber setup

  sdex-cloud-push/              # Separate bin crate — task 0028
    src/
      main.rs

  db/                           # Shared sqlx pool + migrations
    src/
      pool.rs
      migrations/

Cargo.toml (workspace root)
[workspace.dependencies]
xdr-parser  = { git = "https://github.com/rumblefishdev/soroban-block-explorer.git", branch = "main" }
stellar-xdr = "26"              # via xdr-parser, but also accessible direct
tokio       = { version = "1", features = ["full"] }
sqlx        = { version = "0.8", features = ["postgres", "macros", "runtime-tokio"] }
clap        = { version = "4", features = ["derive", "env"] }
tracing     = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

Mapping onto 0022 spec sections (unchanged from the previous design's
§10 — the spec is host-shape-agnostic):

| Rust module  | Spec source                               | Responsibility                                                                 |
| ------------ | ----------------------------------------- | ------------------------------------------------------------------------------ |
| `sync`       | this design §1, §4                        | `aws s3 sync --no-sign-request` subprocess; per-partition.                     |
| `partition`  | this design §4                            | `Partition::from_ledger` + `partitions_for_range`; mirror BE's layout exactly. |
| `ingest`     | this design §4, §5                        | Per-partition loop; per-ledger atomic tx; checkpoint advance.                  |
| `filter`     | 0022 filter-strategy §1.4-1.6, §2         | `TransactionResultMeta` walk; `txSUCCESS` gate; `ClaimAtom` extraction.        |
| `tick`       | 0022 decode-and-bucket §2-§3; 0020 G-note | `ClaimAtom` → `TradeTick`. V0 / ORDER_BOOK / LIQUIDITY_POOL decode.            |
| `canonical`  | 0022 decode-and-bucket §1; 0020 G-note    | Asset canonicalisation, base/quote orientation, surrogation.                   |
| `price`      | 0022 decode-and-bucket §4                 | `amount_bought / amount_sold` → NUMERIC(28,14) with precision policy.          |
| `bucket`     | 0022 decode-and-bucket §5                 | `TradeTick` → 1m `price_ohlcv` row; whole-row replacement UPSERT batch.        |
| `checkpoint` | this design §5                            | Read/write `backfill_progress`; transactional commit per ledger.               |
| `obs`        | this design §6                            | `tracing` subscriber on stdout; structured JSON.                               |
| `run`/`main` | this design §3, §4                        | Orchestration: preflight, resume, partition pipeline, summary.                 |

**`xdr-parser` consumed via git Cargo dep** per ADR 0005 §3.
Verification: `cargo tree -i xdr-parser` resolves to the pinned BE
git commit. No BE-repo changes; the pin can be updated by editing
`Cargo.toml` and `cargo update -p xdr-parser`.

**Planned migration to a published crate.** Per ADR 0005 §3's
"Future direction" note, `xdr-parser` will be released as a
standalone versioned crate independent of the BE workspace. When
that happens, the workspace dep becomes a plain version pin
(`xdr-parser = "X.Y.Z"`) and the `git = "…"` form is dropped. The
cutover is a one-line edit to `Cargo.toml`; v1 of the backfill CLI
is designed to work against either form. No code change is
required because both forms expose the same crate API.

## 11. Cloud-push design (post-backfill)

This is a sketch — the full design lands when task 0028 is activated.
Captured here so the impl task and operators know what's coming.

### 11.1 Tool shape

```bash
sdex-cloud-push \
    --source-url postgres://...local... \
    --target-url postgres://...cloud... \
    --tables price_ohlcv,assets \
    --since-ledger <N>                    # optional; default: all
```

A small Rust binary (≈ 200-500 LOC). Streams rows from local PG to
cloud PG. Two strategies, decided on impl:

- **`COPY … TO STDOUT` + `COPY … FROM STDIN`** — fastest, but no
  UPSERT semantics. Requires "empty target" or table-truncate
  semantics that drop history.
- **`INSERT … SELECT … ON CONFLICT DO UPDATE`** — slower, but
  preserves the whole-row-replacement semantics from 0022 §5.4.
  Batched (typically 5-10k rows / round-trip).

Recommendation (decided at impl): start with the `INSERT … ON
CONFLICT` strategy. `COPY` is faster but the loss of UPSERT
semantics means we'd have to either truncate the cloud table (loses
live-ingestion data accumulated since the last push) or pre-stage to
a temp table and merge — both more complex than batched UPSERTs.

### 11.2 Ordering

Push `assets` first, then `price_ohlcv` — `price_ohlcv` rows FK to
`assets.id` (via `asset_id`, `quote_asset_id`). Surrogate-id
collisions between local and cloud are possible if cloud already
has rows written by the live-ingestion Lambda. Push tool resolves
by natural-key matching:

```text
For each local assets row:
  SELECT id FROM cloud.assets WHERE <natural_key columns> = local;
  IF found: remap local.id → cloud.id in the local → cloud copy.
  ELSE:     INSERT into cloud.assets RETURNING id; use that id.

Then for each local price_ohlcv row:
  rewrite asset_id, quote_asset_id via the map computed above
  UPSERT into cloud.price_ohlcv on (timestamp, asset_id, quote_asset_id, granularity)
```

This is the same surrogate-remap pattern BE solved in their
`db-merge` crate (ADR 0040 §"Surrogate-id remap procedure"),
narrowed to the two tables prices-api needs.

### 11.3 Idempotency

The push tool can be re-run safely: `ON CONFLICT DO UPDATE` makes
each row idempotent; `assets` natural-key matching makes the
remap idempotent across runs.

### 11.4 Triggering live-ingestion handoff

Once a chunk is pushed, the Prices Ledger Processor Lambda (live
ingestion) is unaffected — its writes interleave with the pushed
backfill data via the same UPSERT semantics. There is no explicit
"handoff" event; the cloud DB simply has more historical rows after
each push.

## 12. Handoff — implementation checklists

### 12.1 Task 0027 — local backfill CLI

Per ADR 0005 §Decision points 1-6, 8, 9:

1. Cargo workspace bootstrap with `sdex-backfill` bin crate. Pin
   `xdr-parser` via git Cargo dep.
2. Schema migrations:
   - ADR 0003 PK: `price_ohlcv.PK = (timestamp, asset_id, quote_asset_id, granularity)`.
   - New `backfill_progress` table (§5.1).
3. Rust modules per §10. Each module reviewable against its cited
   spec section.
4. `docker-compose.yml` for local Postgres (§8).
5. Runbook at `docs/runbooks/backfill-sdex.md` per §9.
6. Smoke test: run against a 10 000-ledger recent range; assert
   `price_ohlcv` rows land, `backfill_progress.last_processed`
   advances, p95 latency printed.
7. Verify `cargo tree -i xdr-parser` resolves to the pinned BE
   git commit (no path-dep, no local override).

### 12.2 Task 0028 — cloud-push tool

Per §11 above and ADR 0005 §Decision point 7:

1. `sdex-cloud-push` bin crate alongside `sdex-backfill`.
2. `assets` natural-key remap (§11.2).
3. `price_ohlcv` batched UPSERT with FK rewrite via map.
4. CLI flags for source/target URLs, table list, optional
   `--since-ledger` filter.
5. Smoke test: push a small chunk to a docker-cloud-pg stand-in,
   diff against source.
6. Runbook section in `docs/runbooks/backfill-sdex.md` covering
   the push step.
7. Blocked on task 0011 (cloud RDS exists) and task 0027 (local
   data exists).
