---
id: "0048"
title: "Soroban events pricing decoder spec — what to extract from soroban_events for price_ohlcv, and how the Lambda implements it"
type: RESEARCH
status: active
related_adr: ["0001", "0003", "0004", "0007"]
related_tasks: ["0038", "0039", "0040", "0045", "0046", "0047"]
tags: [layer-research, priority-high, effort-medium, stream-1, lambda, ingestion, clickhouse, decoder, pricing]
links:
  - "../../../3-wiki/project/soroban-events-schema.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../blocked/0038_FEATURE_prices-ledger-processor-lambda.md"
  - "../../blocked/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/README.md"
  - "../../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-19
    status: active
    who: okarcz
    note: >
      Spawned to produce the authoritative decoder + Lambda spec
      that the 0038 rewrite will implement against. Driven by the
      need to enumerate exactly which soroban_events shapes feed
      pricing (and which ingestion paths are NOT covered by
      soroban_events — notably classic SDEX). Empirical findings
      backed by a 10k uniform sample of the local backfill CH
      (ledgers 62019999–62079982; 47.5M total events). Output is
      the G-note `notes/G-soroban-events-pricing-decoder.md`.
---

# Soroban events pricing decoder spec

## Summary

Produce a single authoritative document that (a) enumerates exactly
which `soroban_events` rows carry signal for `price_ohlcv`, with
per-shape decoder rules verified against real data, (b) calls out
which pricing-relevant flows live OUTSIDE `soroban_events` (classic
SDEX, classic LP), and (c) lays out the end-to-end Lambda
implementation that ingests both paths into `prices.*` on Hetzner
ClickHouse per ADR 0007.

The deliverable is **`notes/G-soroban-events-pricing-decoder.md`** —
the spec the 0038 rewrite will implement against.

## Context

- ADR 0007 (proposed) pins the live data sink to BE's Hetzner CH;
  blocked tasks 0038/0039/0040 will be rewritten against it once
  task 0045 (cross-team bundle) closes and task 0047 verifies
  cross-tenant throughput.
- The wiki schema doc at `lore/3-wiki/project/soroban-events-schema.md`
  already documents the per-shape payloads exhaustively (captured
  from ledgers 62078346–62079999). What is missing is the
  pricing-specific mapping: which signatures feed which fields of
  `price_ohlcv`, what assumptions the decoder makes, how it joins
  AMM events into trade ticks, and how the Lambda binary is wired
  end-to-end.
- The 0038 README explicitly defers implementation until ADR 0007
  is accepted. This task produces the spec; 0038's rewrite then
  references it.

## Implementation Plan

### Phase 1 — Empirical inventory

- Sample 10,000 rows uniformly across the full local backfill CH
  (47.5M events, ledgers 62019999–62079982) via
  `WHERE cityHash64(transaction_id, event_index, ledger_sequence) % N = K`.
- Cross-tab signature × contract emitter counts, focusing on
  `trade`, `swap`, `update_reserves`, `REFLECTOR`, `REDSTONE`.
- Verify that classic SDEX (op types 2/3/12/13) is NOT represented
  in `soroban_events` and identify the alternative source
  (`LedgerCloseMeta.OperationResult.ClaimAtom`).

### Phase 2 — Decoder spec

Per signature, document:

- Topic / data shape (already in the wiki doc — cross-link, don't
  duplicate).
- Pricing-relevant fields and their units.
- Asset identity resolution (Int64 `contract_id` → strkey via
  `soroban_contracts`; SAC vs. custom token; `code:issuer` parse).
- Cross-event correlation rules (`trade` ↔ `update_reserves` ↔
  `transfer` on the same tx).
- Per-protocol notes: Soroswap router vs. pair, Phoenix, Aquarius
  (where applicable), Uniswap-V3-style CLMM.
- Decimal handling: REFLECTOR's 14-decimal scaling; native i128 in
  token-native decimals.

### Phase 3 — Lambda E2E spec (ADR 0007 path)

- S3 PutObject on BE's `stellar-ledger-data` → SNS topic (per
  ADR 0007 §3.2 / Cluster A buy-in in task 0045).
- Subscribed `prices-ledger-processor` Rust Lambda, no VPC.
- GET object → zstd-decompress → parse `LedgerCloseMeta`.
- Dispatch kernel runs BOTH:
  1. SDEX extractor over `OperationResult.ClaimAtom` arrays.
  2. Soroban event extractor over `SorobanTransactionMeta.events`.
- Bucket into 1-min OHLCV, INSERT into
  `prices.price_ohlcv_1m(timestamp, asset_id, quote_asset_id,
  granularity, source, …)` over HTTPS-mTLS to Caddy:443.
- Materialised view chain rolls 1m → 15m → … → 1M downstream.
- Idempotency via `ReplacingMergeTree(version)` per ADR 0007.
- Observability: lag metric, per-source trade counts, decode error
  per-tx counters.

### Phase 4 — Open questions

- Which `swap` emitters beyond Soroswap router are pair-level vs.
  router-level? (Catalog by contract address in the G-note.)
- How does the Aquarius AMM emit trade-equivalent events? Not
  observed in the backfill window — flag as a follow-up sample
  needed.
- Reflector / Redstone: are oracle updates used as **price
  inputs** for quote conversion, or **separate sources** in
  `price_ohlcv.source`? Resolve via ADR 0004 lens.

## Acceptance Criteria

- [ ] `notes/G-soroban-events-pricing-decoder.md` exists and covers:
  - [ ] Empirical 10k-sample signature distribution.
  - [ ] Per-signature decoder rules with field-level mapping to
        `price_ohlcv` columns.
  - [ ] Cross-event correlation (`trade` + `update_reserves` +
        `transfer` join recipe).
  - [ ] Explicit "NOT in soroban_events" section for classic SDEX
        and classic LP, with pointers to `operations_appearances`
        / `LedgerCloseMeta.OperationResult.ClaimAtom`.
  - [ ] Per-protocol contract catalog: Soroswap router, CLMM,
        Phoenix-style.
  - [ ] Lambda E2E spec aligned with ADR 0007 (mTLS, no VPC, CH
        sink, SNS fan-out).
  - [ ] Edge-cases section: NULL signatures, three swap shapes,
        REDSTONE bytes-decode, ReplacingMergeTree FINAL.
- [ ] G-note cross-linked from `0038` and `ADR 0007`.
- [ ] Index regenerated.

## Out of scope

- Implementation. 0038's rewrite will consume this spec.
- SDEX classic decoder implementation details — the spec calls
  out the source path (`OperationResult.ClaimAtom`) but a full
  classic-Stellar decoder ADR can live in a follow-up.
- Aquarius AMM coverage if the contracts didn't appear in the
  backfill window — surface as a follow-up sample task.

## Notes

- Spec is grounded in the local backfill CH at
  `/home/oski/Projects/stellar/soroban-block-explorer` (docker
  compose; ledgers 62019999–62079982).
- The companion wiki doc
  (`lore/3-wiki/project/soroban-events-schema.md`) is the
  payload reference; this spec is the **pricing application** of
  it.
