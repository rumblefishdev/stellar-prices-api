---
id: "0306"
title: "The API reference shows type placeholders where the data goes, and parts of the published docs contradict production"
type: DOCS
status: active
related_adr: []
related_tasks: ["0305", "0233", "0195", "0124"]
tags: [layer-docs, layer-frontend, api, portal, priority-medium, effort-medium]
links:
  - "../../../packages/prices-api/src/openapi/descriptions.rs"
  - "../../../web/portal/src/docs/openapi.ts"
  - "../../../web/portal/src/quickstart/QuickStart.tsx"
history:
  - date: "2026-09-23"
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0305: the portal's docs held against production on
      2026-09-23. The two Quick Start statements that contradicted the page
      itself were fixed in 0305; everything else found is here.
  - date: "2026-09-23"
    status: active
    who: stkrolikiewicz
    note: "Activated; implementation on its own branch."
---

# The published docs show placeholders where the data goes, and parts of them contradict production

## Summary

On 2026-09-23 the portal's docs were held against production. The **structure
holds**: 40 live responses — every `/v1` route; native, credit and Soroban
assets; USD and XLM; 13 error bodies — validate against the OpenAPI schemas
with `additionalProperties: false`, nothing missing and nothing extra, and
`/api/openapi.json` is byte-identical to what the API serves. What drifts is
what a reader is **shown**: every example on `/api/docs` is a type
placeholder, several descriptions say something production does not do, and
the hand-written Quick Start and landing examples have smaller slips.

## Findings

### API reference (the OpenAPI document, `packages/prices-api`)

1. **No data in any example.** The document has no `example` except
   `ErrorEnvelope.code`, so `/api/docs` builds them from types:
   `price_usd: "string"`, `timestamp: "string"`, `method: "string"`,
   `sources: {}`. Production: `"0.22086251378147"`, `"2026-09-23T08:08:00Z"`,
   `"traded"`, `{"sdex": {"price": "…", "volume_24h": "…"}}`.
2. **Alphabetical key order.** utoipa is built without `preserve_order`
   (`Cargo.toml:42`); serde writes struct order. A candle example opens with
   `close` and has `timestamp` twelfth; the list reads `cursor, data,
   has_more` where the API sends `data, cursor, has_more`.
3. **`sources` is a bare `object`** — the entry shape `{price, volume_24h}`
   is in prose only.
4. **Omitted fields shown as present:** `ErrorEnvelope.details: null` (never
   serialised, as its own description says), `OhlcvResponse.backfill_note`
   (absent except `timeframe=all` mid-backfill), `Candle.source` / `quality`
   as strings (null outside the USDC self-series).
5. **The batch "Example body" `{"assets": ["string"]}` answers 400
   `invalid_id`** when copied and sent.
6. **401 `unauthorized` is documented on all seven `/v1` operations and
   cannot happen in production**: `API_KEYS` is unset
   (`infra/src/lib/stacks/compute-stack.ts:745`). A missing or wrong key is
   the gateway's 403 `{"message":"Forbidden"}`, documented with no body.
   `ErrorEnvelope` also calls itself "the body of every error response";
   403, 429 and an unknown path (`{"message":"Missing Authentication
   Token"}`) are not.
7. **`AssetDetail.code`** says `""` for `native` and `contract`
   (`openapi/descriptions.rs:148`); production answers `"XLM"` and the
   token's symbol (`"SolvBTC"`). The doc comment in `assets/dto.rs:198` is
   right.
8. **Soroban tokens and `search` / `sort=code`.** The `search` description
   implies a resolved symbol matches (`assets/handlers.rs:226`);
   `search=SolvBTC` → `[]`. `sort=code` orders Soroban tokens as `""`: on
   `asc` all eight come first, in no stated order. Only `assets/dto.rs:225`
   says so.
9. **SAC addresses.** `asset_kind`'s "a Soroban token or SAC" and the
   identifier's "`C…` address of a Soroban contract" invite one; USDC's
   (`CCW67…`) and XLM's (`CAS3J…`) answer 404 `unknown asset`.
10. **`home_domain` was `""` on all 395 assets fetched**, USDC, AQUA and
    yXLM included: populate it, or say it is not populated.
11. **`/health`'s 200 has no body schema**; production answers
    `{"status":"ok","stack":"prices-production"}`.
