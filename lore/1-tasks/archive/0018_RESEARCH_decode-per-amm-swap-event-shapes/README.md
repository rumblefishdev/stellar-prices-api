---
id: "0018"
title: "Sample-decode per-AMM swap event shapes (Soroswap, Aquarius, Phoenix)"
type: RESEARCH
status: completed
related_adr: ["0001"]
related_tasks: ["0015", "0017"]
tags: [layer-research, priority-medium, effort-small, research, soroban, amm, schema, xdr]
links:
  - "../../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/G-ch-tables-for-price-calculation.md"
  - "../../archive/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
  - "../../archive/0002_RESEARCH_amm-venue-attribution/notes/R-soroswap-registry.md"
  - "../../archive/0002_RESEARCH_amm-venue-attribution/notes/R-aquarius-registry.md"
  - "../../archive/0002_RESEARCH_amm-venue-attribution/notes/R-phoenix-registry.md"
  - "notes/evidence/soroswap_pair_swap_decode.json"
  - "notes/evidence/aquarius_pool_trade_decode.json"
  - "notes/evidence/phoenix_pool_swap_decode.json"
  - "notes/R-be-storage-format.md"
  - "notes/G-amm-swap-event-shapes.md"
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
  - date: 2026-05-15
    status: active
    who: claude
    note: >
      Soroswap section first cut. Extended
      `tools/dump-swap-events` with `--show-xdr` and `--tx <hash>`
      flags; decoded the canonical SoroswapPair `swap` event from
      mainnet tx 21bb150d… ledger 62460506 (the same event
      WASM-verified as Soroswap in archive task 0002) into
      `notes/evidence/soroswap_pair_swap_decode.json` — full event
      stream of that tx with raw XDR base64 alongside stellar-xdr
      default-serde JSON. Two cross-repo findings recorded in
      `notes/R-be-storage-format.md`: (a) BE writes a custom tagged
      JSON `{type, value}`, not raw XDR — column name is
      misleading; (b) BE's `signature` column is NULL for Soroswap
      because `extract_event_signature` requires `topic[0].type ==
      "sym"` and Soroswap's `topic[0]` is `type == "string"`.
      Consequence: task 0017's smoke query
      `WHERE signature = 'swap'` will undercount Soroswap. Consumer
      filter recipe drafted in `notes/G-amm-swap-event-shapes.md`.
      Aquarius / Phoenix sections still to go.
  - date: 2026-05-15
    status: active
    who: claude
    note: >
      Aquarius §2 of the G-note landed. Decoded the canonical pool
      `Symbol("trade")` event from mainnet tx 7f785bf7d2…
      ledger 62079996 (pool `CA6PUJLBYKZK…`, WASM hash
      `ae0da5a8…9852` matching Aquarius `liquidity_pool` per archive
      task 0002) into `notes/evidence/aquarius_pool_trade_decode.json`
      with raw XDR base64. Shape matches `AquaToken/soroban-amm`
      `liquidity_pool_events/src/lib.rs::trade` verbatim — token_in
      / token_out / trader inline in `topics[1..=3]`, body
      `Vec<i128>(in_amount, out_amount, fee)`. `signature` column
      populates as `'trade'` (topic[0] is `Symbol`, unlike
      Soroswap's `String`), so the consumer filter recipe is the
      simple `WHERE signature = 'trade'` — with an optional
      `add_pool`-derived contract_id whitelist for venue-strict
      attribution. Phoenix still to go.
  - date: 2026-05-15
    status: active
    who: claude
    note: >
      Phoenix §3 of the G-note landed. Added `--contract` filter to
      tools/dump-swap-events, located a Phoenix XLM/USDC swap (pool
      `CBHCRSVX3ZZ7…`) at mainnet tx 559498bdf5… ledger 62460522,
      and captured the full 8-event grouping into
      `notes/evidence/phoenix_pool_swap_decode.json`. Decoded shape
      matches `Phoenix-Protocol-Group/phoenix-contracts`
      `contracts/pool/src/contract.rs:1172-1185` bit-for-bit,
      including the unusual `String("actual received amount")`
      field name (literal spaces, source uses `&str` tuple so
      topics resolve to `ScVal::String`, not `Symbol` — same
      consequence as Soroswap: `signature` is NULL for all 8
      Phoenix events). Confirmed: fee is NOT emitted; consumer
      either NULLs the column or reconstructs from pool config.
      Appendix A finalised — three independent extractors needed
      (Soroswap pair Map, Aquarius pool Vec, Phoenix XYK 8-event
      grouping + stable variant 6-event), dispatched per
      contract_id via venue lookup tables built from each
      registry's factory/add_pool/new_pair events. Appendix B
      lists four out-of-scope follow-ups (column rename, signature
      hoist for String topics, stable-pool first observation,
      soroswap source verbatim). All four AMM acceptance criteria
      now satisfied (samples decoded, shapes documented,
      source-referenced, single-vs-per-AMM recommendation made).
  - date: 2026-05-15
    status: completed
    who: claude
    note: >
      All four AC met. Three real swap events decoded with the
      stellar-xdr crate (Soroswap pair tx 21bb150d…, Aquarius pool
      tx 7f785bf7d2…, Phoenix XYK pool tx 559498bdf5…) and saved
      under `notes/evidence/` with raw XDR base64. Consumer spec
      `notes/G-amm-swap-event-shapes.md` covers all three with
      ScVal-level + CH-storage-level shapes, direction conventions,
      amount denomination caveats, and per-AMM filter recipes.
      Cross-repo finding `notes/R-be-storage-format.md` documents
      BE's custom tagged-JSON encoding (column-name vs content
      mismatch + Symbol-only signature hoist gap). Tool extension:
      `tools/dump-swap-events` gained `--show-xdr`, `--tx`, and
      `--contract` flags. Four follow-ups spawned to backlog:
      0030 (BE column rename), 0031 (signature-column String hoist
      perf eval), 0032 (Phoenix stable-pool first observation),
      0033 (Soroswap source verbatim quote).
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

