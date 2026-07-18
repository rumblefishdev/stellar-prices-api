---
id: "0028"
title: "SDEX cloud-push — stream local price_ohlcv + assets to Hetzner ClickHouse prices.* after backfill"
type: FEATURE
status: superseded
related_adr: ["0003", "0005", "0007", "0009"]
related_tasks: ["0012", "0027", "0052", "0063", "0051", "0053"]
tags: [layer-indexing, priority-medium, effort-medium, milestone-M1, cloud-push, clickhouse, hetzner, mtls, sdex, stream-2, rust]
milestone: 1
links:
  - "../active/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-local-design.md"
  - "../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../../soroban-block-explorer/lore/2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md"
history:
  - date: 2026-05-14
    status: backlog
    who: operator
    note: >
      Spawned from task 0012 future work alongside the ADR 0005
      pivot. Implements the post-backfill cloud-push step sketched
      in 0012 G-note §11. Blocked on task 0027 (local backfill data
      exists) and the Hetzner CH target being provisioned.
  - date: 2026-06-19
    status: backlog
    who: claude
    note: >
      Refreshed to the post-ADR-0007 reality: source is the local
      ClickHouse from 0027 (not local Postgres), target is the shared
      Hetzner CH `prices.*` (not cloud RDS), push uses the 0052 mTLS
      client, idempotency is ReplacingMergeTree(version) (not Postgres
      ON CONFLICT / xmax). Repointed RDS blocker 0011 → provisioning
      0063 + schema 0051. Replaced personal username with "operator".
  - date: 2026-07-01
    status: superseded
    who: okarcz
    by: "0009"
    note: >
      **Superseded by ADR 0009 (direct-write, Model B).** The operator chose
      to write the backfill directly to Hetzner over the 0052 mTLS client
      (BE's prod-proven `backfill-runner --target clickhouse` pattern) rather
      than stage into a local mirror and run this separate push CLI. That
      dissolves this task's whole reason to exist: no local→cloud copy pass,
      and — because the backfill loads the asset registry from the target —
      **no `assets` surrogate-id natural-key remap** (this task's core
      complexity). The direct-write sink + real-time `backfill_progress`
      updates are folded into task 0053 (Steps 4–5). Retained for history.
---

> **⚠️ Superseded (2026-07-01) — see [ADR 0009](../../2-adrs/0009_backfill-direct-write-to-hetzner-clickhouse.md).**
> The backfill now writes **directly** to Hetzner (Model B); there is no separate
> push step and no `assets` remap. This spec is kept for history only. The
> mTLS-sink + progress-row work lives in task 0053, Steps 4–5.

# SDEX cloud-push — stream local `price_ohlcv` + `assets` to Hetzner CH `prices.*`

## Summary

Lands a small Rust CLI (`sdex-cloud-push`) that streams the finalised
prices tables (`price_ohlcv_1m` + `assets`) from the operator's local
ClickHouse (task 0027 output) to the shared Hetzner ClickHouse `prices.*`
database via the 0052 mTLS client (Caddy:443). The push is idempotent and
re-runnable; it resolves surrogate-id collisions on `assets` via
natural-key matching so the cloud DB can already contain rows written by
the live-ingestion Lambda (0038).

## Context

ADR 0005 (supersedes ADR 0002) commits Stream 2 SDEX backfill to a local
workstation pattern; the data is exposed to API consumers via a separate
push step described in §11 of task 0012's design G-note. This task is that
push step. ADR 0007 then pivoted the live sink from RDS Postgres to BE's
shared Hetzner ClickHouse — so both the local store (0027, already shipped
on local ClickHouse) and the cloud target are now ClickHouse, and the push
is a CH→CH copy over mTLS rather than a Postgres→RDS UPSERT.

The shape mirrors a narrowed version of BE's `crates/db-merge` (BE ADR
0040): natural-key remap on the only FK-source table (`assets`), then a
batched INSERT into `price_ohlcv_1m` keyed by `(timestamp, asset_id,
quote_asset_id, source)` per ADR 0003/0004. Idempotency comes from
`ReplacingMergeTree(version)` collapse, not row-level UPSERT.

Blocked on:

- **Task 0027** — local backfill must have produced data to push (done).
- **Task 0063** — the `prices` database, scoped users, and per-env mTLS
  cert/endpoint must be provisioned on the Hetzner box.
- **Task 0051** — the target `prices.*` schema must be applied.
- **Task 0052** — the shared mTLS CH client used for the cloud hop.

## Implementation

