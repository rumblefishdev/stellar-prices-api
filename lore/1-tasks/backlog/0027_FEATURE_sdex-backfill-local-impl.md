---
id: "0027"
title: "SDEX local-backfill impl — Cargo workspace + Rust CLI + schema migration per task 0012 / ADR 0005"
type: FEATURE
status: backlog
related_adr: ["0003", "0005"]
related_tasks: ["0012", "0022", "0028"]
tags: [priority-high, effort-large, local-backfill, workstation, rust, postgres, docker, sdex, stream-2]
links:
  - "../active/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-local-design.md"
  - "../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
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

- [ ] Cargo workspace with `sdex-backfill` bin crate + `db` lib crate;
      module layout matches 0012 G-note §10.
- [ ] `xdr-parser` consumed via git Cargo dep at a pinned commit;
      verified by `cargo tree -i xdr-parser`.
- [ ] Schema migrations land: `backfill_progress` table + ADR 0003 PK change.
- [ ] `docker-compose.yml` brings up a local Postgres on `127.0.0.1:5432`.
- [ ] `sdex-backfill --start ... --end ... --database-url ...` runs
      end-to-end on a 10 k-ledger range, writing rows to `price_ohlcv`
      and advancing `backfill_progress.last_processed`.
- [ ] Re-running the same command no-ops (partition-level skip works).
- [ ] Final summary block prints partitions/ledgers/bytes/percentile counts.
- [ ] Runbook at `docs/runbooks/backfill-sdex.md` per 0012 G-note §9.

## Blocked by

None. Local backfill needs no AWS infrastructure.

## Spawns

- **0028** — Cloud push of finalised prices tables to RDS. Blocked
  on task 0011 (cloud RDS) AND this task (local data).
