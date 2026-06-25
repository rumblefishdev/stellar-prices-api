---
id: "0054"
title: "Asset Discovery Lambda — Tranche 1 minimal scope (populate ≥20 major assets)"
type: FEATURE
status: completed
related_adr: ["0006", "0007"]
related_tasks: ["0011", "0050", "0051", "0052", "0039", "0067"]
tags: [layer-indexing, priority-medium, effort-medium, milestone-M1, lambda, scheduler, rust, aws, clickhouse, asset-registry]
milestone: 1
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
  - date: 2026-06-25
    status: backlog
    who: oski
    note: >
      Confirmed as the owner of the Asset Discovery worker via 0039's
      open Q#2 → Option A (0039 reuses this binary + CDK, does NOT
      rebuild it; 0039 only adds the deferred Soroswap/Aquarius
      pool-registry maintenance). Supply fetch is NOT in this task —
      0039 split it into a separate `supply-worker` writing
      `prices.asset_supply`; this task stays discovery-only. Blockers
      0011/0051/0052 are cleared; ready to promote ahead of 0039's
      discovery step.
  - date: 2026-06-25
    status: active
    who: oski
    note: >
      Promoted backlog → active. All blockers now cleared: 0011/0051/0052
      archived, and the last one — 0050 (SNS fan-out) — completed today
      (delivered by BE 0306, prod 2026-06-22). Body reconciliation to the
      latest architecture (ReplacingMergeTree INSERT not UPSERT; identity
      columns only — `home_domain` enrichment deferred per the 0067
      clobber hazard) + implementation land on the impl branch, not this
      status-only push.
  - date: 2026-06-25
    status: completed
    who: oski
    note: >
      Implemented + completed on branch feat/0054_asset-discovery-lambda-
      tranche-1-minimal (PR #55), in 4 increments: (1) crate foundation +
      prices.discovery_state schema; (2) seed of 20 verified major assets
      (StellarExpert top-by-rating, checksum-validated) + organic
      trade-activity discovery (discover_window reusing extract_trades /
      process_ledger / decode_object + the Galexie S3 fetcher); (3) CDK —
      attach the Lambda to the existing rate(1h) assetDiscoveryRule + error
      alarm (prepare-only); (4) spec reconcile + close-out. Verified: 4 unit
      tests + 2 integration tests (real-fixture discover_window + seed
      idempotency) pass against local Docker ClickHouse; infra typecheck/
      build/lint green; lib + lambda binary compile; clippy clean. Deploy +
      cursor activation are operator follow-ups (prepare-only). Emerged
      decisions: trade-activity discovery (no issuance walker exists), the
      strkey checksum guard, and the 0067 home_domain hazard.
---

# Asset Discovery Lambda — Tranche 1 minimal scope

## Summary

> **Done 2026-06-25** (PR #55, prepare-only). As-built differs from the
> original sketch in two ways, reconciled below: writes are
> **ReplacingMergeTree INSERT** (CH has no UPSERT), and discovery is
> **trade-activity-based** (SDEX + Soroban AMM), not an account-entry
> issuance walker — none exists in the pipeline, and an asset that never
> trades has no price row to populate.

The Asset Discovery Lambda (EventBridge `rate(1 hour)`): each invocation
(1) **seeds** the major-asset baseline into `prices.assets`, then (2)
**scans** a window of recent ledgers from S3 and registers every asset
appearing in trades, advancing the `prices.discovery_state` high-water-mark.
It reuses `prices-ingest-core`'s tested `AssetRegistry` / `OhlcvWriter` /
`extract_trades` / `process_ledger` so rows are byte-identical to the live
ledger processor, and `prices-ledger-processor`'s Galexie S3 key scheme +
`ObjectFetcher`. Identity columns only — never `home_domain` (the task-0067
whole-row-clobber hazard).

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

- [x] `packages/asset-discovery` Lambda binary builds against
      `provided.al2023` (ARM64). `cargo lambda build` target; lib + binary
      compile, clippy clean.
- [x] CDK definition with the `rate(1 hour)` EventBridge rule — attached to
      the existing `assetDiscoveryRule` in `EventBridgeStack` + error alarm.
      infra typecheck/build/lint green. **(Deploy itself is the operator's
      prepare-only follow-up.)**
- [x] `prices.assets` carries **20 verified major assets** after the seed
      runs (`seed_meets_tranche1_bar` test + the integration seed test
      against local CH). Issuers checksum-validated as Stellar keys.
- [~] After 24h, ≥1 asset organically added beyond the seed — **verifiable
      only on a live deploy**; the mechanism is proven by the real-fixture
      `discover_window` integration test (assets extracted + written from
      ledgers 62460540–62460542).
- [x] Idempotent: re-running a window produces no duplicate rows
      (ReplacingMergeTree + `FINAL`) — asserted by both integration tests
      (seed re-run + empty-tail re-scan leaves count/cursor unchanged).
- [x] CloudWatch error alarm wired (informational; registry maintenance is
      non-critical).

## Dependencies (all cleared)

- **0011** — CDK Lambda + EventBridge scaffolding. **Archived.**
- **0050** — mTLS material + SNS/CH endpoint. **Completed** (SNS via BE 0306;
  CH tenant + certs via 0063).
- **0051** — `prices.assets` table. **Archived** (schema on prod).
- **0052** — shared mTLS CH client. **Archived.**

## Implementation Notes

- **`packages/asset-discovery`** — lib + EventBridge-scheduled Lambda binary
  (behind the `lambda` feature so default build/test stays lean). Reuses
  `prices-ingest-core` (asset model + decode + extract) and
  `prices-ledger-processor` (Galexie key scheme + `ObjectFetcher` /
  `S3Fetcher`); no XDR walking re-implemented.
- **Seed** — `seed/major_assets.json` (20 entries), parsed into
  `AssetIdentity`, written idempotently via `load_assets → from_existing →
  get_or_assign → write_assets`. `seed_issuers_are_valid_strkeys` fails the
  build on any malformed issuer.
- **Discovery** — `discover_window<F: ObjectFetcher>` scans `cursor+1` forward
  until the first S3 gap, registers SDEX + Soroban AMM assets, writes, and
  advances `prices.discovery_state` (new schema table). Generic fetcher →
  `LocalDiskFetcher` in tests, `S3Fetcher` in the Lambda.
- **CDK** — `EventBridgeStack` creates the Lambda (mirrors 0038: `pricesLambda
  Defaults`, baseline role, secrets extension, S3 read grant) and
  `rule.addTarget`s the existing `assetDiscoveryRule`.
- **Tests** — 4 unit + 2 integration (`#[ignore]`, local CH), all green;
  real-fixture discovery + seed idempotency verified end-to-end.

## Design Decisions

### From Plan

1. **Carve-out reused by 0039** (Q#2 Option A): 0039 consumes this binary +
   CDK, adding only pool-registry maintenance.
2. **ReplacingMergeTree INSERT, not UPSERT**: CH has no UPSERT; dedup is
   engine-side on the natural-key sort tuple, read with `FINAL`.

### Emerged

3. **Trade-activity discovery, not an issuance walker**: the pipeline has no
   account-entry / `ChangeTrust` scanner, and building one was out of
   proportion — discovery extracts assets from SDEX trades + Soroban AMM
   swaps (reusing `extract_trades`/`process_ledger`). An asset that never
   trades has no price row, so it needs no registry entry. The original §5.3
   "account entries" sketch is superseded.
4. **Strkey checksum guard**: a unit test validates every seed issuer is a
   well-formed Stellar ed25519 key — caught a corrupted source row
   (BTCLN/IDRT/XCHF collapsed onto one address), which was dropped.
5. **Reuse `prices-ledger-processor` for S3**: depend on its `galexie_key` +
   `ObjectFetcher` (S3Fetcher behind `lambda`) rather than duplicating the
   Galexie scheme — avoids key-scheme drift with the live processor.
6. **`discovery_state` activation, not a synth gate**: `INITIAL_DISCOVERY_
   LEDGER` is left unset in CDK so synth isn't gated; the binary seeds
   gracefully and scanning activates once the operator seeds the cursor.

## Deploy prerequisites (operator, prepare-only)

- [ ] `cargo lambda build -p asset-discovery --release --arm64` before
      `cdk synth`/deploy (produces the `Code.fromAsset` bootstrap).
- [ ] After deploy, **activate the ledger scan**: seed
      `prices.discovery_state` with `(worker='asset-discovery', last_ledger=N)`
      where N = the ledger to resume from (recent tip − 1), **or** set the
      `INITIAL_DISCOVERY_LEDGER` Lambda env. Until then the worker seeds only.
- [ ] Confirm the `prices/{env}` `ingestion` mTLS secret exists (shared with
      0038) — the worker writes `prices.assets` under that identity.

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
