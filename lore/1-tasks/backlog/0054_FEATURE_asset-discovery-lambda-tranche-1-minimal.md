---
id: "0054"
title: "Asset Discovery Lambda — Tranche 1 minimal scope (populate ≥20 major assets)"
type: FEATURE
status: backlog
related_adr: ["0006", "0007"]
related_tasks: ["0011", "0050", "0051", "0052", "0039"]
tags: [layer-indexing, priority-medium, effort-medium, milestone-M1, lambda, scheduler, rust, aws, clickhouse, asset-registry]
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../blocked/0039_FEATURE_prices-periodic-workers-lambda-set.md"
  - "./0050_FEATURE_be-side-prep-sns-mtls-prices-db-provisioning.md"
  - "./0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration.md"
  - "./0052_FEATURE_clickhouse-mtls-client-shared-crate.md"
history:
  - date: 2026-05-21
    status: backlog
    who: okarcz
    note: >
      Spawned during Tranche 1 task-set creation. The §9 Tranche 1
      bullet "Asset Discovery Lambda running; `prices.assets`
      populated for at least 20 major assets" is an acceptance
      criterion, but 0039 bundles all five periodic workers
      together with Tranche 2/3-scoped scope (price-updater,
      oracle, cleanup, rollup). Asset Discovery alone is in T1
      scope; carve it out so it can ship without dragging the
      other four with it.
---

# Asset Discovery Lambda — Tranche 1 minimal scope

## Summary

Build and deploy the Asset Discovery Lambda specified in §2.1 /
§5.3 (EventBridge `rate(1 hour)`), but limit the Tranche 1 scope
to: detect new classic asset issuances and new SEP-41 contract
deployments, UPSERT them into `prices.assets`, and ensure at
least 20 major assets (XLM, USDC, EURC, AQUA, BTC, ETH, plus
top-volume Soroban tokens) are present in the table by end of
Tranche 1.

Cross-source pool-registry maintenance (Soroswap / Aquarius API
integration) is deferred to 0039's broader periodic-worker scope.

## Context

The general-overview §2.1 lists Asset Discovery as a Tranche-1-budgeted
Lambda. §5.3 specifies trigger (`rate(1 hour)`), source (account
entries in `LedgerCloseMeta`), and output (`prices.assets`
UPSERT on `(asset_code, issuer_address, contract_address)` unique
key per §3.1).

0039 sweeps in the full five-worker bundle, but its scope and
gating events (post-0038 live ingestion working, post-T2-API)
make it impractical to ship in Tranche 1. The T1 acceptance
criterion only needs the Asset Discovery worker, so a carve-out
is warranted.

When 0039 unblocks and lands, it should consume this task's
binary and CDK definitions; this task is the foundation, not the
duplicate.

## Implementation Plan

### Step 1: Lambda binary crate

Add `packages/asset-discovery/` (binary crate). Depends on:

- `lambda_runtime` — `provided.al2` runtime per ADR 0006.
- 0052 shared CH client — UPSERTs to `prices.assets`.
- `aws_sdk_s3` — to read recent `LedgerCloseMeta` files from
  BE's `stellar-ledger-data/` bucket.
- `stellar-xdr` — decoding account entries + Soroban contract
  invocations.

### Step 2: Discovery logic

For each invocation (hourly):

1. Determine the ledger window: from `MAX(closed_at)` already
   processed (stored in a `prices.discovery_state` ReplacingMT
   table — small, single row) up to now.
2. List S3 keys for that window via the `stellar-ledger-data/`
   bucket prefix.
3. For each file, decode `LedgerCloseMeta` and walk:
   - `LedgerHeaderHistoryEntry.txSet` for `ChangeTrustOp` /
     `PaymentOp` that surface new classic assets.
   - `SorobanTransactionMeta.invocations` for new SEP-41
     contract deployments (matching the SEP-41 token interface
     signature).
4. UPSERT discovered assets into `prices.assets` via
   `INSERT … VALUES (...)` (ReplacingMergeTree handles dedup on
   the sort-key tuple).
5. Update `prices.discovery_state` with the new high-water-mark.

### Step 3: Seed the major-asset baseline

To meet the "20 major assets" Tranche 1 acceptance bar without
waiting for hours of discovery cycles, ship a small static seed
file (`packages/asset-discovery/seed/major_assets.toml`) listing
the well-known asset identities (XLM native + 19 mainstays).
On Lambda cold start, ensure the seed is present in
`prices.assets`; UPSERT against the natural-key tuple so this
is idempotent.

The seed is a Tranche 1 expedient. By Tranche 2 / 3 the
discovery worker should have organically populated the table
beyond the seed.

### Step 4: CDK + EventBridge wiring (depends on 0011)

- Lambda function definition: `provided.al2`, 512 MB, 60s
  timeout, no VPC, IAM role with `secretsmanager:GetSecretValue`
  on the mTLS material and read on the
  `stellar-ledger-data/` bucket.
- EventBridge Scheduler rule: `rate(1 hour)`.
- CloudWatch alarm: invocation error rate > 5% over 1 hour →
  SNS.

### Step 5: Tests

- Unit: classic asset detection given a recorded
  `LedgerCloseMeta` fixture with a `ChangeTrustOp`; SEP-41
  detection given a recorded Soroban contract creation.
- Integration: against a Docker CH with 0051's schema applied,
  run the binary against a recorded S3 fixture; assert
  `prices.assets` rows.
- Seed: verify 20 major assets are present after the seed
  step runs against an empty table.

## Acceptance Criteria

- [ ] `packages/asset-discovery` Lambda binary builds against
      `provided.al2`
- [ ] Deployed via CDK with `rate(1 hour)` EventBridge rule
- [ ] After cold start in any env, `prices.assets` contains
      ≥20 major assets (verified via `SELECT count() FROM
      prices.assets FINAL`)
- [ ] After 24 hours of operation, the discovery worker has
      organically added at least one asset beyond the seed
      (verified via timestamp diff)
- [ ] Discovery is idempotent: re-running on the same ledger
      window produces no duplicate `prices.assets` rows
- [ ] CloudWatch alarm wired for invocation error rate > 5%

## Blocked on

- **0011** — CDK Lambda + EventBridge scaffolding.
- **0050** — mTLS material + Hetzner CH endpoint provisioning.
- **0051** — `prices.assets` table must exist.
- **0052** — shared mTLS CH client.

## Out of scope

- Pool registry maintenance for Soroswap / Aquarius — that's
  read by 0038's extractors; the registry-update concern is
  in 0039's broader scope.
- Cross-classic-asset rollup (e.g. CMC-style listing rules) —
  separate product concern.
- Asset-metadata fetch from external sources (logos, descriptions)
  beyond `home_domain` — backlog if needed.

## Notes

- The seed list is **for Tranche 1 only**. By Tranche 2 the
  organic discovery loop should be self-sufficient; if it
  isn't, that's a separate bug to surface.
- When 0039 (full periodic-workers bundle) unblocks, this
  task's binary and CDK definitions should be consumed as-is;
  don't reimplement. 0039 covers price-updater, oracle,
  cleanup — Asset Discovery is already done here.
- The §3.1 `assets` natural-key tuple `(asset_code,
  issuer_address, contract_address)` is the sort key, so
  UPSERT semantics work via ReplacingMergeTree's natural
  collapse.
