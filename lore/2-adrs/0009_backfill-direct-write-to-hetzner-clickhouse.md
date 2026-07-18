---
id: "0009"
title: "Prices historical backfill writes directly to Hetzner ClickHouse over mTLS (Model B), not a local-stage-then-push"
status: accepted
deciders: [okarcz]
related_tasks: ["0053", "0028"]
related_adrs: ["0001", "0007"]
tags: [architecture, backfill, clickhouse, hetzner, mtls, cloud-push, direct-write, stream-1, stream-2]
links:
  - "../../../soroban-block-explorer/docs/architecture/indexing-pipeline/indexing-pipeline-overview.md"
history:
  - date: 2026-07-01
    status: accepted
    who: okarcz
    note: >
      Decided after checking BE's production approach and the A-vs-B analysis.
      Supersedes the "completion push to the Hetzner cloud CH; the local
      instance is torn down afterwards" clause of ADR 0001's 2026-07-01
      amendment. Operator rationale: BE's direct-write is already in production
      and well tested, and it lets `/backfill/status` update in real time.
---

# ADR 0009: Prices historical backfill writes directly to Hetzner ClickHouse over mTLS (Model B)

**Supersedes:** the push-model clause of [ADR 0001](./0001_stream1-clickhouse-sourced-amm-backfill.md)'s 2026-07-01 amendment (which committed to "local `prices.*` mirror → completion push → tear down local"). Everything else in ADR 0001's amendment — the **combined single-pass extraction**, download-once, forward discovery, dual `backfill_progress` rows — stands unchanged. This ADR changes only *how the decoded rows reach Hetzner*.

## Context

The rescoped [task 0053](../1-tasks/active/0053_FEATURE_soroban-amm-backfill-cli-stream-1-impl.md) combined single-pass backfill decodes each `LedgerCloseMeta` once into `prices.*` rows (SDEX + AMM + oracle). The open question was **how those rows land on the shared Hetzner ClickHouse**:

- **Model A — stage-then-push** ([task 0028](../1-tasks/backlog/0028_FEATURE_sdex-cloud-push.md)): decode into a local throwaway ClickHouse mirror, then a separate `sdex-cloud-push` CLI streams rows to Hetzner over the [0052](../1-tasks/archive/) mTLS client. Requires a natural-key `assets` surrogate-id remap (local ids assigned in isolation can diverge from cloud ids the live path wrote). This was the clause ADR 0001's amendment inherited — but *carried over from the superseded `soroban_events`-era design, never argued against direct-write*.
- **Model B — direct write:** point the backfill's sink at Hetzner (the 0052 mTLS client) and write rows straight to `prices.*` during the run, loading the asset registry from the target first so surrogate ids align by construction.

BE already solved the identical problem: `crates/backfill-runner --target clickhouse` runs on an operator workstation and **writes directly to Hetzner ClickHouse over the Caddy mTLS proxy** — the same connection the live Lambdas use (BE `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` §6). Their local ClickHouse is dev/pilot only.

## Decision

**Adopt Model B — the prices historical backfill writes directly to the shared Hetzner ClickHouse `prices.*` over the 0052 mTLS client, with no intermediate local mirror and no separate push step.** The asset registry is loaded from the target at run start (`AssetRegistry::from_existing`) so surrogate ids match what the live path (0038) wrote — eliminating the natural-key remap.

Rationale (operator, 2026-07-01):

1. **Prod-proven.** BE's direct-write-to-Hetzner backfill is already in production and well tested — we mirror a validated pattern rather than build a bespoke push (0028).
2. **Real-time `/backfill/status`.** Because rows land on Hetzner as they are decoded, the two `backfill_progress` rows can be advanced *during* the run, so `GET /backfill/status` is truthful in real time — not only after a terminal push.
3. **No assets remap.** Loading the registry from the target makes ids align by construction; the remap that was Model A's dominant complexity disappears.
4. **Simpler operability.** One phase (decode+write), not two (decode, then push); no local CH mirror to size, host, or tear down.

## Rationale details

- **Id alignment removes the ADR-0001-era hazard, not just the remap.** Because both the backfill and live now allocate surrogate ids against the *same* Hetzner `assets` table, backfill reuses live's ids for assets live already saw. The residual race — both minting a *new* id for a *different* new asset at the same instant — is narrow (historical-only vs tip-new assets) and is the same race BE runs with in production; accepted for v1, revisited only if observed.
- **Concurrency with live is safe under the existing rules.** No code coordination exists between backfill and live ([[backfill-live-no-code-coordination]]); the operator keeps ledger ranges disjoint and the activation split minute-aligned. Since backfill = old ledgers and live = new ledgers, the only shared minute is the seam, handled by the minute-aligned split (0053 decision 7). Same-ledger reprocessing is idempotent (`ReplacingMergeTree(version)`, backfill == live by construction).
- **Resume is unchanged and crash-safe.** `backfill_sdex_ledgers` now lives on Hetzner; a stop/re-run skips completed partitions and idempotently re-writes any in-flight partition.

## Alternatives Considered

### Model A — local stage then push (task 0028)
Rejected as the primary path. Its one genuine advantage — decode stays fully prod-free and inspectable, prod touched only at a single auditable push — is outweighed by the remap complexity, the second (non-overlapped) push pass, the local mirror to size/host/tear-down, and the loss of real-time status. Speed is ~a tie (both dominated by the identical ~5.2 TB download; the push delta is ~hours). Task 0028 is superseded by this ADR but retained for history.

### Hybrid — stage local, push via the direct-write engine
Rejected: keeps the local mirror and the two-phase operability while gaining little once Model A's remap is dropped anyway.

## Consequences

### Positive
- Real-time, truthful `/backfill/status` throughout the run.
- No `assets` surrogate-id remap; no separate push CLI; no local CH mirror lifecycle.
- Mirrors a production-validated BE pattern; reuses the 0052 mTLS client already proven by the live processor sink.
- Minimal local footprint: only the rolling per-partition ledger scratch (~20–25 GB), no accumulated candle store.

### Negative
- **Shared prod CH under write load for the whole multi-day run** (competes with live writes + API reads on `ch-prod-01`). Mitigation: chunked INSERTs, retry/backoff on the mTLS sink, and running during a lower-traffic window; the real full-history run is an explicit, confirmed operator action.
- **Partial history is visible on Hetzner between a stop and completion** (no offline staging). Acceptable: partial rows are correct per-minute, coverage fills on resume.
- **Decode is no longer prod-free/inspectable** before writing — loses the local-mirror dry-run affordance used during 0060/0053 sizing. Mitigation: local plaintext CH remains available for testing against a stand-in target.
- Requires an mTLS-writer credential (SM bundle) present to run — a guard against accidental prod writes, but also an operational prerequisite.

## References
- [ADR 0001](./0001_stream1-clickhouse-sourced-amm-backfill.md) — combined single-pass backfill (this ADR supersedes only its push-model clause)
- [ADR 0007](./0007_live-data-sink-on-shared-hetzner-clickhouse.md) — live data sink on shared Hetzner ClickHouse
- [Task 0053](../1-tasks/active/0053_FEATURE_soroban-amm-backfill-cli-stream-1-impl.md) — implementation
- [Task 0028](../1-tasks/backlog/0028_FEATURE_sdex-cloud-push.md) — superseded stage-then-push spec
- BE `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` §6 — BE's direct-write backfill
