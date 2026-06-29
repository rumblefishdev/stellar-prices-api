---
id: "0026"
title: "volume_quote_usd enrichment Lambda — implement the Phase 1 spec from task 0024"
type: FEATURE
status: active
related_adr: ["0003", "0004", "0007"]
related_tasks: ["0024", "0012", "0022", "0023", "0038", "0058", "0059"]
tags: [layer-indexing, priority-medium, effort-medium, lambda, ohlcv, enrichment, oracle, phase-2, clickhouse]
links:
  - "../../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md"
  - "../../archive/0024_FEATURE_volume-quote-usd-enrichment/README.md"
  - "../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
history:
  - date: 2026-05-13
    status: blocked
    who: claude
    note: >
      Spawned from 0024 as Phase 2 (implementation). Blocked on
      task 0012 — needs RDS bootstrap, `price_ohlcv` schema, and
      Oracle Fetcher Lambda to be deployed before the enrichment
      Lambda can be built and integration-tested.
  - date: 2026-06-08
    status: active
    who: oski
    note: >
      Activated with the same scope reduction applied to task 0038:
      local-only Rust crate + a written design document for the BE
      cross-team meeting. No AWS deploy, no CDK apply, no live CH
      writes, no EventBridge rule registration. The deliverable is
      a runnable local binary the operator can demonstrate against
      fixture data plus a G-note that lets BE react to the schema
      and merge-semantics choices before any infra commitment.

      **Critical architectural caveat.** The 0024 design spec was
      written 2026-05-13 against the original RDS-Postgres data
      plane. ADR 0007 (accepted 2026-05-20) supersedes that to
      Hetzner ClickHouse. The PG-flavoured SQL in §2 of the
      0024 G-note (`WITH ... FOR UPDATE SKIP LOCKED`,
      `UPDATE ... FROM`, row-lock-bounded batches) does not
      translate directly: CH has no row locks, `ALTER TABLE ...
      UPDATE` is asynchronous, and the idiomatic enrichment
      pattern is INSERT-with-newer-version into a
      `ReplacingMergeTree`, deduped on next merge by the
      ORDER BY key. The local prototype + spec G-note translate
      the algorithm to CH semantics; the PG→CH translation is
      one of the BE-meeting agenda items.

      Out-of-scope for this activation: any AWS deploy,
      EventBridge / Scheduler rule, IAM grants, SSM consumption,
      CDK stack apply, or live CH writes — see the forthcoming
      G-note under `notes/G-local-prototype-spec.md` for the full
      Part C cross-team contract.
  - date: 2026-06-09
    status: blocked
    who: oski
    note: >
      Re-blocked after landing the local prototype + production CH
      Form-B enrichment path (commit 75d00d0). The reduced
      local-only scope is delivered: runnable crate, fixture path,
      production INSERT…SELECT ASOF-JOIN path wired, schema
      requirement (ReplacingMergeTree + restored `volume_quote`)
      documented, follow-ups 0058/0059 spawned. The remaining
      acceptance criteria are integration-only and cannot be met
      without live infra: blocked on 0012 (live ClickHouse endpoint
      + Oracle Fetcher writing `oracle_prices`) and 0051
      (`price_ohlcv_1m` + MV rollup-chain DDL deployed). Also gated
      on 0058 (writers must populate `volume_quote`) and a BE
      cross-team review of the schema/merge-semantics choices in the
      G-note before any infra commitment. The production path
      compiles and passes the prototype unit suite but has NOT been
      run against a live ClickHouse.
  - date: 2026-06-26
    status: active
    who: oski
    note: >
      **Unblocked — every dependency from the 2026-06-09 re-block has
      resolved.** 0012 (live CH endpoint + Oracle Fetcher) ✅ completed;
      0051 (`price_ohlcv_1m` + MV rollup-chain DDL) ✅ completed and live on
      ch-prod-01; 0059 (rollup version propagation under enriched re-inserts)
      ✅ completed/merged (PR #61). The `volume_quote` data dependency
      (tracked as 0058) is satisfied: the shared
      `prices_ingest_core::OhlcvWriter` (writer.rs:122/210) now populates
      `volume_quote` for BOTH writer paths (sdex-backfill + the 0038
      ledger-processor), with `volume_quote_usd` left at DEFAULT 0 for this
      enrichment to fill. Live CH is reachable (0063 tenant + 0052 mTLS, per
      memory). Moving blocked → active to drive the remaining
      integration-only ACs. **Constraints carried in:** stay local-first /
      prepare-not-deploy (no AWS deploy, EventBridge rule, or live prod
      writes without explicit approval); the BE cross-team review of the
      Form-B merge semantics in `notes/G-local-prototype-spec.md` is still
      an open agenda item, not a code blocker. (Body "Context" / "blocked
      until 0012" prose is historical — 0012 is archived/completed.)
  - date: 2026-06-29
    status: active
    who: oski
    note: >
      **Re-integrated the orphaned `enrichment-worker` crate into the Cargo
      workspace and drove the locally-achievable integration ACs green.**
      The crate had been dropped from the root `Cargo.toml` `members` list by
      a later merge (the `members` block was rewritten when 0039/0054's crates
      landed), leaving it out of `cargo metadata`, `cargo check --workspace`,
      and `cargo test --workspace` — i.e. silently un-built and un-tested by CI
      since the prototype commit (d13d3dc). Re-added the member; the crate
      compiles in-workspace against the current `prices-clickhouse` and the
      full workspace check stays clean. Fixed one `collapsible_if` clippy nit
      in `ch_enrich.rs` (run path; behaviour unchanged). Verified: 24 unit +
      2 e2e pass; **all 3 `#[ignore]`d live-CH integration tests
      (`tests/ch_enrich_it.rs`) pass against a local ClickHouse pinned to the
      prod version 26.3.10.60** — exercising the oracle / stablecoin-peg /
      XLM-pivot tiers, idempotency (2nd pass = no change), oracle-miss stays
      `close_usd = 0`, budget-exhaustion-defers-not-pegs, and the snapshot
      watermark bound, plus the run-scoped pivot-ref table cleanup. CI keeps
      these `#[ignore]`d (no CH on the runner) so they compile but don't run
      there. **Remaining ACs are deploy/infra-gated** (CDK Lambda + EventBridge
      Scheduler rule + IAM, CloudWatch metric publish + dashboard, live-prod
      backfill credibility check) and stay out of scope under the carried-in
      prepare-not-deploy constraint; the BE Form-B review is still open. Task
      stays `active`.
---

# `volume_quote_usd` enrichment Lambda — implementation

## Summary

Implement the EventBridge cron Lambda specified in task 0024's
[Phase 1 design G-note](../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md).
Phase 1 produced a complete spec; this Phase 2 task lands the
running code + CDK + integration test.

## Context

Task 0024 (archived) split into a design Phase 1 (the G-note) and
this implementation Phase 2. The split was driven by the fact
that 0024's implementation can't usefully exist before task 0012
provides:

- RDS PostgreSQL with `price_ohlcv`, `oracle_prices`, and `assets`
  tables.
- The Oracle Fetcher Lambda writing `oracle_prices` rows.
- The SDEX backfill writing `price_ohlcv` rows that need
  enrichment.

When 0012 lands, 0026 unblocks and can be promoted.

## Implementation

Follow the design spec §1–§6 verbatim. Concretely:

- **Code**: new Rust crate (e.g. `crates/enrichment-worker`) in
  whatever workspace structure task 0012 sets up.
- **CDK**: Lambda function + EventBridge rule + IAM role +
  CloudWatch metric filters. Reuse the patterns 0012 establishes
  for the other Lambdas.
- **Tests**: an integration test that seeds `price_ohlcv` +
  `oracle_prices` and asserts the UPDATE result row-by-row.
- **Telemetry**: emit the four metrics enumerated in spec §5.
- **Alarms**: CloudWatch alarm on
  `EnrichmentRowsRemainingAtVolumeZero` per spec §5.

The historical (post-backfill) one-shot pass per spec §4 lands
either as a separate Lambda or as an invocation mode of the same
Lambda — implementer's choice; document the choice in this task's
notes when made.

## Acceptance Criteria

Carried over from task 0024's design spec §7:

- [ ] EventBridge cron Lambda exists with the schema in §2 wired up.
- [ ] CDK + IAM matches §1.1 / §1.2.
- [ ] Re-running on already-enriched rows produces zero changes
      (idempotency test).
- [ ] Rows with missing oracle stay at `volume_quote_usd = 0`,
      `EnrichmentOracleMiss` metric increments.
- [ ] After full SDEX backfill + a one-shot historical enrichment
      pass, `current_prices.volume_24h_usd` for at least 3
      XLM-quoted assets reflects SDEX-sourced volume (>0 and
      credible against Horizon's historical aggregates).
- [ ] CloudWatch metrics from spec §5 are emitted and visible in
      the dashboard.

## Future Work

Spawned from the production implementation (see G-note Decision Log,
2026-06-09):

- **0058** — populate the restored `volume_quote` column in the OHLCV
  writers (prices-ledger-processor 0038 + sdex-backfill + soroban-amm
  backfill). Enrichment reads this column directly; writers must fill it.
- **0059** — MV rollup-chain version propagation under enriched `_1m`
  re-inserts (task 0051 dependency). 0026 enriches `_1m` only.

## Notes

- This task is `blocked` until 0012 lands. When unblocked, move
  via `/lore-framework-tasks` from `blocked/` to `active/`.
- A separate v2 two-hop enrichment task for exotic-quote pairs
  (no direct USD oracle) may be spawned later — see 0024 design
  §3.1. Not part of this Phase 2.