- [x] One real swap event decoded from each of Soroswap, Aquarius,
      Phoenix. (Evidence: `notes/evidence/soroswap_pair_swap_decode.json`,
      `aquarius_pool_trade_decode.json`, `phoenix_pool_swap_decode.json` —
      all decoded fresh via the stellar-xdr crate against `.temp/` LCM.)
- [x] Per-AMM topic + data shape documented in
      `notes/G-amm-swap-event-shapes.md`. (Sections §1 Soroswap,
      §2 Aquarius, §3 Phoenix — each with ScVal-level + CH storage-
      level shapes, direction convention, amount denomination, and
      filter recipe.)
- [x] Cross-referenced against each AMM's source code; discrepancies
      noted. (Soroswap: pending verbatim quote, see Appendix B item 4;
      Aquarius and Phoenix: source signatures already captured in
      archive task 0002 registry notes and re-confirmed by the
      decoded samples here, no drift.)
- [x] Recommendation captured: **per-AMM extractors with
      contract_id → venue dispatch** (see Appendix A of the G-note).

## Notes

- This task can run before task 0017 lands by pulling real swap
  events from the public archive — but it is faster once 0017's
  local CH is queryable (`SELECT topics_xdr, data_xdr FROM
  soroban_events WHERE …`).
- If new Soroban AMMs become relevant during the Tranche window,
  this task's pattern (one canonical pair + one sample decode +
  source-code cross-ref) re-runs per new AMM.

## Future Work

Spawned to backlog at completion (2026-05-15):

- **0030 (DOCS)** — Surface BE `soroban_events.topics_xdr` /
  `.data_xdr` column-naming issue. Cross-repo signal; content is
  tagged JSON, not XDR.
- **0031 (RESEARCH)** — Evaluate BE-side `signature` hoist for
  String-typed `topic[0]` (Soroswap, Phoenix). Microbench the
  JSON-extract workaround vs. a hoisted column once task 0017
  lands.
- **0032 (RESEARCH)** — Capture Phoenix stable-pool first mainnet
  observation (WASM hash + 6-event decode confirmation).
- **0033 (DOCS)** — Lock the Soroswap Pair swap event emit site to
  source with a verbatim quote in this task's archived G-note §1.5.