12. **Enum-valued fields typed `string`**: `OhlcvResponse.granularity` and
    `base_currency` (the `Granularity` / `BaseCurrency` components exist),
    `asset_type`, `asset_kind`, `method`, `status`.

### Quick Start and landing (`web/portal`)

13. The 400 hint says "CODE:ISSUER (uppercase code…)"; `yXLM:GARD…` → 200.
14. The backfill example's `realtime_tip_ledger: 63795749` is
    `sdex.target_ledger` — the frozen value the field's own description
    calls a past bug. Production read 64 573 020.
15. The landing's "GET /v1/assets/native/price — 200 OK"
    (`landing/Endpoints.tsx`) lacks the required `method`, with no `…`; the
    hero (`landing/Terminal.tsx`) lacks `price_xlm`, `updated_at` and
    `method` and reorders the rest.
16. The examples and the `sources` line name three venues; production has
    five (`phoenix`, `sushiswap` besides).
17. `/assets/{id}` lacks `contract` / `home_domain` and `/prices/batch`
    lacks `not_found`, both without `…`, so both read as complete bodies.
18. The SDK tabs are titled "fetch all prices"; the code fetches one.
19. The 400 row omits `invalid_body`.

### To explain — maybe not a docs fix

20. The Quick Start's venues sum exactly to `volume_24h_usd` ("as the real
    response's do", `QuickStart.tsx:371`). `native` on 2026-09-23: Σ
    `sources` 7.32 M against 9.23 M with all five venues present, where the
    documented reasons for Σ < total are an excluded venue or one with no
    priced close.

## Implementation Plan

1. `preserve_order` on utoipa; re-extract `web/portal/public/openapi.json`.
2. `#[schema(example = …)]` per field, taken from production responses. Not
   one `json!` per struct: `serde_json` has no `preserve_order` here, so an
   object example comes out alphabetical again. `sources` gets a map schema
   with its entry type; omitted-when-absent fields get no example.
3. Descriptions 6–12. For 401: take it out of the published document or
   mark it self-hosted only, and give 403/429 the gateway's body.
4. Portal copy 13–19.
5. 20: find where the difference comes from; then fix the example or the
   data.

## Acceptance Criteria

- [x] Every example on `/api/docs` is shaped like a real response: key
      order, decimal strings, ISO timestamps, enum tokens, the venue map
      (all seven operations' rendered examples match production's key sets
      and order, recursively)
- [x] The batch example body, copied and sent, answers 200 (`prices:
      [native]`, `not_found: [FOO:…]`, as the response example shows)
- [x] No published description contradicts production on items 6–12
      (12: `granularity` and `base_currency` reference their enums;
      `asset_type`, `asset_kind`, `method` and `status` stay strings with
      real tokens as examples)
- [x] Quick Start and landing: items 13–19 fixed
- [x] Item 20 explained, and the example agrees with the explanation:
      `volume_24h_usd` counts both sides of each trade, `sources` the base
      side only
- [x] A spec fails when a published example stops validating against its
      schema: `no-invalid-schema-examples` in `redocly.yaml` (checked
      against a mistyped example), and `every_property_has_an_example_or_a_reason`
      in `tests/openapi.rs`

## Shipping order

**API first, portal second.** `/api/docs` reads the live `/api-docs-json`,
and the portal now leaves out an optional field that has no example. Against
today's document — no examples at all — that would drop most optional fields
from the rendered examples, so the portal bundle must not ship before the
API's new document is served (Compute deploy, then `make -C infra
flush-production-cache`).

**Shipped 2026-09-24, in that order.** The API half went out with Adam's 0216
rollout (Compute, 11:30 CEST, `develop` with #343): the live `/api-docs-json`
is this task's document. The portal half at 11:42 — `make -C infra
sync-portal-explorer` from `develop` `d8f46f3b`, guild check ok, invalidation
`I3784KES66EBFQ18475YIQ45I2` completed. Measured right after: `index.html`
names the new bundle `index-i5pA_HeE.js`; `/api/`, `/api/docs`,
`/api/quickstart` and `/api/privacy-policy` answer 200; the bundled
`openapi.json` equals the live `/api-docs-json`. The Quick Start's 403 row
still explains `Missing Authentication Token`, which is true until [[0309]]
ships and rewrites it.
