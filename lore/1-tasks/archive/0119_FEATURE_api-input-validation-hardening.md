---
id: "0119"
title: "Input-validation hardening — every path, query and body param rejected with 400 on invalid input"
type: FEATURE
status: completed
related_adr: ["0006", "0008"]
related_tasks: ["0040", "0118", "0120", "0206"]
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
  - date: 2026-08-18
    status: completed
    who: stkrolikiewicz
    note: >
      COMPLETED via PR #217 (squash-merged to develop, approved by okarcz).
      All 8 ACs met. Four implementation phases + two hardening rounds: an
      8-angle /code-review (10 verified findings, 9 fixed — incl. a
      client-reachable 500 via the cursor and a fencepost in the window cap)
      and okarcz's 7-point review (all applied — length-only search/cursor
      rules matching the real data model, URL-safe cursors, '+'-decoding
      recovery in dates, span-derived auto-granularity that also defuses the
      2029 timeframe=all cliff). ~50 CH-less negative tests in CI + 21
      live-CH integration tests; prices-api added to the clippy gate.
      Spawned [[0206]] for the deferred follow-ups.
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

- [x] Every param documented in §4.1–§4.5 has an enumerated or range-checked
      validator, with a table in the task mapping param → rule → error code
- [x] Invalid input on every endpoint returns `400` with the standard error
      body; none returns 500 or a silently clamped result
- [x] Malformed/truncated/foreign `cursor` values return `400`, not a silent
      first page
- [x] `POST /prices/batch` enforces a documented maximum batch size (100
      elements + 16 KB `DefaultBodyLimit`)
- [x] An over-wide `start`/`end` range is rejected explicitly rather than
      truncated to `OHLCV_MAX_POINTS`
- [x] Negative tests cover each rule and run in CI (CH-less, plain
      `cargo test --workspace`)
- [x] No validation path issues a ClickHouse query before rejecting (proved
      structurally: the test state panics on CH access)
- [x] OpenAPI spec reflects the enumerations and ranges (feeds [[0124]];
      contract-tested in `tests/openapi.rs`)

## Implementation Notes

Shipped as PR #217 (8 commits, squash-merged 2026-08-18). Layout:

- `src/common/extract.rs` (new) — `ValidatedQuery`/`ValidatedJson`/
  `ValidatedPath` wrappers routing every axum extractor rejection into the
  `ErrorEnvelope` (new `invalid_body` code; `Cache-Control: no-store` on 400s).
- `src/assets/queries_ch.rs` — the six param enums derive
  `Deserialize + ToSchema` with explicit renames; the six stringly `parse_*`
  fns and the `now() - INTERVAL` SQL path are deleted.
  `Granularity::finest_for_span` picks the auto-granularity for explicit
  windows and `timeframe=all`.
- `src/assets/handlers.rs` — window rule (`start ≤ end`,
  `ceil(span/granularity)+1 ≤ 5000`, clamped derived start, future-start
  message), `parse_time` (chrono; epoch/date/datetime incl. recovered
  `+`-offsets), length-only `search` cap.
- `src/common/cursor.rs` — `deny_unknown_fields`, 256-char token cap,
  URL-safe encoding (STANDARD still decoded), `valid_for` type-check against
  the active sort.
- `src/batch/` — 16 KB `DefaultBodyLimit`; schema bounds asserted against
  `MAX_BATCH`.
- Tests: `tests/{common/,batch,list,ohlcv}.rs` (~50 CH-less negative tests) +
  OpenAPI contract tests; `.github/workflows/ci.yml` clippy line gains
  `prices-api`.

## Design Decisions

### From Plan

1. **Wrapper extractors over axum-extra/WithRejection** — zero new deps; the
   old 415/422 plain-text rejections had no machine-readable contract worth
   preserving, so uniform 400 + envelope.
2. **Typed serde enums as the single source** for validation and the OpenAPI
   document (AC 1 + AC 8 from one mechanism).
3. **CH-less negative tests as structural proof of AC 7** —
   `AppState::without_ch` panics on any CH access, so every clean 400 also
   proves no query preceded the rejection.
4. **Validated epoch binds into SQL** (`toDateTime(?)`), not the raw string —
   exactly one interpretation of the window.

### Emerged

5. **Length-only rules wherever a value is compared against `asset_code`**
   (search, code-sort cursor payloads; shared `MAX_STRING_PAYLOAD_LEN = 64`):
   the DB legitimately holds empty and lossy-decoded codes, so any charset
   rule 400s values the API itself serves. Surfaced by okarcz's review after
   the initial charset approach shipped for `search`.
6. **URL-safe cursors with STANDARD-decode fallback** — `next_cursor` echoed
   verbatim into a query string must survive percent-decoding; in-flight
   STANDARD tokens keep working across the deploy.
7. **Query-string `+` recovery in `parse_time`** — a literal `+` offset
   percent-decodes to a space; a trailing ` HH:MM`/` HHMM` after a time is
   reinterpreted as the lost `+`. The space *separator* is fixed positionally
   (byte 10), not first-match.
8. **Span-derived auto-granularity** for explicit windows and `timeframe=all`
   (finest fitting 5000 points) — answers `?start=`-only requests at a
   granularity the caller can use, and self-coarsens `all` (1d today, 1w
   post-2029) instead of the recorded cliff.
