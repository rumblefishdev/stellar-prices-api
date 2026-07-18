---
title: "Production schema apply / version-tracking strategy"
prefix: S
status: mature
spawned_from: "0051"
related_tasks: ["0051", "0060", "0063"]
date: 2026-06-17
---

# S — Production apply / version-tracking strategy

## Question

How is the `prices.*` schema applied and tracked in production — wholesale
idempotent re-apply (as already shipped in `prices-clickhouse`), or numbered
migrations with a `schema_migrations` version table (the original 0051 sketch)?

## Decision

**Keep the wholesale-idempotent model that `prices-clickhouse` already ships.**
The `prices-clickhouse-init` binary applies `init.sql` → `seed.sql` →
`views.sql` (+ `rollups.sql` under `--rollups`); every statement is
`CREATE … IF NOT EXISTS` and the seed is guarded by a `NOT IN`, so a re-run is
a safe no-op. No `schema_migrations` table, no numbered-migration runner.

## Why

1. **Mirrors BE.** BE's `db-clickhouse` applies a single idempotent `init.sql`
   via a sidecar; matching that keeps the two tenants operationally identical
   and lets the same Ansible/sidecar pattern apply our schema (see [[0063]]).
2. **Already built and tested.** The apply path + Docker integration test
   (`views_it.rs`) exist and pass; a versioned runner would be net-new surface
   to maintain for no current benefit.
3. **CREATE … IF NOT EXISTS covers Tranche 1.** Every object is additive; there
   is no destructive change in scope that idempotent re-apply cannot express.
4. **Smaller blast radius.** One embedded SQL source of truth, diffed in PRs,
   re-applied on demand — no migration-ordering or partial-apply state to
   reason about.

## Limits / when to revisit

Idempotent re-apply cannot safely express **destructive or mutating** DDL:
a column drop, a type change, an engine/ORDER BY change, or a data backfill
that must run exactly once. The first time such a change is needed:

- Spawn a dedicated migration task; introduce a minimal
  `prices.schema_migrations(version UInt32, applied_at DateTime)` +
  numbered files **then**, scoped to the changes that need it — do not
  pre-build it now.
- Until then, schema evolution = edit the embedded `.sql` + re-run the binary.

## Consequences for the live apply (0051 Steps 3–4)

- The live apply is just the existing binary pointed at Caddy:443 over mTLS
  (via 0052's `client_mtls`), run under a **DDL-capable** identity — not
  `prices_writer` (`write_no_ddl`). Resolve that identity with [[0063]].
- `seed.sql` runs as part of the same apply, so production gets the two
  canonical `backfill_progress` rows with no extra step.
