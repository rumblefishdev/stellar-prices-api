---
id: "0120"
title: "Endpoint conformance — all 7 route groups return correct, schema-valid responses for 20 major assets"
type: TEST
status: active
related_adr: ["0008"]
related_tasks: ["0072", "0118", "0119", "0124", "0128"]
tags: [layer-backend, priority-high, effort-medium, milestone-M2, api, testing, verification, acceptance]
milestone: 2
links:
  - "../../../packages/prices-api/src/lib.rs"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Owns Tranche 2
      acceptance criterion 1 — the only AC that covers the full public API
      surface, which M1 deployed but never verified beyond
      `GET /backfill/status`.
  - date: 2026-08-18
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active — starting the endpoint conformance pass.
---

# Endpoint conformance for 20 major assets

## Summary

Tranche 2 AC 1: *"All 7 endpoint groups return correct, schema-valid responses
for at least 20 major assets."*

M1 deployed the whole route surface but verified only `GET /backfill/status`
(stated plainly in `milestone-1-evidence.md` Table 4). This task is the
verification pass that turns "routed" into "correct".

## Context

Two distinct claims hide inside AC 1, and they need separate evidence:

- **Schema-valid** — the response matches the documented shape in §4.1–§4.5
  and the generated OpenAPI spec. Mechanically checkable.
- **Correct** — the numbers mean what they claim. Not checkable against the
  spec; needs either an independent source or an internal cross-check.

This task owns both, with the numeric-correctness half deliberately narrow:
deep VWAP reconciliation is [[0123]] and historical spot-checking is [[0127]].
Here the bar is *"no field is a stub, a sentinel, or obviously wrong"*.

**Dependency:** several §4 response fields are still stubs until [[0072]] lands
(`price_xlm`, `change_24h_pct`, `change_7d_pct`, `sources` are all at their
table DEFAULTs). Running this suite before 0072 will correctly fail. Sequence
after 0072 and [[0119]].

## Implementation

- **Fix the asset list.** Pick 20 named assets and record them in the task, not
  in a shell variable — the same list must be reusable by [[0121]], [[0123]],
  [[0127]] and the [[0128]] evidence package. Seed from §9's Tranche 1 list
  (XLM, USDC, EURC, AQUA, BTC, ETH) and extend with the highest-volume assets
  the store actually holds. Include at least one asset per identifier form:
  `native`, `CODE:ISSUER`, and a `C…` contract address.
- **Exercise all 7 groups** per asset: `GET /assets`, `GET /assets/{id}`,
  `GET /assets/{id}/price`, `GET /assets/{id}/ohlcv`, `POST /prices/batch`,
  `GET /oracles/{id}`, `GET /backfill/status`.
- **Schema-validate** each response against the generated OpenAPI spec rather
  than hand-written assertions, so the spec and the tests cannot drift apart.
- **Sanity assertions** beyond the schema:
  - no documented field is absent, and no field is left at its zero/empty
    sentinel for an asset that should have a value
  - numeric strings parse; `Decimal(38,14)` precision survives the JSON
    round-trip (values are serialised as strings by design — §3.3)
  - OHLCV invariants hold: `low ≤ open,close ≤ high`, timestamps strictly
    increasing and aligned to the requested granularity, no duplicate buckets
  - `GET /assets` pagination: walking the cursor to exhaustion yields every
    asset exactly once, and `has_more` is accurate at the boundary (extends
    the 0074 250-row pagination test to the M2 asset set)
  - `POST /prices/batch` returns the same numbers as the per-asset `/price`
    calls for the same assets in the same window
- **Run against production** over the real gateway with an API key — this is an
  acceptance check, not a unit test. Keep it a scripted, re-runnable artifact so
  [[0128]] can cite a fresh run.

## Acceptance Criteria

- [x] The 20-asset list is fixed, documented in-task, and covers all three
      identifier forms
- [x] All 7 route groups exercised for every asset; every response validates
      against the OpenAPI spec (0 schema failures in the 2026-08-19 run)
- [ ] No documented response field is a stub/sentinel for a liquid asset
      (**failing on production** — deferred to [[0207]], [[0208]], [[0209]];
      re-run must go green after those land)
- [x] OHLCV invariants asserted (OHLC ordering, bucket alignment, no dupes —
      all pass wherever data exists)
- [x] Cursor pagination on `GET /assets` proven exhaustive and duplicate-free
      (20 pages, 3880 distinct assets, no dup identity triples)
- [x] `POST /prices/batch` agrees with per-asset `/price` for the same assets
      (all 19 priced assets equal at matching timestamps)
- [x] Suite is re-runnable (`npm run conformance:0120`) and its JSON report is
      citable evidence for [[0128]]
