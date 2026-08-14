---
id: "0119"
title: "Input-validation hardening — every path, query and body param rejected with 400 on invalid input"
type: FEATURE
status: active
related_adr: ["0006", "0008"]
related_tasks: ["0040", "0118", "0120"]
tags: [layer-backend, priority-high, effort-medium, milestone-M2, api, validation, security]
milestone: 2
links:
  - "../../../packages/prices-api/src/identity.rs"
  - "../../../packages/prices-api/src/assets/handlers.rs"
  - "../../../packages/prices-api/src/common/errors.rs"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Owns the §9 Tranche 2
      work bullet "Input validation: asset identifier format enforced, param
      ranges validated, 400 on invalid input" and overview §7's
      input-validation security bullet.
  - date: 2026-08-13
    status: active
    who: stkrolikiewicz
    note: >
      Activated. First of the M2 verification sequence agreed with okarcz:
      stkrolikiewicz takes [[0119]] -> [[0120]] -> [[0121]] in that order.
---

# Input-validation hardening

## Summary

Overview §9 Tranche 2 requires *"Input validation: asset identifier format
enforced, param ranges validated, 400 on invalid input"*, and §7 lists it as a
security control (*"Asset identifiers validated against known patterns"*).

Asset-identifier parsing is already solid — `AssetIdentifier::parse`
(`identity.rs`) runs **before** any DB call, so a malformed id 400s without
touching ClickHouse. The gap is everything else: query params, the batch body,
and the ranges §4 documents.

## Context

