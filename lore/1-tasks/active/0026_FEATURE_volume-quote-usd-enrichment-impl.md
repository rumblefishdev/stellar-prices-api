---
id: "0026"
title: "volume_quote_usd enrichment Lambda — implement the Phase 1 spec from task 0024"
type: FEATURE
status: active
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
