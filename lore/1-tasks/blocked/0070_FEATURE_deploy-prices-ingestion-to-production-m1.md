---
id: "0070"
title: "Deploy prices live-ingestion + periodic workers to production (M1 Part E rollout)"
type: FEATURE
status: blocked
related_adr: ["0006", "0007"]
related_tasks: ["0038", "0039", "0050", "0052", "0063", "0064", "0047", "0053", "0076", "0077", "0078"]
tags: [layer-ops, milestone-M1, deploy, priority-high, effort-medium, aws, cdk, lambda, clickhouse, hetzner, cross-team]
milestone: 1
links:
  - "../active/0038_FEATURE_prices-ledger-processor-lambda/README.md"
  - "../archive/0039_FEATURE_prices-periodic-workers-lambda-set/README.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
history:
  - date: 2026-07-03
    status: blocked
    who: oski
    note: >
      Blocked on AMM live-coverage wiring before go-live (operator decision).
      Deploy prep is green — preconditions ✅, all 9 arm64 bootstraps built
      (build list corrected 5→9, see 0077), synth + diff clean (5 stacks, zero
      deletes). But investigation found live AMM prices need BOTH a pool_registry
      seed (0053 backfill) AND a live-processor preload fix (0078): today the live
      processor passes Registries::new() and never loads prices.pool_registry, so
      pre-existing AMM pools' live swaps go unresolved. SDEX live is unaffected.
      Holding for full AMM+SDEX coverage rather than shipping SDEX-only. Unblock
      when 0053 (seed) + 0078 (preload) land, then resume at Step 2 (cursor seed).
  - date: 2026-07-03
    status: active
    who: oski
    note: >
      Promoted backlog → active to start the production rollout. Two blockers
      cleared beforehand: prod CH schema drift closed (task 0076, PR #76 merged)
      and the cross-team S3→SNS fan-out + platform SSM keys + prices RBAC already
      live in prod (BE tasks 0306/0314). Remaining critical path is our-side only
      (build bootstraps → seed cursor → synth/diff → deploy → smoke-test) plus one
      BE confirm: live CLICKHOUSE_CN_USER_MAP maps suffixed prices-ingestion-production.
  - date: 2026-06-26
    status: backlog
    who: oski
    note: >
      Spawned from 0038's deferred "Part E" deploy work so the
      code-complete ledger-processor (0038) and periodic workers (0039)
      can be archived while the **pure production deployment** is tracked
      separately for the milestone-M1 plan. All engineering is done and all
      external gates are clear (BE 0227 done; 0047 deferred). This task is
      the operator/cross-team rollout: build the arm64 bootstraps, seed the
      cursor, coordinate BE's S3→SNS refactor, `cdk deploy`, and smoke-test
      live mTLS writes. Holds the deferred deploy acceptance criteria moved
      out of 0038/0039. Prepare-only until an explicit deploy go-ahead.
---

# Deploy prices live-ingestion + periodic workers to production (M1 Part E)

> **Prepare-only until go-ahead.** Per the standing policy, no
> `cdk bootstrap`/`deploy`, no cert issuance, no Caddy edits, and no live
> ClickHouse writes happen without explicit per-session operator approval.
> This task documents the rollout so it is ready to execute on that go-ahead.

## Summary

The milestone-M1 ingestion code is complete and merged but **not deployed**:
the **ledger processor** (0038, Compute stack) and the four **periodic
workers** (0039, EventBridge stack — cleanup/supply/oracle + asset-discovery)
all run only locally/in CI. This task is the production rollout: stand up the
AWS resources and prove live end-to-end ingestion into the shared Hetzner
`prices.*` ClickHouse over mTLS. No new application code — deployment,
cross-team coordination, and verification only.

## Context

- ADR 0007 (accepted) puts the live data sink on BE's shared Hetzner CH; the
  `prices.*` tenant is already provisioned + isolation-proven (0052/0063).
- BE 0227 (Hetzner Ansible + mTLS CA + production CH) is **done/archived** —
  the mTLS endpoint exists.
- The S3→SNS→SQS doorbell transport was agreed cross-team (0050): BE fans out
  their ledger-bucket notification via SNS; prices-api owns its own
  `prices-ingest-{env}` SQS queue + DLQ + Lambda.
- Throughput verification (0047) is **deferred** — there is no real combined
  load to measure until this rollout exists; run it here, BE-coordinated.
- Env: **production**, region **eu-central-1**, AWS profile `soroban-explorer`.
  CDK app `node dist/bin/production.js` (Makefile `*-production` targets).
  Stacks: `Prices-production-{Secrets,Compute,ApiGateway,EventBridge,Observability}`.

## Deploy runbook

Run from `infra/`. Each `make deploy-*` wraps
`cdk --app "node dist/bin/production.js" deploy … --require-approval broadening`.

### 0. Preconditions (verify, don't assume)
- [ ] AWS creds for the shared account active (`aws sts get-caller-identity`,
      profile `soroban-explorer`); CDK already bootstrapped in eu-central-1.
- [ ] BE 0227 endpoint reachable (Caddy:443 mTLS host); CH `prices.*` tenant +
      scoped users exist (0063).
- [ ] mTLS bundle secret for the `ingestion` identity exists in Secrets Manager
      (created out-of-band by the operator / Secrets stack) — name from
      `mtlsSecretName('production','ingestion')`. **Never read the key
      material**; only confirm the secret exists.
- [ ] Caddy `CLICKHOUSE_CN_USER_MAP` maps the prices ingestion cert CN →
      the CH user (BE-side config; coordinate).

### 1. Build the Lambda bootstrap binaries (arm64)
```
cargo lambda build --release --arm64 --features lambda \
  -p prices-ledger-processor -p asset-discovery \
  -p cleanup-worker -p supply-worker -p oracle-worker \
  -p enrichment-worker -p backfill-freshness-probe \
  -p mtls-notafter-probe -p prices-api
```
- [ ] Verify `target/lambda/<name>/bootstrap` exists for all **nine** assets the
      production app references. NOTE (2026-07-03): the earlier 5-crate list was
      stale — `synth-production` also needs enrichment-worker, the two 0056 probes,
      and the prices-api handler, or it fails `CannotFindAsset`. CI builds only 6
      and never runs `synth-production`, so it does not guard this (see follow-up 0077).

### 2. Seed the bootstrap cursor (operator, one-time)
- [ ] Create SSM `String` param `/prices/production/ledger-processor/initial-cursor`
      = the **last ledger already accounted for** (SDEX backfill
      `max(sequence) FROM prices.backfill_sdex_ledgers`, or `currentTip − 1`
      for forward-only). **Never `0`.** Synth fails fast if absent. Retired by 0064.

### 3. Cross-team: BE S3 → SNS fan-out (coordinate, blocking)
- [ ] BE deploys the bucket-notification refactor `S3 → SNS` (`SnsDestination`,
      `rawMessageDelivery: true`) on `production-stellar-ledger-data`.
- [ ] BE confirms the SNS topic ARN + grants this account cross-account
      `Subscribe`; topic ARN published to the agreed SSM key.

### 4. Synth + diff (no mutation)
```
make synth-production && make diff-production
```
- [ ] Review the diff: Secrets, Compute (ledger-processor Lambda + the
      `prices-ingest-production` SQS queue + DLQ + SNS subscription), EventBridge
      (4 worker Lambdas + rules + alarms), Observability. No unexpected deletes.

### 5. Deploy (the go-ahead gate)
```
make deploy-production-secrets        # if not already applied
make deploy-production-compute        # ledger processor + SQS/DLQ + SNS sub
make deploy-production-eventbridge    # 4 periodic workers + rules + alarms
make deploy-production-observability  # dashboards/alarms incl. lag_seconds
```
- [ ] Each stack `CREATE/UPDATE_COMPLETE`.

### 6. Smoke-test live ingestion (mTLS)
- [ ] Drop one ledger object event (or wait for the next live PutObject) →
      confirm the ledger-processor invocation succeeds, advances the cursor,
      and **rows land in `prices.price_ohlcv_1m`** over mTLS.
- [ ] Trigger each worker once (manual invoke): oracle writes `oracle_prices`;
      supply writes `asset_supply`; cleanup is a safe no-op on first run;
      discovery inserts/updates `assets`. The `current_prices` MV populates.
- [ ] Confirm no DLQ messages on `prices-ingest-production-dlq`.

### 7. Observability
- [ ] `prices.ledger_processor.lag_seconds` metric emitting; >60s sustained
      alarm wired and not firing under steady state.
- [ ] Per-Lambda error/duration alarms green; oracle + supply alarms remain
      informational.

### 8. Post-deploy
- [ ] Run **0047** throughput verification against the now-live combined load,
      with BE (joint `system.query_log`/`metric_log` review).
- [ ] Hand off cursor-bootstrap retirement to **0064**.

## Rollback
- `make destroy-production-eventbridge` / `-compute` removes the prices-side
  Lambdas/queues (BE's SNS fan-out is independent and harmless if left). The
  cursor SSM param + mTLS secret are data — leave them. No CH schema changes
  are made by this task (schema is owned by 0051/0039), so there is nothing to
  un-migrate.

## Acceptance Criteria

- [ ] All five arm64 bootstraps build and `cdk synth` packages them (Step 1).
- [ ] Cursor SSM param seeded with a sane last-accounted ledger (Step 2).
- [ ] BE S3→SNS fan-out live + cross-account subscription working (Step 3).
- [ ] Compute + EventBridge + Observability stacks deployed to production (Step 5).
- [ ] **Live mTLS write proven**: a real ledger produces rows in
      `prices.price_ohlcv_1m` end-to-end (moved from 0038). (Step 6)
- [ ] Each periodic worker writes its table on a live invoke; `current_prices`
      MV populates; no DLQ backlog. (moved from 0039 deploy scope)
- [ ] `lag_seconds` metric + >60s alarm live (moved from 0038). (Step 7)
- [ ] 0047 throughput verification scheduled/run with BE post-deploy (Step 8).

## Out of scope

- Application/code changes (all owned by 0038/0039; this is deploy-only).
- The read API (0040) — separate, not yet built.
- CH schema/MV DDL (owned by 0051/0039).
- mTLS CA / cert issuance mechanics (operator + BE 0227).
