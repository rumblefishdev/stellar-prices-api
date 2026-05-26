---
id: '0024'
title: 'volume_quote_usd enrichment pass: join price_ohlcv with oracle_prices for non-USD-quoted pairs'
type: FEATURE
status: completed
related_adr: ['0003']
related_tasks: ['0022', '0012', '0023']
tags:
  [
    layer-indexing,
    priority-medium,
    effort-medium,
    ohlcv,
    enrichment,
    oracle,
    backfill,
    stream-2,
  ]
links:
  - '../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md'
  - '../archive/0023_RESEARCH_ohlcv-row-identity-base-vs-pair/notes/S-recommendation.md'
  - '../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md'
  - '../../../docs/database-schema/database-schema-overview.md'
  - '../../../docs/prices-api-general-overview.md'
history:
  - date: 2026-05-13
    status: backlog
    who: claude
    note: >
      Spawned from 0022 future-work item 2. Decode spec §5.3 + §6
      item 2 deferred this to a follow-up pass; the backfill
      writes volume_quote_usd = 0 until enrichment runs.
  - date: 2026-05-13
    status: active
    who: okarcz
    note: >
      Promoted to active. First-phase scope is design-only:
      produce a G-note specifying the trigger, SQL join,
      idempotency, and missing-oracle behaviour. Implementation
      (the actual Lambda or backfill extension) waits for task
      0012's CDK + Rust impl to land. ADR 0003's
      `quote_asset_id` PK column simplifies the SQL join.
  - date: 2026-05-13
    status: completed
    who: okarcz
    note: >
      Phase 1 design landed via PR #11. G-note specifies hourly
      cron Lambda + FOR UPDATE SKIP LOCKED + 5m forward-fill +
      CloudWatch telemetry. Phase 2 implementation spawned as
      task 0026 — blocked on task 0012's RDS+backfill landing.
---

# `volume_quote_usd` enrichment pass

## Summary

The SDEX backfill (task 0022 / 0012) writes `volume_base`
authoritatively but leaves `volume_quote_usd = 0` whenever the
quote-side reference price in USD is not directly available.
Implement an enrichment pass that fills in `volume_quote_usd` by
joining `price_ohlcv` against `oracle_prices` at minute
granularity.

## Context

For pairs where `quote ∈ {USDC, USDT}`, enrichment is approximately
the identity (USDC/USDT are pegged to ~$1). For pairs where
`quote = native XLM` (the bulk of SDEX volume — task 0022 profile
showed XLM-quoted pairs are common), enrichment requires the
`XLM/USD` oracle price at the minute bucket.

Schema is already in place:

- `price_ohlcv.volume_quote_usd` `NUMERIC(28,14) NOT NULL DEFAULT 0`
- `oracle_prices` table holds Reflector / Chainlink / RedStone
  / Band quotes, indexed by `(timestamp, asset_id, oracle_name)`.

The pass joins these, computes
`volume_quote_usd = price_ohlcv.volume_quote * oracle_price`,
and UPSERTs the result back to `price_ohlcv`.

## Two-phase scope

This task is split into two phases. **Phase 1 (this iteration)**
is design-only and lands a spec note; **Phase 2** is the actual
Lambda implementation and follows once task 0012 (RDS bootstrap +
SDEX backfill) lands.

### Phase 1 — Design spec (this iteration)

Produce [`notes/G-enrichment-pass-design.md`](./notes/G-enrichment-pass-design.md)
covering:

- Trigger architecture choice (cron Lambda vs end-of-backfill vs
  live-writer extension), with reasoning.
- SQL contract: the UPDATE statement joining `price_ohlcv` and
  `oracle_prices` on `quote_asset_id` (ADR 0003).
- Minute-bucket alignment between `price_ohlcv` (1m) and
  `oracle_prices` (5m forward-fill window).
- Idempotency contract (`WHERE volume_quote_usd = 0` gate).
- Missing-oracle behaviour, including exotic-quote (no direct
  USD oracle) edge cases.
