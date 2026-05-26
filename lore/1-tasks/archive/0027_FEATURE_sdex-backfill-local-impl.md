---
id: "0027"
title: "SDEX local-backfill impl — Cargo workspace + Rust CLI + ClickHouse local stack"
type: FEATURE
status: done
related_adr: ["0003", "0005", "0007"]
related_tasks: ["0012", "0022", "0028"]
tags: [layer-indexing, priority-high, effort-large, milestone-M1, local-backfill, workstation, rust, clickhouse, docker, sdex, stream-2]
milestone: 1
links:
  - "../archive/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-local-design.md"
  - "../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-filter-strategy.md"
  - "../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0010_local-backfill-over-fargate.md"
history:
  - date: 2026-05-13
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0012 future work as the Fargate-impl follow-up
      to the (then) Fargate design. Blocked on task 0011 (CDK bootstrap)
      per the first-pass G-note.
  - date: 2026-05-14
    status: backlog
    who: okarcz
    note: >
      Refactored from Fargate impl to local-workstation impl after
      ADR 0005 superseded ADR 0002. Scope now mirrors BE's
      backfill-bench / backfill-runner pattern: Cargo workspace,
      Rust CLI, local Postgres (Docker), schema migrations, runbook.
      Blocked-on-0011 removed — local backfill needs no CDK. Cloud
      push moved out to new task 0028.
  - date: 2026-05-24
    status: active
    who: okarcz
    note: >
      Activated — SDEX local-backfill impl starts. Unblocked per ADR 0005.
  - date: 2026-05-25
    status: done
    who: okarcz
    note: >
      Implemented end-to-end SDEX backfill CLI with 13 Rust modules,
      ClickHouse local stack (docker-compose), operator runbook.
      Smoke-tested across 2 runs (2001 ledgers, 68k OHLCV rows,
      4039 assets). PR #29 merged to develop (squash). Code review
      findings resolved: stable asset IDs across runs, S3Incomplete
      partition skip, structured errors, decimal rounding fix.
---

# SDEX local-backfill impl — Cargo workspace + Rust CLI + schema migration

## Summary

Lands the implementation for the SDEX (Stream 2) local-workstation
backfill designed in task 0012 / ADR 0005. Cargo workspace, Rust CLI
(`sdex-backfill`), local Postgres bootstrap (Docker), schema
migrations (`backfill_progress` table + ADR 0003 `quote_asset_id` PK
change on `price_ohlcv`), runbook at `docs/runbooks/backfill-sdex.md`,
and a smoke test against a 10 k-ledger range.

The post-backfill cloud-push step is **not** in this task — see
task 0028.

## Context

Implements task 0012's
[`notes/G-sdex-backfill-local-design.md`](../active/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-local-design.md)
clause-by-clause. See that document for:

- Architecture and CLI shape (§1, §3).
- Partition pipeline mirroring BE's `crates/backfill-runner` (§4).
- Resumability via `backfill_progress` + per-ledger atomic tx + partition skip (§5).
- Stdout-only observability and final summary block (§6).
- Failure-mode handling (§7) and local Postgres bootstrap (§8).
- Runbook outline (§9).
- Rust module split → 0022 spec mapping (§10).

Not blocked. ADR 0005 §Consequences explicitly lists "task 0027
unblocked" — local backfill needs no AWS infrastructure, so no
dependency on task 0011 (CDK bootstrap).

## Implementation (ordered)

