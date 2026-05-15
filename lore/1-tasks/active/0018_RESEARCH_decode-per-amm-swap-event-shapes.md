---
id: "0018"
title: "Sample-decode per-AMM swap event shapes (Soroswap, Aquarius, Phoenix)"
type: RESEARCH
status: active
related_adr: ["0001"]
related_tasks: ["0015", "0017"]
tags: [priority-medium, effort-small, research, soroban, amm, schema, xdr]
links:
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/G-ch-tables-for-price-calculation.md"
history:
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0015 G-note open spike. Pin down the per-AMM
      topic + data ScVal shapes before the Stream 1 consumer's
      swap-extraction logic is written. The stellar-xdr parser crate
      (Q4 of 0015) handles bytes->ScVal; this task pins the
      semantic conventions (which symbol = swap, what's in data, in
      which order) that each AMM uses.
  - date: 2026-05-15
    status: active
    who: okarcz
    note: >
      Promoted to active. Cross-referenced BE ADR 0044 (ClickHouse
      pilot) — the local CH `soroban_events` table from BE task 0206
      stores full-content events with `topics_xdr` + `data_xdr` as
      ZSTD(3)-coded String columns and `signature` as
      `LowCardinality(Nullable(String))`, partitioned by
      `intDiv(ledger_sequence, 500000)`. Sampling queries can filter
      `WHERE contract_id = <amm_router_id> AND signature = 'swap'`
      against that table once 0017 lands a local CH instance; until
      then, pull from the public archive per the task's Notes.
---

# Sample-decode per-AMM swap event shapes (Soroswap, Aquarius, Phoenix)

## Summary

For each of the three target Soroban AMMs (Soroswap, Aquarius,
Phoenix), capture one real swap event from the public archive (or
from the local CH instance once task 0017 lands) and decode the
ScVal-encoded `topics_xdr` + `data_xdr`. Record the per-AMM topic
list and data payload shape so the prices-api Tranche 1 consumer's
swap-extraction logic can be written against a precise spec rather
than a guess.

## Context

The CH `soroban_events.signature` column is a `LowCardinality(Nullable(String))`
hoisted first-topic Symbol — but the meaning of "swap" (full topic
list, data layout) is AMM-contract-defined, not protocol-defined.
Each AMM may emit different topic arity (sender/recipient order,
pool-ID inclusion, etc.) and different data ScVal shapes
(`ScVal::Map` vs `ScVal::Vec`, key names, amount direction).

Without this per-AMM mapping pinned, the consumer either has to
introspect every event at run time (slow, fragile) or guess one
canonical shape and fail loudly on the others.

## Implementation

For each AMM:

1. Pick one canonical pair (e.g. XLM/USDC) and one known swap
   transaction (look it up on stellar.expert or by querying the
   local CH instance once 0017 lands, filtering
   `WHERE contract_id = <amm_router_id> AND signature = 'swap'`
   `LIMIT 1`).
2. Decode `topics_xdr` and `data_xdr` with the stellar-xdr parser
   crate.
3. Record:
   - Full topic list with ScVal type for each (`Symbol`, `Address`,
     `U128`, etc.).
   - Data payload shape: ScVal variant + sub-fields + types + units.
   - Token-in / token-out order convention (is `topics[1]` the
     source or the destination?).
   - Amount denomination (stroops? contract-defined precision? signed?).
4. Cross-reference against the AMM contract's source code on its
   GitHub repo to confirm the decoded shape matches the emitter.

Output the findings as `notes/G-amm-swap-event-shapes.md` with a
table per AMM. Spawn a follow-up only if any AMM's shape is
incompatible with the others to a degree that requires a per-AMM
extractor strategy in the consumer.

## Acceptance Criteria

- [ ] One real swap event decoded from each of Soroswap, Aquarius,
      Phoenix.
- [ ] Per-AMM topic + data shape documented in
      `notes/G-amm-swap-event-shapes.md`.
- [ ] Cross-referenced against each AMM's source code; discrepancies
      noted.
- [ ] Recommendation captured: single shared extractor, or per-AMM
      extractors.

## Notes

- This task can run before task 0017 lands by pulling real swap
  events from the public archive — but it is faster once 0017's
  local CH is queryable (`SELECT topics_xdr, data_xdr FROM
  soroban_events WHERE …`).
- If new Soroban AMMs become relevant during the Tranche window,
  this task's pattern (one canonical pair + one sample decode +
  source-code cross-ref) re-runs per new AMM.