9. **Anchor-to-end window semantics** and **UTC pinning of naive datetimes** —
   consumer-visible flips, recorded in Notes.
10. **Batch body cap 16 KB** — sized from `MAX_BATCH` × identifier length;
    stops multi-MB bodies from being parsed just to fail the element cap.

## Issues Encountered

- **utoipa-axum does not auto-register params-tuple `$ref` schemas** — the
  served document carried dangling refs until the six enums were listed in
  `ApiDoc::components`; caught by the new enum-publication contract test.
- **`+` percent-decoding** made offset-carrying dates and STANDARD-base64
  cursors unreachable over real HTTP while unit tests passed — both found in
  review, both now covered by tests written against the post-decode forms.
- **serde_urlencoded duplicate known keys** error out (→ 400 envelope), which
  became the recorded duplicate-param policy for free.
- Local `docker compose` ClickHouse (prod-pinned 26.3.10.60) smoke caught the
  one positive-path regression (see modified tests below).

**Broken/modified tests:** `tests/ohlcv_it.rs::ohlcv_merges_sources_and_notes_backfill`
leaned on the old silent-truncation semantics (`timeframe=all&granularity=1h`);
narrowed with explicit `start`/`end` around the seeded candles — intentional,
not a regression. `tests/list.rs` search/cursor negative tests were rewritten
twice as the rules evolved (charset → length-only; STANDARD → URL-safe).

## Future Work

Spawned [[0206]]: cursor `{sort, order}` binding (closes the recorded
same-typed-sort-swap limitation), `ValidatedPath<AssetIdentifier>` (deletes
four identical parse blocks; makes `invalid_id` correct by construction),
shared negative-test assert helper, `parse_time` rejection-block dedup
(natural fold-in when [[0118]] adds `min_volume_usd`).

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
| `search` | ≤64 bytes, length-only (stored codes include lossy-decoded on-chain bytes — a charset rule would make listed assets unsearchable; PR #217); empty = absent | `invalid_query` |
| `cursor` | ≤256 chars, Base64 (issued URL-safe/no-pad; STANDARD still decoded for in-flight tokens) of exactly `{v,id}` (`deny_unknown_fields`); `v` type-checked against active sort (finite number for numeric sorts, ≤64 bytes for `code` — same length-only rule as `search`) | `invalid_query` |
| `timeframe` | `1h` \| `24h` \| `7d` \| `30d` \| `1y` \| `all` (default `24h`) | `invalid_query` |
| `granularity` | `1m` \| `15m` \| `1h` \| `4h` \| `1d` \| `1w` \| `1M`; omitted ⇒ per-timeframe default, except explicit `start`/`end` windows and `timeframe=all` ⇒ finest granularity fitting the 5000-point cap (PR #217) | `invalid_query` |
| `base_currency` | `USD` \| `XLM` (+ all-lowercase aliases; default `USD`) | `invalid_query` |
| `start` / `end` | epoch (s/ms) \| `YYYY-MM-DD` \| ISO-8601 datetime (`T`/space, optional offset; a `+` offset percent-decoded to a space in transit is recovered); real calendar instant; ≤ 2100; `start ≤ end` (equal = one inclusive bucket); `ceil(span/granularity)+1 ≤ 5000` | `invalid_query` |
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

- **Consumer-visible change:** ANY window × granularity over 5000 buckets is
  now a 400 where it used to silently return the newest 5000. That is not just
  `timeframe=all` with `1m`–`4h`: it also flips `7d×1m` (10,080), `30d×1m`
  (43,200), `1y×1m`, `1y×15m`, and `1y×1h` (8,760). `all` remains legal with
  `1d`/`1w`/`1M` or with explicit `start`/`end`. The pre-existing `ohlcv_it`
  merge test leaned on the old semantics and was narrowed with explicit
  bounds — intentional, not a regression.
- **Two more recorded semantic shifts** (review findings, accepted): (i) naive
  `start`/`end` values are now pinned to **UTC** in-handler instead of being
  interpreted in the ClickHouse server timezone by `parseDateTimeBestEffort` —
  deterministic, and a no-op while CH runs UTC; (ii) a far-future `?end` used
  to behave like "no upper bound" (window `[now-tf, end]`) and now anchors the
  window to `end`, so it returns an empty 200 — consistent with the anchoring
  policy above, recorded here because it flips an observable behavior.
- **Time bomb defused (PR #217 review):** bare `timeframe=all` previously
  defaulted to `1d` and would have started 400ing around **2029-06** (genesis →
  now crossing 5000 daily buckets). The default for `all` — and for any
  explicit `start`/`end` window with no `granularity` — is now the finest
  granularity that fits the cap, so the response self-coarsens (today: `1d`,
  post-2029: `1w`) instead of hard-failing. An explicitly requested granularity
  still rejects explicitly.
- `?min_volume_usd=` from [[0118]] lands in this validation table too;
  whichever task ships second adds the row.
- API Gateway request validation (§2.1) can reject some malformed input at the
  edge. Prefer in-handler validation as the source of truth so the local test
  server and the deployed API behave identically — the gateway layer is
  defence in depth, not the contract.
