---
id: "0001"
title: "Dump AMM swap event topics+data from .zst ledger sample"
type: RESEARCH
status: completed
related_adr: []
related_tasks: ["0002", "0003", "0004"]
tags: [priority-medium, effort-small, soroban, amm, schema-validation]
links:
  - "../../../../docs/database-schema/amm-trades-schema.md#11-open-questions-for-the-be-team"
  - "../../../../docs/database-schema/amm-trades-schema.md"
history:
  - date: 2026-05-07
    status: backlog
    who: okarcz
    note: "Task drafted from investigation into Soroban swap event topic shape"
  - date: 2026-05-07
    status: active
    who: okarcz
    note: "Promoted from backlog to active"
  - date: 2026-05-07
    status: completed
    who: claude
    note: >
      Built tools/dump-swap-events crate (self-contained, no BE coupling).
      Scanned 614 .xdr.zst files, 2,738,082 events. Found two distinct
      AMM swap-like topic_0 symbols: Symbol("swap") (53 hits, 1 emitter,
      router pattern) and Symbol("trade") (91 hits, 29 emitters, pool
      pattern). Phoenix not observed in 3.5-day window. Spawned 0002
      (venue attribution), 0003 (DOCS update §11.1), 0004 (wider sample
      for Phoenix).
---

# Dump AMM swap event topics+data from .zst ledger sample

## Summary

Built a self-contained Rust tool (`tools/dump-swap-events`) that parses
zstd-compressed `LedgerCloseMetaBatch` files and prints Soroban contract
event topics + data as JSON. Ran it against the 614-file mainnet sample
in `.temp/FC4DB5FF--62016000-62079999/` and captured concrete findings for
the open schema question §11.1 in `docs/database-schema/amm-trades-schema.md`.

## Status: Completed

**Outcome:** Two distinct AMM swap-like topic symbols observed —
`Symbol("swap")` and `Symbol("trade")` — with different decoder shapes.
The schema's per-venue filter mapping is **mandatory**, not hypothetical.
Three follow-up backlog tasks spawned (0002 / 0003 / 0004).

## Implementation Notes

### What was built

- **`tools/dump-swap-events/`** — new self-contained Rust crate (3 files):
  - `Cargo.toml` (~15 lines) — deps: `stellar-xdr 26` with `serde`/`serde_json`
    features, `stellar-strkey`, `zstd`, `serde_json`, `hex`.
  - `src/main.rs` (~325 lines) — CLI tool: zstd decompress →
    `LedgerCloseMetaBatch::from_xdr` → walk V0/V1/V2 ledger metas →
    walk V3/V4 transaction metas → extract events → filter / emit.
  - `README.md` — usage, flags, output schema.
- **Research notes under this task's `notes/`:**
  - `R-swap-topic-shapes.md` — raw observed shapes + counts.
  - `S-amm-trades-schema-§11-1-resolved.md` — synthesis answering §11.1.
  - `evidence/swap_event_sample.json`, `evidence/trade_event_sample.json` —
    captured event JSON for posterity.

### Key numbers from the run

| Metric | Value |
|---|---:|
| Files scanned | 614 / 614 (0 failed) |
| Total contract events seen | 2,738,082 |
| `swap` events | 53 (1 distinct emitter) |
| `trade` events | 91 (29 distinct emitters) |
| `update_reserves` events | 98 (31 emitters; 29/29 overlap with `trade`) |
| `SwappedFromVUsd` events | 1 (non-AMM, virtual-USD synthetic) |

## Acceptance Criteria

- [x] Tool exists and runs end-to-end against the sample without panics.
      Built in `tools/dump-swap-events/` (in this repo, not soroban-block-explorer
      — see "Design Decisions / Emerged" below).
- [x] Output captured for at least one swap event from each of two AMM
      shapes. **Phoenix not observed** in this 3.5-day window — documented
      as a finding (not a blocker) and spawned task 0004 to widen the sample.
- [x] `topics[0]` symbol per shape documented (`R-swap-topic-shapes.md`).
- [x] `data` payload shape per shape documented (`R-swap-topic-shapes.md` —
      includes ScVal-type table and concrete sample JSON).