1. **`sdex-cloud-push` bin crate** alongside `sdex-backfill` in the Cargo
   workspace established by task 0027. Reads the local plaintext CH with
   the raw `clickhouse` crate; writes the Hetzner target with the 0052
   shared mTLS client (`packages/prices-clickhouse`, `aws-mtls` feature).

2. **CLI flags** (CH-flavoured update of 0012 G-note §11.1):
   ```bash
   sdex-cloud-push \
       --source-ch-url http://localhost:8123     # local CH (0027 output) \
       --target-db prices                         # Hetzner prices.* via Caddy:443 \
       --tables price_ohlcv_1m,assets \
       --since-ledger <N>                         # optional; defaults to all
   ```
   - `--source-ch-url` reads the local CH (no mTLS).
   - The target endpoint + per-env cert come from the 0052 client
     (`MTLS_SECRET_NAME` → AWS Secrets Manager `{cert,key,ca}` bundle).
   - `--tables` defaults to `assets,price_ohlcv_1m`.
   - `--since-ledger` filters local rows by the source ledger range.

3. **Assets remap** per 0012 G-note §11.2 (still required because 0027
   assigns local surrogate ids `max+1`, which can diverge from cloud ids
   the live Lambda already wrote):
   - For each local `assets` row, look up the cloud row by its natural key
     (the same unique columns the live-ingestion path keys on).
   - Build a `local_id → cloud_id` map in memory.
   - INSERT genuinely-new assets into the cloud table.

4. **`price_ohlcv_1m` push:**
   - Stream rows from local CH in batches (5–10k rows / round-trip).
   - Rewrite `asset_id` and `quote_asset_id` via the map from step 3.
   - INSERT into Hetzner `prices.price_ohlcv_1m`. `version` (per ADR 0004)
     carries the dedup key; `ReplacingMergeTree(version)` collapses any
     overlap with live-ingested or previously-pushed rows on background
     merge. No `ON CONFLICT` — CH has no row-level UPSERT.

5. **Idempotency:** the tool must be safely re-runnable. A re-run is a
   no-op once local and cloud are in sync — same `version`s collapse.
   Test: run the push twice; `SELECT count() … FINAL` is identical before
   and after the second run.

6. **Observability:** mirror `sdex-backfill`'s stdout-JSON tracing
   pattern. Stable event names:
   - `push_started`, `assets_remapped` (counts: new vs existing),
   - `price_ohlcv_batch` (per-batch summary),
   - `push_complete` (total counts + duration).

7. **Runbook section** added to `docs/runbooks/backfill-sdex.md`
   (created by task 0027): cloud-push step, when to run, what to verify
   post-push.

8. **Smoke test:** spin up a local "cloud stand-in" ClickHouse via
   docker-compose (plaintext, no mTLS), run `sdex-backfill` against a
   10k-ledger range to populate the source CH, then run `sdex-cloud-push`
   against the stand-in. Diff `SELECT count(), min(timestamp),
   max(timestamp) FROM price_ohlcv_1m FINAL` between source and target —
   should match.

## Acceptance Criteria

- [ ] `sdex-cloud-push` bin crate added to the workspace.
- [ ] `--source-ch-url` / `--target-db` / `--tables` / `--since-ledger`
      CLI flags implemented; target cert/endpoint resolved via the 0052
      client (`MTLS_SECRET_NAME` bundle).
- [ ] `assets` natural-key remap correctly handles three cases:
      (a) new row → INSERT + capture id, (b) existing row → reuse cloud
      id, (c) re-run after partial failure → idempotent.
- [ ] `price_ohlcv_1m` batched INSERT lands in Hetzner `prices.*` with
      the correct PK/`version` shape (ADR 0003/0004); duplicates collapse
      under `ReplacingMergeTree(version)`.
- [ ] Smoke test passes: backfill 10k-ledger range to local CH, push to
      the CH stand-in, row counts and aggregates match between source and
      target (`… FINAL`).
- [ ] Re-running the push on a synced source+target is a no-op (verified
      by `count() FINAL` diff).
- [ ] Runbook section in `docs/runbooks/backfill-sdex.md` covers
      first-push and subsequent-push operator workflows.

## Blocked by

- **0027** — local backfill must produce data to push (done).
- **0063** — Hetzner `prices` DB, scoped users, and per-env mTLS
  cert/endpoint provisioned (supersedes the old RDS dependency on 0011).
- **0051** — target `prices.*` schema applied.
- **0052** — shared mTLS CH client for the cloud hop.
