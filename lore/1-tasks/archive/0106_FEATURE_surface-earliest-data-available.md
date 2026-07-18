---
id: "0106"
title: "Surface earliest_data_available in GET /backfill/status (AC 6)"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0089", "0102"]
tags: [layer-api, priority-medium, effort-small, milestone-M1, backfill, api]
links:
  - "../../../packages/prices-api/src/backfill/dto.rs"
history:
  - date: 2026-07-17
    status: completed
    who: okarcz
    note: >
      DONE — merged (PR #125, 74b36df), deployed to prod (make
      deploy-production-compute), and verified on the live endpoint:
      `GET /v1/backfill/status` now returns
      `sdex.earliest_data_available: "2015-11-18T03:47:00Z"` (and the AMM
      stream's). AC 6 is now verifiable directly from the endpoint as worded.
      Field threaded through queries_ch -> dto -> handler for both streams; DTO
      comment corrected (archive floor / available-to-backfill, not ingested);
      integration test seeds + asserts both streams. All ACs met.
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      Surfaced during the SCF AC-6 verification (0102). The Tranche-1 AC 6
      criterion names `sdex.earliest_data_available` in GET /backfill/status, but
      the handler omits it — the DTO comment said the column did not exist yet.
      Query 9 on prod shows `prices.backfill_progress` NOW HAS the column
      (sdex 2015-11-18 = archive floor, soroban_amm 2024-02-20), so the comment
      is stale and the field can be surfaced. Option A: expose it as-is (archive
      floor = earliest data AVAILABLE TO BACKFILL, not ingested depth).
---

# Surface earliest_data_available in GET /backfill/status

## Summary

Tranche-1 AC 6 is worded *"`sdex.earliest_data_available` in `GET
/backfill/status` shows a date approximately 6 months ago."* The api-handler
(`packages/prices-api/src/backfill/`) omits the field — its DTO comment records
that `prices.backfill_progress` had no such column when it was written. That is
now stale: the column exists on prod (measured 2026-07-17: sdex `2015-11-18`,
soroban_amm `2024-02-20`). Surface it so AC 6 is verifiable from the endpoint.

## Context

`earliest_data_available` is the earliest ledger available **to backfill** (the
public-archive floor), NOT the earliest candle ingested. The SDEX pre-Soroban
tail is still backfilling toward it; the ingested/queryable depth is ~2024-02
(see 0102 AC 6). The API contract and the SCF evidence both state this plainly,
so exposing the archive-floor value is not an overclaim.

## Implementation

- `queries_ch.rs` — add `earliest_data_available: Option<String>` to
  `ProgressRow`; select `formatDateTime(earliest_data_available, …)` (NULL-guarded
  like `last_push_at`).
- `dto.rs` — add `earliest_data_available: Option<String>` to `SdexStream` and
  `AmmStream`; replace the stale "column does not exist yet" note with what the
  field means (archive floor / available-to-backfill).
- `handlers.rs` — map the new field for both streams.
- Update/extend the handler unit test if one asserts the response shape.
- Deploy: `make deploy-production-compute` (api-handler Lambda) — operator step.

## Acceptance Criteria

- [x] `GET /v1/backfill/status` returns `sdex.earliest_data_available` (and the
      AMM stream's) as an ISO datetime — verified live: `2015-11-18T03:47:00Z`.
- [x] DTO comment corrected; no stale "no such column" claim.
- [x] Build + tests green; deployed to prod and verified on the live endpoint.