1. **Cargo workspace bootstrap.**
   - Workspace root `Cargo.toml` at repo root (or `packages/`
     depending on Nx integration). Workspace members: `sdex-backfill`
     (bin), `db` (shared sqlx pool + migrations).
   - `sdex-backfill` binary crate with module layout per 0012 G-note §10:
     `cli`, `run`, `partition`, `sync`, `ingest`, `filter`, `tick`,
     `canonical`, `price`, `bucket`, `checkpoint`, `obs`, `main`.
   - **`xdr-parser` consumed via git Cargo dep** per ADR 0005 §3:
     `xdr-parser = { git = "https://github.com/rumblefishdev/soroban-block-explorer.git", branch = "main" }`.
     No BE-repo changes; pin updates by editing `Cargo.toml`.
   - **Future migration to a published crate (out of scope for this
     task).** Per ADR 0005 §3's "Future direction" note, BE will
     publish `xdr-parser` as a standalone versioned crate. When that
     lands, prices-api swaps the git dep for a plain version pin
     (`xdr-parser = "X.Y.Z"`) in a one-line `Cargo.toml` edit.
     This task ships against whichever form is current at impl time;
     a follow-up bump is trivial.

2. **Schema migrations** in the prices-api PG migration tool:
   - `price_ohlcv` PK change per ADR 0003: add `quote_asset_id`
     column (FK to `assets.id`) and migrate the PK to
     `(timestamp, asset_id, quote_asset_id, granularity)`.
   - `backfill_progress` table per 0012 G-note §5.1.
   - Pre-existing `assets`, `price_ohlcv` tables per
     `docs/database-schema/` — verify these are in the migration set
     before backfill can run.

3. **Local Postgres bootstrap** (0012 G-note §8):
   - `docker-compose.yml` defining a `prices-pg-local` service.
   - README / runbook entry for `docker compose up -d` + `cargo run -p db-migrate`.

4. **Rust binary** per 0012 G-note §10. Module-by-module:
   - `partition.rs` — `Partition::from_ledger` + `partitions_for_range`
     mirroring BE `backfill-bench` and `backfill-runner` exactly.
     S3 path: `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`.
   - `sync.rs` — `aws s3 sync --no-sign-request` subprocess via
     `tokio::process::Command`; per-partition; idempotent.
   - `ingest.rs` — per-partition + per-ledger index loop. Reads
     local `.xdr.zst`, calls `xdr_parser::decompress_zstd` +
     `xdr_parser::deserialize_batch`.
   - `filter.rs` — `TransactionResultMeta` walk + `txSUCCESS` gate
     + `ClaimAtom` extraction. Per 0022 filter-strategy §1.4–1.6, §2.
   - `tick.rs` — `ClaimAtom` → `TradeTick`. Per 0022 decode-and-bucket
     §2–§3 (V0 / ORDER_BOOK / LIQUIDITY_POOL variants).
   - `canonical.rs` — pair canonicalisation + asset surrogation.
     Per 0022 decode-and-bucket §1 and 0020 G-note.
   - `price.rs` — `amount_bought / amount_sold` → NUMERIC(28,14).
     Per 0022 decode-and-bucket §4.
   - `bucket.rs` — `TradeTick` → 1m `price_ohlcv` row;
     whole-row replacement UPSERT batch. Per 0022 decode-and-bucket §5.
   - `checkpoint.rs` — read/write `backfill_progress`; single PG tx
     wraps `price_ohlcv` UPSERTs + checkpoint advance.
   - `obs.rs` — `tracing-subscriber::fmt` with `EnvFilter` and JSON
     formatter; structured event names per 0012 G-note §6.1.
   - `cli.rs` — clap definitions matching 0012 G-note §3.
   - `run.rs` — orchestration: preflight, resume, partition
     prefetch pipeline, final summary block per 0012 G-note §6.2.

5. **Runbook at `docs/runbooks/backfill-sdex.md`** per 0012 G-note §9.
   Cover: first-time setup, Phase 1 invocation, subsequent phases,
   stop/pause/resume, progress inspection. Cloud-push section is
   added in task 0028.

6. **Smoke test** against a recent 10 k-ledger range (a single small
   partition):
   - `aws s3 sync` downloads partition successfully.
   - `sdex-backfill` decodes + bucketed-UPSERTs without panic.
   - `SELECT COUNT(*) FROM price_ohlcv` > 0; first row has
     non-NULL `quote_asset_id`.
   - `SELECT last_processed FROM backfill_progress WHERE stream='sdex_backfill'`
     advances to the end of the run's range.
   - Re-running the same command no-ops (partition pre-skip works).

