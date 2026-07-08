---
id: "0089"
title: "Deploy Prices ApiGateway + verify GET /v1/backfill/status live (M1 AC)"
type: FEATURE
status: active
related_adr: ["0008"]
related_tasks: ["0040", "0055", "0070", "0088", "0082"]
tags: [layer-api, priority-high, effort-small, milestone-M1, apigateway, backfill-status, read-api, operational, deploy]
milestone: 1
links:
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-07-08
    status: active
    who: okarcz
    note: >
      Created to close the M1 AC "GET /backfill/status endpoint live and
      returning valid progress data". Endpoint CODE is complete + merged (task
      0040, PR #68; 0055 was the isolated carve-out, superseded by 0040) but the
      Prices-production-ApiGateway stack was **never deployed** — 0070 explicitly
      excluded it ("ApiGateway intentionally NOT deployed, read API = 0040, out of
      scope"), and `describe-stacks Prices-production-ApiGateway` returns
      "does not exist". No task owned the deploy + live check until now.
---

# Deploy Prices ApiGateway + verify `GET /v1/backfill/status` live

## Summary

Close the Milestone-1 acceptance criterion **"GET /backfill/status endpoint live
and returning valid progress data."** The endpoint is fully built and merged
(task **0040**, `packages/prices-api/src/backfill/handlers.rs`) but has **never
been deployed to production** — the `Prices-production-ApiGateway` stack does not
exist. This task deploys that stack and confirms the endpoint answers with a
valid progress envelope over the live `prices.backfill_progress` rows.

**No code work** — the read handler, envelope DTO, usage-plan, API-key auth, and
30s cache all landed in 0040. This is deploy + verify only.

## Context

- The API is a single axum Lambda (ADR 0008); every `/v1` route is a Lambda-proxy
  onto `ComputeStack.apiHandlerFunction`. The `ApiGateway` stack fronts it with
  the REST API, `apiKeyRequired: true` data routes, a UsagePlan (per-key
  `apiKeyRateLimit` req/s + daily quota), and an API key `prices-{env}-partner-key`.
- The endpoint returns the §4.5 envelope `BackfillStatus`:
  - `realtime_tip_ledger`
  - `sdex` → `{ status, current_ledger, start_ledger, target_ledger, progress_pct,
    ledgers_remaining, last_push_at }`
  - `soroban_amm` → `{ status, last_push_at, completed_at }`
- **Data already exists** to return: the two backfill chunks that ran (see 0088)
  populated `backfill_progress`, so the endpoint returns real (partial) progress
  immediately; it grows more meaningful as **0088**'s run advances.
- Two known shape deltas from 0040 (not bugs): `earliest_data_available` omitted
  pending the column (task **0073**); `realtime_tip_ledger` derived from the SDEX
  `target_ledger`, not a dedicated tip row.

## Implementation Plan

1. **Build the api-handler Lambda asset** (proxied by the stack):
   `cargo lambda build -p prices-api --release --arm64 --features lambda`.
2. **Deploy the stack:**
   `cd infra && AWS_PROFILE=soroban-explorer make deploy-production-apigateway`
   (target = `Prices-production-ApiGateway`). Confirm `CREATE_COMPLETE`.
3. **Read the outputs:** REST API id (also in SSM) + `ApiKeyId` CfnOutput; fetch
   the key value with `aws apigateway get-api-key --include-value` to call the
   `apiKeyRequired` route. Base URL = the stage invoke URL (no custom domain yet
   — ACM/WAF/CORS deferred).
4. **Verify liveness + envelope:** `GET /health` (liveness), then
   `GET /v1/backfill/status` with `x-api-key: <key>` → expect 200 and a valid
   `BackfillStatus` for both streams with sane `progress_pct` / `ledgers_remaining`.

## Acceptance Criteria

- [ ] `Prices-production-ApiGateway` deployed, `CREATE_COMPLETE`.
- [ ] `GET /health` returns 200 (unauthenticated liveness).
- [ ] `GET /v1/backfill/status` with a valid `x-api-key` returns **200** and the
      §4.5 `BackfillStatus` envelope populated for both `sdex` and `soroban_amm`
      from the live `backfill_progress` rows.
- [ ] `progress_pct` ∈ [0,100] and `ledgers_remaining` = `target − current`
      (non-negative) for `sdex_archive`; monotonic vs. the underlying rows.
- [ ] A missing/invalid `x-api-key` is rejected (403) on the data route
      (confirms the UsagePlan + key auth are wired).
- [ ] Result recorded (endpoint URL, sample response, timestamp) — either a note
      here or folded into 0082's post-deploy verification record.

## Out of scope

- Custom domain + ACM, WAF WebACL, CORS — explicitly deferred in the ApiGateway
  stack; not required for the M1 "live + valid data" AC.
- The rest of the public API surface (assets/ohlcv/batch/oracles) — those are
  the same deployed stack, but their T2 data-correctness verification is not this
  task. This task only gates the M1 `/backfill/status` AC.
- `earliest_data_available` column — task **0073** (backlog).

## Notes

- Standing rules keep the deploy + AWS calls operator-run
  ([[feedback-prepare-not-deploy]]); build + synth are fine to prepare.
- Depends on the api-handler Lambda existing in `ComputeStack` (deployed in 0070);
  if the handler asset predates 0040's routes, rebuild + redeploy Compute too.
- Sibling verification of workers + `current_prices` MV is task **0082**; the two
  together close the M1 post-deploy verification surface.