- Concurrency contract under PG `FOR UPDATE SKIP LOCKED` against
  the backfill + live writers.
- Historical (post-backfill) one-shot enrichment vs the hourly
  rolling pass.
- CloudWatch telemetry.

### Phase 2 — Implementation (post-0012)

Spawned as a separate FEATURE task when task 0012 lands. Carries
forward the Phase 1 spec into a runnable Rust Lambda + CDK
deployment + integration test. The Phase 2 acceptance criteria
are listed in §7 of the design spec.

## Acceptance Criteria

Phase 1 (this iteration):

- [x] Design spec produced as a G-note covering the eight
      concerns enumerated above.
- [x] Trigger architecture chosen with reasoning (cron Lambda;
      see design §1).
- [x] SQL contract spec'd against ADR 0003's schema.
- [x] Missing-oracle behaviour documented including the
      no-direct-USD-oracle case (design §3 + §3.1).
- [x] Phase 2 acceptance criteria enumerated for the follow-up
      impl task (design §7).
- [x] Spawn Phase 2 follow-up task — spawned as task
      [0026](../../backlog/0026_FEATURE_volume-quote-usd-enrichment-impl.md),
      `blocked` on task 0012.

## Implementation Notes

Phase 1 landed in one commit on `feat/0024_volume-quote-usd-enrichment`
(PR [#11](https://github.com/rumblefishdev/stellar-prices-api/pull/11)
squash-merged into develop as `01ce5ab`):

| Commit    | Scope                                             |
| --------- | ------------------------------------------------- |
| `3312492` | Convert task to directory + Phase 1 design G-note |

Artifact: [`notes/G-enrichment-pass-design.md`](./notes/G-enrichment-pass-design.md)
(~317 lines) covering trigger, SQL, idempotency, concurrency,
missing-oracle, historical pass, telemetry. Mapped 1:1 to Phase 2
impl acceptance criteria in §7.

## Design Decisions

### From Plan

1. **Cron Lambda over backfill-extension / live-writer.** The
   README listed three trigger candidates; the spec recommends
   the cron Lambda for decoupling, idempotency, and consistency
   with the existing Oracle Fetcher Lambda's design pattern.

### Emerged

2. **Two-phase task split.** The original README treated 0024 as
   a single-phase FEATURE. Implementation can't land before task
   0012 (RDS bootstrap + SDEX backfill) exists. Split the work:
   design now (this iteration), impl later (spawned task 0026).
3. **`FOR UPDATE SKIP LOCKED` chosen over write-side coordination.**
   The concurrency analysis surfaced a "writer clobbers enrichment"
   race that loses ~1h of USD volume per affected row in the worst
   case. Adding write-side coordination would eliminate it but
   adds complexity to live writers and the backfill. The chosen
   trade-off accepts the bounded loss — next pass re-enriches.
4. **USDC/USDT treated as ordinary quote assets.** Considered
   hard-coding `volume_quote_usd = volume_quote` for stablecoin
   quotes; rejected to keep one code path and to capture depeg
   corrections from the oracle data.
5. **Exotic-quote two-hop enrichment punted to v2.** Quote assets
   with no direct USD oracle (e.g. AQUA) accumulate at
   `volume_quote_usd = 0` indefinitely in v1. The Current Price
   Updater silently excludes them (correct: we don't know what
   they're worth in USD). A v2 two-hop pass via XLM is named but
   not specced — separate task when prioritised.

## Issues Encountered

- **First activation commit captured only the rename**, not the
  frontmatter edits — `git mv` had staged the rename before my
  Edit landed in the index. Caught immediately; followup commit
  `f926874` captured the frontmatter changes.

## Future Work

Phase 2 implementation lives in spawned task
[0026](../../backlog/0026_FEATURE_volume-quote-usd-enrichment-impl.md)
(status `blocked` on task 0012). A potential Phase 3 task for
two-hop enrichment of exotic-quote pairs is mentioned in design
spec §3.1; not spawned yet — wait for operational evidence that
the missing-oracle rate is high enough to justify.