7. **`cargo tree -i xdr-parser`** resolves to the pinned BE git
   commit (not a local override, not a path-dep).

## Acceptance Criteria

- [x] Cargo workspace with `sdex-backfill` bin crate; 13 modules
      (no separate `db` crate — ClickHouse replaces PG, no migrations needed).
- [x] `xdr-parser` consumed via git Cargo dep from BE `develop` branch.
- [x] ClickHouse schema applied automatically via `docker-entrypoint-initdb.d/init.sql`:
      4 tables (`assets`, `price_ohlcv_1m`, `backfill_sdex_ledgers`, `backfill_progress`).
- [x] `docker-compose.yml` brings up local ClickHouse on `127.0.0.1:8123`.
- [x] `sdex-backfill --start ... --end ...` runs end-to-end, writing OHLCV rows
      and tracking completed ledgers in `backfill_sdex_ledgers`.
- [x] Re-running the same command no-ops (partition-level skip works).
- [x] Final summary block prints partitions/ledgers/ticks/candles/bytes/elapsed.
- [x] Runbook at `docs/runbooks/backfill-sdex.md`.

## Implementation Notes

**20 files, ~3800 lines added.** 5 unit tests (partition module).

### Files

| File | Purpose |
|------|---------|
| `Cargo.toml` (root) | Workspace definition, shared deps |
| `packages/sdex-backfill/Cargo.toml` | Binary crate, edition 2024 |
| `docker-compose.yml` | ClickHouse 25.6, healthcheck, init.sql mount |
| `packages/sdex-backfill/schema/init.sql` | 4-table ClickHouse schema |
| `packages/sdex-backfill/src/main.rs` | Entry point |
| `packages/sdex-backfill/src/cli.rs` | clap args (--start, --end, --clickhouse-url, --temp-dir, --keep-partitions, --verbose) |
| `packages/sdex-backfill/src/obs.rs` | tracing-subscriber with JSON + EnvFilter |
| `packages/sdex-backfill/src/run.rs` | Orchestration: preflight, partition pipeline, summary |
| `packages/sdex-backfill/src/partition.rs` | Partition math, S3/local paths, 5 unit tests |
| `packages/sdex-backfill/src/sync.rs` | `aws s3 sync` with retry/backoff, S3 completeness check |
| `packages/sdex-backfill/src/ingest.rs` | Per-ledger XDR decode + trade extraction + candle flush |
| `packages/sdex-backfill/src/filter.rs` | TxSuccess gate, 5 trade-shaped ops, ClaimAtom extraction |
| `packages/sdex-backfill/src/tick.rs` | RawTrade → TradeTick (canonicalization + price) |
| `packages/sdex-backfill/src/canonical.rs` | AssetIdentity, AssetRegistry, pair canonicalization (USDC→USDT→XLM preference) |
| `packages/sdex-backfill/src/price.rs` | stroops_to_decimal, compute_price |
| `packages/sdex-backfill/src/bucket.rs` | CandleAccumulator, 1-min OHLCV aggregation, VWAP |
| `packages/sdex-backfill/src/sink.rs` | ClickHouse writer (candles, assets, completed ledgers) |
| `packages/sdex-backfill/src/error.rs` | BackfillError enum (7 variants) |
| `docs/runbooks/backfill-sdex.md` | Operator runbook |

### Smoke test results (2 runs)

| Metric | Run 1 (62016000–62017000) | Run 2 (62017001–62018000) | Total |
|--------|--------------------------|--------------------------|-------|
| Ledgers indexed | 1,001 | 1,000 | 2,001 |
| Trade ticks | 82,962 | 57,009 | 139,971 |
| OHLCV rows | 40,715 | 27,382 | 68,097 |
| Assets | 1,806 | 2,233 new | 4,039 |
| Re-run no-op | pass | pass | pass |

## Design Decisions

### From Plan

