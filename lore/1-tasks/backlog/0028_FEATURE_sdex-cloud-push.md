---
id: '0028'
title: 'SDEX cloud-push — stream local price_ohlcv + assets to cloud RDS after backfill'
type: FEATURE
status: backlog
related_adr: ['0003', '0005']
related_tasks: ['0011', '0012', '0027']
tags:
  [
    layer-indexing,
    priority-medium,
    effort-medium,
    milestone-M1,
    cloud-push,
    clickhouse,
    hetzner,
    postgres,
    sdex,
    stream-2,
    rust,
  ]
milestone: 1
links:
  - '../active/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-local-design.md'
  - '../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md'
  - '../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md'
  - '../../../../soroban-block-explorer/lore/2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md'
history:
  - date: 2026-05-14
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0012 future work alongside the ADR 0005
      pivot. Implements the post-backfill cloud-push step sketched
      in 0012 G-note §11. Blocked on task 0011 (cloud RDS exists)
      and task 0027 (local backfill data exists).
---

# SDEX cloud-push — stream local `price_ohlcv` + `assets` to cloud RDS

## Summary

Lands a small Rust CLI (`sdex-cloud-push`) that streams the finalised
prices tables (`price_ohlcv` + `assets`) from the operator's local
Postgres (task 0027 output) to the cloud RDS instance (task 0011
output). The push is idempotent and re-runnable; it resolves
surrogate-id collisions on `assets` via natural-key matching so that
the cloud DB can already contain rows written by the live-ingestion
Lambda.

## Context

ADR 0005 (supersedes ADR 0002) commits Stream 2 SDEX backfill to a
local workstation pattern; the cloud is exposed to API consumers via
a separate push step described in §11 of task 0012's design G-note.
This task is that push step.

The shape mirrors a narrowed version of BE's `crates/db-merge`
(BE ADR 0040): natural-key remap on the only FK-source table
(`assets`), then a batched UPSERT into the downstream
`price_ohlcv` keyed by `(timestamp, asset_id, quote_asset_id,
granularity)` per ADR 0003. Two tables in scope, vs. BE's twelve —
significantly simpler.

Blocked on:

- **Task 0011** — Cloud RDS must exist (CDK bootstrap landing).
- **Task 0027** — Local backfill must have produced data to push.

## Implementation

1. **`sdex-cloud-push` bin crate** alongside `sdex-backfill` in the
   Cargo workspace established by task 0027. Reuses the `db` lib
   crate's sqlx pool.

2. **CLI flags** per 0012 G-note §11.1:

   ```bash
   sdex-cloud-push \
       --source-url postgres://...local... \
       --target-url postgres://...cloud... \
       --tables price_ohlcv,assets \
       --since-ledger <N>                # optional; defaults to all
   ```

   - `--source-url` reads `DATABASE_URL_LOCAL` env.
   - `--target-url` reads `DATABASE_URL_CLOUD` env.
   - `--tables` defaults to `assets,price_ohlcv`.
   - `--since-ledger` filters local rows by `MIN(ledger)` derived
     from `price_ohlcv` row's source ledger range. Optional.

3. **Assets remap** per 0012 G-note §11.2:
   - For each local `assets` row, look up the cloud row by its
     natural key (the same unique constraint columns the
     live-ingestion Lambda upserts on).
   - Build a `local_id → cloud_id` map in memory.
   - INSERT new assets into cloud, capturing the returned `id`s
     into the map.

4. **`price_ohlcv` push:**
   - Stream rows from local in batches (5-10k rows / round-trip).
   - Rewrite `asset_id` and `quote_asset_id` via the map from step 3.
   - `INSERT … ON CONFLICT (timestamp, asset_id, quote_asset_id, granularity)
DO UPDATE SET …` matching the whole-row replacement contract
     from task 0022 decode-and-bucket §5.4.

5. **Idempotency:** the tool must be safely re-runnable. A re-run
   should be a no-op when local and cloud are in sync. Test:
   run the push twice in a row; second run produces no row changes
   in cloud (verifiable via `xmax` or row-count diff).

6. **Observability:** mirror `sdex-backfill`'s stdout-JSON tracing
   pattern. Stable event names:
   - `push_started`, `assets_remapped` (with counts: new vs existing),
   - `price_ohlcv_batch` (per-batch summary),
   - `push_complete` (total counts + duration).

7. **Runbook section** added to `docs/runbooks/backfill-sdex.md`
   (created by task 0027): Cloud push step, when to run, what to
   verify post-push.

8. **Smoke test:** spin up a local "cloud stand-in" Postgres via
   docker-compose, run `sdex-backfill` against a 10k-ledger range
   to populate local, then run `sdex-cloud-push` against the
   stand-in. Diff `SELECT COUNT(*), MIN(timestamp), MAX(timestamp)
FROM price_ohlcv` between source and target — should match.

## Acceptance Criteria

- [ ] `sdex-cloud-push` bin crate added to the workspace.
- [ ] `--source-url` / `--target-url` / `--tables` / `--since-ledger`
      CLI flags implemented per 0012 G-note §11.1.
- [ ] `assets` natural-key remap correctly handles three cases:
      (a) new row → INSERT + capture id, (b) existing row → reuse
      cloud id, (c) re-run after partial failure → idempotent.
- [ ] `price_ohlcv` batched UPSERT preserves whole-row replacement
      semantics (task 0022 §5.4) on the new PK shape (ADR 0003).
- [ ] Smoke test passes: backfill 10k-ledger range to local, push to
      stand-in, row counts and aggregates match between source and target.
- [ ] Re-running the push on synced source+target is a no-op
      (verified by post-run row-count diff).
- [ ] Runbook section in `docs/runbooks/backfill-sdex.md` covers
      first-push and subsequent-push operator workflows.

## Blocked by

- **0011** — Cloud RDS must exist (CDK bootstrap landing).
- **0027** — Local backfill must produce data to push.