- [x] Any defect found is fixed or spawned as its own task — spawned
      [[0207]]–[[0211]]

## Fixed asset list (AC 1)

Machine-readable copy: `tools/scripts/conformance-assets.json` (shared with
[[0121]], [[0123]], [[0127]], [[0128]]). Derived 2026-08-19 from production:
`GET /v1/assets?limit=200` (default sort `volume_24h desc`) + `search=` probes
for the §9 majors, over the real gateway with the team API key.

| # | Asset | Identifier (form) |
|---|-------|-------------------|
| 1 | XLM | `native` (native) |
| 2 | USDC | `USDC:GA5ZSEJY…K4KZVN` (code-issuer, canonical Circle) |
| 3 | EURC | `EURC:GDHU6WRG…ITNPP2` (code-issuer, canonical Circle) |
| 4 | AQUA | `AQUA:GBNZILST…M67AQUA` (code-issuer) |
| 5 | BTC | `BTC:GDPJALI4…5O2MZM` (code-issuer, top-volume BTC) |
| 6 | ETH | `ETH:GBFXOHVA…CMGSOCC` (code-issuer, top-volume ETH) |
| 7 | (soroban) | `CBIJBDNZ…5FM6VN` (contract; top-volume soroban asset) |
| 8–20 | USDCAllow, AUD, sUSD, yUSDC, XRP, SHX, SCOP, RON, BOL, EQL, yXLM, PYUSD, VELO | code-issuer, filled by store volume rank |

Full 56-char identifiers live in the JSON; the table above is for humans.
Selection rules applied:

- §9 six seeded first; BTC/ETH pinned to the highest-volume issuer the store
  holds (ticker codes are not unique — the M1 evidence doc's canonical-pinning
  caveat applies).
- Fill by store volume rank, skipping the `*BANK*` spam family, secondary
  wrappers of an already-listed code, and obscure USD clones.
- **USDT excluded deliberately** — known bug [[0172]]; including it fails the
  suite on an already-tracked defect.
- All three identifier forms covered: `native` (#1), `CODE:ISSUER` (#2–6,
  8–20), contract `C…` (#7). A classic asset **cannot** be addressed by its SAC
  contract address (probed: AQUA via SAC → 404 `unknown asset`; the store only
  carries `contract_address` for soroban rows), so the contract slot must be a
  soroban-native asset.

## Suite and run results

Suite: `tools/scripts/conformance-0120.mjs` (`npm run conformance:0120`;
needs `API_KEY`/`BASE_URL`, repo-convention `.env.local`). Validates every
response — errors included — against the **live** spec from `/api-docs-json`
(ajv, JSON Schema 2020-12), then layers the sanity assertions. Paced ≤1 rps
for the free usage plan; ~2.5 min per run; report written as
`conformance-0120-report-<ts>.json` (gitignored, regenerable).

**Run 2026-08-19 08:16 UTC: 752 pass, 55 fail, 0 skip.** Zero schema
failures — the entire failure surface is the correctness layer, and every
failure maps to a spawned defect task:

| Failing check | Count | Task |
|---|---|---|
| `price_usd` zero sentinel | 18 | [[0207]] |
| `vwap_24h` zero + `sources` empty (fresh tips, volume > 0) | 24 | [[0209]] |
| OHLCV windows empty (USDC; CBIJ, AUD, RON, BOL, EQL) | 12 | [[0208]] / [[0209]] |
| Canonical USDC `/price` → 404 | 1 | [[0208]] |

Findings confirmed by the run, beyond the failures: soroban rows carry empty
`asset_code` ([[0210]]); OHLCV `start`/`end` are both **inclusive** but
undocumented ([[0211]] — the suite encodes the measured behavior). Passing
highlights: pagination walk over 20 pages / 3880 assets with zero duplicate
identity triples and accurate `has_more`; batch equal to singles at matching
timestamps for all 19 priced assets; `Decimal(38,14)` strings parse
everywhere; all OHLCV invariants hold wherever data exists.

The stub/sentinel AC stays open until [[0207]]–[[0209]] land; the suite is
the acceptance gate — re-run it after each fix and cite the green report in
[[0128]].

## Notes

- Deliberately **not** the Tranche 3 "integration test suite runs in CI"
  deliverable. This is an acceptance pass against the deployed API; wiring an
  equivalent suite into GitHub Actions is M3.
- Expect `sources` to name **Aquarius** here — that is where §9's "Aquarius
  appearing as a named source in VWAP" bullet is actually observed. If it does
  not appear, the cause is [[0072]] (column not written) or [[0080]]
  (concentrated pools not extracted), not this task.