M1 shipped the route surface but only `GET /backfill/status` was verified for
the milestone. Tranche 2 AC 1 (*"All 7 endpoint groups return correct,
schema-valid responses"*) implicitly requires the **negative** path to be
correct too — a reviewer poking an endpoint with `limit=999999` or
`granularity=17m` must get a clean `400`, not a 500, a silent clamp, or a
full-table scan.

Unvalidated params on a ClickHouse-backed read path are also a cost concern:
an over-wide `start`/`end` or an unbounded `limit` turns into an unbounded
partition scan on a **shared** cluster (the same contention risk task 0047
tracks).

## Implementation

Audit and harden every input, per the §4 contracts:

**`GET /assets`** — `type` ∈ {`classic`,`soroban`,`all`}; `sort` ∈
{`price`,`volume_24h`,`change_24h`,`code`}; `order` ∈ {`asc`,`desc`};
`limit` 1–200 (§4.1 says default 50, max 200); `search` length-capped and
character-restricted; `cursor` must be well-formed Base64-JSON with the
expected keys — a corrupt cursor is a `400`, never a panic or a silent
first-page fallback.

**`GET /assets/{id}/ohlcv`** — `timeframe` ∈ {`1h`,`24h`,`7d`,`30d`,`1y`,`all`};
`granularity` ∈ {`1m`,`15m`,`1h`,`4h`,`1d`,`1w`,`1M`}; `base_currency` ∈
{`USD`,`XLM`}; `start`/`end` parseable ISO8601 with `start < end`; explicit
rejection of a `start`/`end` span whose point count would exceed
`OHLCV_MAX_POINTS` at the chosen granularity, rather than a silent truncation
that looks like missing data.

**`POST /prices/batch`** — array present, non-empty, **length-capped** (pick and
document a cap; unbounded batch is the cheapest DoS on this surface), each
element a valid identifier, and a clear error naming *which* element failed.

**`GET /oracles/{id}`** — identifier validation (already covered) plus any
range/limit params the handler accepts.

**Cross-cutting**

- One consistent error body across all handlers (`common/errors.rs` already has
  the shape) with a stable machine-readable code — reviewers and the [[0120]]
  conformance suite both key off it.
- Unknown query params: decide **reject vs ignore** and apply it uniformly.
  Recommended **ignore** (forward-compatible, and API Gateway cache keys make
  strict rejection user-hostile) — record the choice.
- Validate **before** the CH round-trip in every handler, matching the
  `AssetIdentifier` precedent.
- Ensure no validation failure path can 500. A `400` with a useful message is
  the contract.

## Acceptance Criteria

- [ ] Every param documented in §4.1–§4.5 has an enumerated or range-checked
      validator, with a table in the task mapping param → rule → error code
- [ ] Invalid input on every endpoint returns `400` with the standard error
      body; none returns 500 or a silently clamped result
- [ ] Malformed/truncated/foreign `cursor` values return `400`, not a silent
      first page
- [ ] `POST /prices/batch` enforces a documented maximum batch size
- [ ] An over-wide `start`/`end` range is rejected explicitly rather than
      truncated to `OHLCV_MAX_POINTS`
- [ ] Negative tests cover each rule and run in CI
- [ ] No validation path issues a ClickHouse query before rejecting
- [ ] OpenAPI spec reflects the enumerations and ranges (feeds [[0124]])

## Validation table (AC 1)

Every rule rejects **before** any ClickHouse round-trip; every rejection is a
`400` with the standard `ErrorEnvelope` and `Cache-Control: no-store`.

| Param | Rule | Error code |
|---|---|---|
| `{asset_identifier}` (path ×4 routes) | `native` \| `CODE:ISSUER` (code 1–12, issuer G-strkey) \| C-strkey; path-layer failures (bad %-encoding, invalid UTF-8) same code | `invalid_id` |
| `type` | `classic` \| `soroban` \| `all` (default `all`) | `invalid_query` |
| `sort` | `price` \| `volume_24h` \| `change_24h` \| `code` (default `volume_24h`) | `invalid_query` |
| `order` | `asc` \| `desc` (default `desc`) | `invalid_query` |
| `limit` | integer 1–200 (default 50); non-numeric/overflow rejected in extractor | `invalid_query` |
| `search` | 1–12 ASCII alphanumeric (SEP-11 alphanum12 prefix); empty = absent | `invalid_query` |
| `cursor` | ≤256 chars, Base64 of exactly `{v,id}` (`deny_unknown_fields`); `v` type-checked against active sort (finite number for numeric sorts, asset-code shape for `code`) | `invalid_query` |
| `timeframe` | `1h` \| `24h` \| `7d` \| `30d` \| `1y` \| `all` (default `24h`) | `invalid_query` |
| `granularity` | `1m` \| `15m` \| `1h` \| `4h` \| `1d` \| `1w` \| `1M` (default from timeframe) | `invalid_query` |
| `base_currency` | `USD` \| `XLM` (+ all-lowercase aliases; default `USD`) | `invalid_query` |
| `start` / `end` | epoch (s/ms) \| `YYYY-MM-DD` \| ISO-8601 datetime (`T`/space, optional offset); real calendar instant; ≤ 2100; `start < end`; `ceil(span/granularity) ≤ 5000` | `invalid_query` |
| batch body | valid JSON object with `assets`, ≤16 KB (`DefaultBodyLimit`) | `invalid_body` |
| batch `assets` | non-empty, ≤100 (`MAX_BATCH`), every element a valid identifier (error names the element) | `invalid_query` / `invalid_id` |

**Recorded policies**

- **Unknown query params: ignored** (forward-compatible; strict rejection would
  fight API Gateway cache-key declarations). Duplicate *known* keys are a 400
  (serde_urlencoded).
- **Case: exact documented tokens.** Sole exception: `base_currency` keeps
  all-lowercase `usd`/`xlm` aliases (historically case-insensitive); mixed case
  is now a 400. `granularity` is case-sensitive by necessity (`1m` ≠ `1M`).
- **Window semantics:** the timeframe window anchors to `end` when only `end`
  is given (`?end=…&timeframe=7d` = the 7d window ending there);
  `timeframe=all` starts at Stellar genesis (2015-09-30). The **validated
  epoch** is what binds into SQL (`toDateTime(?)`), so exactly one
  interpretation of the window exists.
- **Known limitation:** the cursor does not record which `sort`/`order`
  produced it — switching between two same-typed sorts mid-walk yields a wrong
  page (not a 500, not a wrong-type bind). Upgrade path: carry `{sort, order}`
  in the token.
- **CI:** negative tests are CH-less (`AppState::without_ch` panics on any CH
  access, so each clean 400 also proves the no-query-before-reject property)
  and run in the plain `cargo test --workspace` CI step. A ClickHouse service
  container in CI was deliberately not added — that is [[0120]]/[[0122]]
  territory. `prices-api` added to the CI clippy gate.
- `Cache-Control: no-store` on 400s protects client/CDN caching only; the API
  Gateway cache is config-side — gateway-layer verification belongs to
  [[0122]].

## Notes

- `?min_volume_usd=` from [[0118]] lands in this validation table too;
  whichever task ships second adds the row.
- API Gateway request validation (§2.1) can reject some malformed input at the
  edge. Prefer in-handler validation as the source of truth so the local test
  server and the deployed API behave identically — the gateway layer is
  defence in depth, not the contract.
