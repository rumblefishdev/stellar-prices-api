---
id: "0023"
title: "OHLCV row identity: decide base-only PK vs (base, quote) PK before SDEX backfill implementation"
type: RESEARCH
status: completed
related_adr: ["0003"]
related_tasks: ["0022", "0012", "0024", "0025"]
tags: [priority-high, effort-small, schema, ohlcv, sdex, backfill, blocking]
links:
  - "../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../../../docs/database-schema/database-schema-overview.md"
  - "../../../docs/prices-api-general-overview.md"
  - "./notes/Q-decision-space.md"
  - "./notes/S-recommendation.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
history:
  - date: 2026-05-13
    status: backlog
    who: claude
    note: >
      Spawned from 0022 future-work item 1. Decode spec §2.3 + §6
      surfaced this as a load-bearing schema gap before task 0012's
      Rust implementation can start.
  - date: 2026-05-13
    status: active
    who: okarcz
    note: >
      Promoted to active immediately after 0022 archival. Blocking
      task 0012 — needs an ADR-level decision on the OHLCV row PK
      shape before the SDEX backfill Rust impl can be written.
  - date: 2026-05-13
    status: completed
    who: okarcz
    note: >
      ADR 0003 accepted (Option A: add `quote_asset_id` to the
      `price_ohlcv` PK). Notes: Q-decision-space (option framing),
      S-recommendation (analysis + recommendation). PR #10 merged
      to develop. Tasks 0024 / 0025 unblocked; task 0012 picks up
      the DDL in its pre-backfill schema migration.
---

# OHLCV row identity: decide base-only PK vs (base, quote) PK

## Summary

`price_ohlcv` today keys on `(timestamp, asset_id, granularity)`
where `asset_id` is the **base asset only**. SDEX trades commonly
have one base trading against multiple quotes in the same minute
(e.g. `USDC/XLM` and `USDC/USDT` both produce candles with
`asset_id = USDC.id`). They collide on the PK.

Pick one of three resolutions and write the schema-change ADR
before task 0012 starts.

## Context

Task 0022's decode-and-bucket spec §2.3 and §6 item 1 flagged
this as a real ambiguity. Three options:

A. **Add `quote_asset_id` to the PK.**
   `PRIMARY KEY (timestamp, asset_id, quote_asset_id, granularity)`.
   Simplest; preserves existing schema shape; doubles the row count
   for assets that trade against many quotes (still bounded — a
   handful of quotes per asset in practice).

B. **Introduce `asset_pair_id` as a surrogate.**
   New `asset_pairs` table; `price_ohlcv.asset_pair_id` replaces
   `asset_id`. Cleaner relational model; bigger migration; needs
   the `asset_pairs` table to exist and be populated.

C. **Status quo: aggregate across quotes per asset.**
   Drop the per-quote distinction; one "USDC candle" per minute
   that merges USDC/XLM + USDC/USDT + … trades. Loses
   information; defensible only if API consumers never need
   per-quote granularity (they probably do, for USDC vs USDT
   stablecoin depeg detection).

Option A is the default working assumption from 0022's spec.
This task confirms / refutes that and produces the ADR.

## Implementation

- Audit API endpoints for any consumers that need per-quote OHLCV.
- Decide A vs B vs C with the data team.
- Write ADR (`lore/2-adrs/000N_ohlcv-row-identity.md`).
- Open follow-up issue for the schema migration (or fold into task
  0012 if Option A).

## Acceptance Criteria

- [x] ADR landed in `lore/2-adrs/` documenting the choice and
      reasoning. ADR 0003 (accepted) records Option A: add
      `quote_asset_id` to the `price_ohlcv` PK.
- [ ] Task 0012's spec is updated to reference the chosen row
      identity. (Deferred to task 0012 itself when it is promoted
      to active; the ADR is the source of truth in the meantime.)
- [x] If the choice is Option B, an `asset_pairs` table DDL is
      drafted. N/A — Option A chosen.

## Implementation Notes

Landed across 2 commits on `research/0023_ohlcv-row-identity-base-vs-pair`
(PR [#10](https://github.com/rumblefishdev/stellar-prices-api/pull/10),
squash-merged into develop as `8e57003`):

| Commit    | Scope                                                        |
| --------- | ------------------------------------------------------------ |
| `745a626` | Convert task file → directory (`notes/`)                     |
| `ff23481` | Research (Q-note + S-note) + ADR 0003 draft                  |

Followed by completion + archive (this commit on develop).

Artifacts produced:

- `notes/Q-decision-space.md` (~75 lines) — frames the four
  options (A/B/C/D), surfaces the API contract constraint that
  rules out option C.
- `notes/S-recommendation.md` (~228 lines) — option matrix,
  why-A-over-B / why-not-C / why-not-D, DDL, projection-at-read
  sketch, impact on tasks 0012 / 0022 / 0024 / 0025.
- `lore/2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md`
  (~185 lines) — accepted ADR.

## Design Decisions

### From Plan

1. **Option A (add `quote_asset_id` to PK)** — the README laid out
   A/B/C as candidates; A is the recommended pick. Reasoning lives
   in S-recommendation §"Why A over B" and §"Why not C".

### Emerged

2. **Migration is greenfield — no row backfill.** The schema-bootstrap
   migration in task 0012 lands the DDL before any `price_ohlcv`
   rows exist, so the change reduces to one `ALTER` per column +
   PK + indexes. No data-migration plan needed.
3. **`asset_pairs` table not introduced.** Option B's surrogate
   was rejected as speculative; introduce it just-in-time when a
   second consumer needs it. Documented in S-recommendation
   §"Why A over B".
4. **Naming infelicity in API noted but not fixed.** The
   `?base_currency=USD|XLM` query param on `/assets/{id}/ohlcv`
   is mislabelled (it's the quote currency). Rename deferred to
   an API v2 — not blocking the ADR.
5. **Task 0022 spec edit deferred to task 0012's worklog.** The
   archived decode-and-bucket spec needs a one-line correction
   (in-memory accumulator key + UPSERT conflict target gain
   `quote_asset_id`). Edit lands when 0012 implements, not now —
   preserves the historical record of 0022 as written.

## Issues Encountered

None substantive. The research was straightforward once the API
contract was surveyed and the four options laid out side-by-side.

## Future Work

None spawned. The follow-ups already exist:

- Task **0012** (backlog) — implements the ADR's DDL as part of
  its pre-backfill schema migration.
- Task **0024** (backlog) — `volume_quote_usd` enrichment pass,
  now unblocked: joins `oracle_prices` on `price_ohlcv.quote_asset_id`.
- Task **0025** (backlog) — live multi-source merge contract,
  now unblocked: collides on the new PK shape per source.