- [ ] Schema doc `amm-trades-schema.md` §11.1 updated. **Deferred to task 0003.**

## Design Decisions

### From Plan

1. **Self-contained tool, no BE workspace coupling.** Per the user's
   instruction (option C in the readiness check), the tool lives in this
   repo and depends only on `stellar-xdr 26` from crates.io. No path
   dependency on the soroban-block-explorer's `xdr-parser` crate.

2. **Diagnostic events filtered by `EventSource`, not inner type.**
   CAP-67 / soroban-block-explorer task 0182: the diagnostic container
   holds byte-identical Contract-typed mirrors of per-op consensus events
   when diagnostic mode is enabled, so filtering by `event.type_` is
   unsafe. The tool tags each event with `TxLevel | PerOp | Diagnostic`
   based on which container it came from and drops `Diagnostic` by
   default.

### Emerged

3. **Substring topic filter, not exact match.** Plan said
   `--symbol <substring>`; in practice this was crucial — the default
   `--symbol swap` would have matched `swap` but missed `trade` and
   misled the investigation if treated as a hard filter. Workflow ended
   up: histogram first, then targeted `--symbol` runs per discovered
   topic. Documented as the canonical workflow in the tool README.

4. **Added `--histogram` flag mid-investigation.** Plan implied per-event
   output only. After seeing only 53 `swap` hits, needed a way to scan
   all 2.7M events without printing per-event JSON. `--histogram` mode
   counts everything, suppresses per-event emission, prints a sorted
   `topic_0` histogram on stderr. Implies `--no-filter`. This is what
   surfaced `trade` and `SwappedFromVUsd`.

5. **Used stellar-xdr's native serde Serialize for ScVal output.** The
   BE's `xdr-parser` has a custom `scval_to_typed_json` producing
   ergonomic shapes (e.g. `{"address": "G..."}`). Replicating that would
   have been ~200 lines. Instead, enabled the `serde` and `serde_json`
   features on `stellar-xdr` and let serde do it. Output is slightly
   different (`{"symbol":"swap"}` vs `"swap"`, `{"i128":"100"}` vs
   `"100"`) but **more informative for investigation** — type tags are
   visible. No coupling cost.

6. **`hex` and `serde` deps both pulled.** `serde` came in via
   `stellar-xdr` features; `hex` is used only for tx-hash encoding.
   Could have inlined a 4-line hex encoder and dropped the dep, but the
   crate is a one-file investigation tool with no shipping concern.

## Issues Encountered

- **`stellar-xdr 26` defaults don't include serde.** First build failed
  with "the trait bound `ScVal: Serialize` is not satisfied". Fixed by
  adding `features = ["curr", "serde", "serde_json"]` to the
  `stellar-xdr` dep.
- **`ContractEvent.contract_id` is `Option<ContractId>`, not
  `Option<Hash>`.** First draft assumed Hash directly; the v26 type is
  `pub struct ContractId(pub Hash)`, so the strkey conversion uses
  `(cid.0).0` to reach the `[u8; 32]`. Not a regression, just a v26
  detail vs older guides.

## Future Work (spawned as backlog tasks)

- **0002 — RESEARCH:** Attribute the observed contract IDs to specific
  AMM venues (Soroswap / Aquarius / Phoenix) using public registries.
- **0003 — DOCS:** Update `amm-trades-schema.md` §7 step 3 and §11.1
  with the empirical findings (replace hypothetical wording).
- **0004 — RESEARCH:** Re-run `dump-swap-events --histogram` on a
  ~1-month sample to detect Phoenix and any additional swap-like
  topic symbols not seen in this 3.5-day window.

## References

- Tool: `tools/dump-swap-events/` (this repo)
- Findings: `notes/R-swap-topic-shapes.md`, `notes/S-amm-trades-schema-§11-1-resolved.md`
- Evidence: `notes/evidence/swap_event_sample.json`, `notes/evidence/trade_event_sample.json`
- Schema doc this resolves: `docs/database-schema/amm-trades-schema.md` §7, §11.1