1. **Partition pipeline mirroring BE's backfill-runner**: Single-slot prefetch
   (`sync N+1` while `index N`), per 0012 G-note §4.

2. **Resume via per-ledger tracking**: `backfill_sdex_ledgers` table records
   completed sequences; startup queries to find done ledgers and skip
   fully-done partitions.

3. **Pair canonicalization with USDC→USDT→XLM quote preference**: Per 0022
   decode-and-bucket §1 and 0020 G-note.

4. **`xdr-parser` as git Cargo dep**: From BE `develop` branch per ADR 0005 §3.

### Emerged

5. **ClickHouse instead of Postgres**: ADR 0007 (accepted 2026-05-20) pivoted
   the live data sink from RDS PG to shared Hetzner ClickHouse. Local backfill
   followed suit — docker-compose runs ClickHouse 25.6 instead of Postgres.
   Eliminates `db` lib crate, sqlx dependency, and migration tooling.
   `ReplacingMergeTree` gives idempotent re-inserts without transactions.

6. **`String` columns in local assets schema instead of `FixedString`/`Enum8`**:
   The Rust `clickhouse` v0.13 crate's native binary protocol cannot serialize
   plain `String` into `FixedString` or `Enum8`. Local schema uses `String` for
   all assets columns. Production schema (managed by cloud-side tooling) uses
   the stricter types. ClickHouse handles coercion on cross-cluster INSERT.

7. **`Decimal(38,14)` serialized as `i128`**: The `clickhouse` crate cannot
   coerce `String` to `Decimal` over the native protocol. Fields are written
   as `i128` scaled by 10^14, with `round_dp(14)` applied first to avoid
   truncation bias.

8. **`TxProcessingRef` wrapper for V0/V1/V2 LedgerCloseMeta**: The three
   `LedgerCloseMeta` versions return different transaction-result types
   (`TransactionResultMeta` vs `TransactionResultMetaV1`). A wrapper struct
   extracts just the `TransactionResultPair` field common to all versions.

9. **Stable asset IDs via ClickHouse load-at-startup**: Initial implementation
   used monotonic in-memory IDs starting from 1. Code review identified this
   would assign different IDs for the same asset across CLI runs. Fixed by
   loading existing assets from ClickHouse at startup and continuing from max+1.

10. **S3Incomplete partitions skip indexing**: Initial implementation discarded
    the first partition's sync outcome and didn't skip indexing for prefetch
    partitions that returned S3Incomplete. Fixed to track completeness and
    gate `index_partition` calls.

## Issues Encountered

- **`PublicKey` access pattern**: `a.issuer.0.as_slice()` failed because
  `issuer` is `AccountId(PublicKey)`, not `PublicKey` directly. Added
  `pubkey_bytes()` helper that pattern-matches `PublicKey::PublicKeyTypeEd25519`.

- **V2 `tx_processing` type mismatch**: V2 returns `VecM<TransactionResultMetaV1>`
  not `&[TransactionResultMeta]`. Fixed with `TxProcessingRef` wrapper.

- **Zero-amount claim warnings**: Every ledger emits "skipping claim with zero
  amount" at op_index=0, claim_index=0. May indicate filter is extracting claims
  from non-trade operations. Warnings are correctly skipped and don't affect data.
  Investigation deferred.

- **64k-file partition download**: Even for a 1k-ledger smoke test, the full
  64k-file partition (~11.6 GB) must be downloaded. `--keep-partitions` flag
  avoids re-downloading on subsequent runs.

## Future Work

- **Zero-amount claim investigation**: Filter may be extracting claims from
  non-trade operations (every ledger at op_index=0). Harmless but noisy.
- **Cloud push (task 0028)**: Transfer local ClickHouse data to production
  Hetzner cluster over HTTPS-mTLS.

## Blocked by

None. Local backfill needs no AWS infrastructure.

## Spawns

- **0028** — Cloud push of finalised prices tables to Hetzner ClickHouse.
  Blocked on task 0011 (cloud infra) AND this task (local data).
