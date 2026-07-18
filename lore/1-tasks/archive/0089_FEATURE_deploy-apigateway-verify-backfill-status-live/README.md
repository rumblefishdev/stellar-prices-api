---
id: "0089"
title: "Deploy Prices ApiGateway + verify GET /v1/backfill/status live (M1 AC)"
type: FEATURE
status: completed
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
  - date: 2026-07-08
    status: completed
    who: okarcz
    note: >
      DONE. Deployed Prices-production-ApiGateway (CREATE_COMPLETE); built +
      confirmed the api-handler asset was current (Compute no-diff). First real
      gateway call 404'd EVERY /v1 route — root-caused to the api-handler missing
      AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true, so lambda_http kept the
      /production stage in the path and axum matched nothing (health 200 only
      because it is a gateway mock; 0040 tested in-process only). Fixed by adding
      that env var in compute-stack.ts (mirrors BE), env-only Compute redeploy.
      GET /v1/backfill/status now returns 200 + a valid §4.5 BackfillStatus
      (sdex paused 79.62%%, ledgers_remaining 12914101; soroban_amm running),
      no-key → 403, health → 200. M1 AC "backfill/status endpoint live +
      returning valid progress data" satisfied. Would have 404'd the whole API in
      T2 — good early catch. Fix + completion on branch
      fix/0089-api-handler-stage-prefix-404.
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

- [x] `Prices-production-ApiGateway` deployed, `CREATE_COMPLETE` (2026-07-08).
      URL `https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/`.
- [x] `GET /health` returns 200 (unauthenticated liveness — API Gateway mock).
- [x] `GET /v1/backfill/status` with a valid `x-api-key` returns **200** and the
      §4.5 `BackfillStatus` envelope for both `sdex` and `soroban_amm` from the
      live `backfill_progress` rows. *Required the stage-prefix fix below.*
- [x] `progress_pct` ∈ [0,100] and `ledgers_remaining` = `target − current`. *Live
      sample: `sdex` paused, `current 50457424 / target 63371525`, `progress_pct
      79.62`, `ledgers_remaining 12914101` (= 63371525 − 50457424 ✓); `soroban_amm`
      running, `last_push_at 2026-07-07T19:37:03Z`, `realtime_tip_ledger 63371525`.*
- [x] A missing/invalid `x-api-key` is rejected (403) on the data route.
- [x] Result recorded (URL + sample response + 2026-07-08 timestamp — this AC list).

## Issues Encountered

**Every `/v1` route 404'd through API Gateway (stage prefix not stripped).** After
deploying ApiGateway, `GET /v1/backfill/status` (and `/v1/assets`, etc.) returned
**404** with a valid key, while `/health` returned 200 and a keyless call returned
403. Root cause: the api-handler Lambda was missing
**`AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true`**, so `lambda_http` handed axum the
path *with* the API Gateway stage (`/production/v1/backfill/status`), which matches
no route → 404 in ~2 ms (no CH call). `/health` masked it because it is a keyless
API Gateway **mock**, not a Lambda route, and 0040's tests are all in-process
`tower::oneshot` (never through the real gateway), so this had zero prior coverage.
**This would have 404'd the *entire* public API in T2, not just `/backfill/status`.**

**Fix (config-only, no Rust change):** added `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH:
'true'` to the api-handler `environment` in `ComputeStack`
(`compute-stack.ts`), mirroring BE's api-handler (BE documents the same env var in
`crates/api/src/common/edge_lock.rs`). Redeploy of `Prices-production-Compute` was
env-only (diff = one added variable on `ApiHandlerFunction`; ledger-processor
untouched). Post-deploy the endpoint returns 200 + the valid envelope above.

## Design Decisions

### Emerged

1. **Stage-prefix strip via env var, not a path-rewrite layer.** `lambda_http`'s
   `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH` is the intended, BE-matching mechanism —
   preferred over a bespoke tower layer stripping `/{stage}`. One env var, no code.
2. **Fix folded into 0089 rather than a separate bug task.** The defect surfaced
   as the direct blocker to this task's single AC (endpoint live), the fix is
   one line, and no other work depended on it — so it is recorded here as an
   emerged finding instead of spawning a bug task.

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
