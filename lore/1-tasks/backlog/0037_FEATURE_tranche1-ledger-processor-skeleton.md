---
id: "0037"
title: "Tranche 1 Ledger Processor skeleton — Phoenix pool registry + extractor dispatch hook"
type: FEATURE
status: backlog
related_adr: ["0001", "0006"]
related_tasks: ["0034", "0018"]
tags: [layer-indexing, priority-medium, effort-medium, milestone-M1, stream-1, consumer, scaffolding, rust]
milestone: 1
links:
  - "../blocked/0034_FEATURE_consumer-multi-xyk-wasm-tolerance.md"
  - "../archive/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md"
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
history:
  - date: 2026-05-18
    status: backlog
    who: oski
    note: >
      Spawned from 0034. ADR 0006 says the first Rust binary lands
      with the Tranche 1 Ledger Processor; 0034 needs that binary
      (or at least its pool-registry + extractor-dispatch module)
      to exist before it can wire in the pool_type + event_count
      classifier. This task creates the minimum scaffolding 0034
      requires; it does NOT need to be a complete consumer.
---

# Tranche 1 Ledger Processor skeleton

## Summary

Stand up the minimum Rust scaffolding for the Tranche 1 Ledger
Processor so that downstream tasks (starting with 0034) have a
real `phoenix-pool-registry` + extractor dispatch surface to
extend. No backfill logic, no ClickHouse client, no Lambda
packaging — just the crate layout, the per-venue extractor trait
from 0018 Appendix A, and a Phoenix pool registry that 0034 can
attach `pool_type` to.

## Context

Per ADR 0006 the runtime is Rust + axum + sqlx and the first
binary lands with the Tranche 1 Ledger Processor. As of
2026-05-18 `packages/` is empty — only `tools/dump-swap-events`
exists. Task 0034 (`consumer must tolerate ≥2 Phoenix XYK WASM
builds`) was activated and immediately re-blocked because there
is no consumer to apply the tolerance to.

The 0018 G-note already specifies the per-venue extractor trait
shape; 0034's S-note specifies the Phoenix classifier
(`pool_type + event_count`). This task wires those two specs
into a real crate skeleton so 0034 (and Soroswap / Aquarius
follow-ups) become incremental.

## Implementation Plan

### Step 1: Workspace layout

Add a top-level `[workspace]` `Cargo.toml` at the repo root (or
under `packages/`) that owns the Rust crates. Keep the existing
`tools/dump-swap-events` outside the workspace or pull it in
explicitly — decide which at impl time.

Recommended initial crates:

- `packages/extractors-core` — `SwapExtractor` trait,
  `SorobanEventRow`, `ExtractResult`, venue enum.
- `packages/phoenix-extractor` — XYK and stable extractor stubs
  plus the Phoenix pool registry (where 0034 lands).
- `packages/ledger-processor` — binary crate; wires registries
  and extractors into a dispatcher. Stub `main` is fine.

### Step 2: SwapExtractor trait + venue enum

Transcribe the trait from 0018 G-note Appendix A. Implement
`SoroswapPairExtractor`, `AquariusPoolExtractor`,
`PhoenixXykPoolExtractor`, `PhoenixStablePoolExtractor` as
`unimplemented!()` stubs so 0034 (Phoenix XYK) can replace just
its body.

### Step 3: Phoenix pool registry

Define `PhoenixPool { contract_id, pool_type: u32, last_seen_wasm_hash: Option<[u8;32]> }`
and `PhoenixPoolRegistry` with:

- `register(contract_id, pool_type)` — adds a pool entry.
- `lookup(contract_id) -> Option<&PhoenixPool>` — used by the
  dispatcher.
- Seed-from-fixture constructor for unit tests (so 0034's
  config-fixture test does not need a live Soroban RPC).

A live Soroban-RPC-backed `register_from_factory` is **out of
scope** for this task — 0034 can add it, or it can be its own
follow-up.

### Step 4: Dispatcher hook

In `ledger-processor`, sketch a function
`dispatch(rows: &[SorobanEventRow], registry: &PhoenixPoolRegistry) -> Vec<TradeRow>`
that:

1. Resolves each row's `contract_id` against the registry.
2. Picks the extractor by `(pool_type, event_count)` for
   Phoenix; static dispatch for Soroswap / Aquarius (deferred).
3. Returns extracted trades.

The whole thing can compile against `todo!()` for non-Phoenix
venues — the task only needs the Phoenix path callable from a
unit test.

### Step 5: CI

Add `cargo check` / `cargo test` to whatever CI the repo uses
(or to a new `.github/workflows/rust.yml` if nothing exists).
The Nx `nx.json` is TypeScript-only per ADR 0006 — Rust gets a
sibling workflow.

## Acceptance Criteria

- [ ] `cargo build -p phoenix-extractor` succeeds.
- [ ] `cargo test -p phoenix-extractor` runs at least one test
      that constructs a `PhoenixPoolRegistry` from a fixture and
      asserts `lookup()` returns the expected `pool_type`.
- [ ] `SwapExtractor` trait defined in `extractors-core` matches
      the shape in 0018 G-note Appendix A.
- [ ] Stub `ledger-processor` binary compiles; `dispatch()`
      routes a fixture event through the registry without
      panicking on the Phoenix XYK path.
- [ ] CI runs `cargo check` and `cargo test` on PRs.

## Out of scope

- BE ClickHouse client (deferred).
- Live Soroban RPC client for `register_from_factory` (deferred
  — 0034 may add it).
- Soroswap / Aquarius extractor bodies (stubs only).
- Backfill orchestration, Fargate packaging, Lambda runtime
  wiring.
- OHLCV write path.

## Notes

Keep the scaffolding minimal — anything beyond what 0034 needs
to compile and unit-test the classifier is scope creep. If a
later consumer task (e.g. real ClickHouse reader, Soroswap
extractor) needs more shape, add it then.
