---
id: "0026"
title: "volume_quote_usd enrichment Lambda — implement the Phase 1 spec from task 0024"
type: FEATURE
status: blocked
related_adr: ["0003"]
related_tasks: ["0024", "0012", "0022", "0023"]
by: ["0012"]
tags: [layer-indexing, priority-medium, effort-medium, lambda, ohlcv, enrichment, oracle, phase-2]
links:
  - "../archive/0024_FEATURE_volume-quote-usd-enrichment/notes/G-enrichment-pass-design.md"
  - "../archive/0024_FEATURE_volume-quote-usd-enrichment/README.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
history:
  - date: 2026-05-13
    status: blocked
    who: claude
    note: >
      Spawned from 0024 as Phase 2 (implementation). Blocked on
      task 0012 — needs RDS bootstrap, `price_ohlcv` schema, and
      Oracle Fetcher Lambda to be deployed before the enrichment
      Lambda can be built and integration-tested.
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

## Notes

- This task is `blocked` until 0012 lands. When unblocked, move
  via `/lore-framework-tasks` from `blocked/` to `active/`.
- A separate v2 two-hop enrichment task for exotic-quote pairs
  (no direct USD oracle) may be spawned later — see 0024 design
  §3.1. Not part of this Phase 2.
