---
id: "S-schema-deploy-verification"
title: "Pre-deploy schema verification — close_usd wired end-to-end; clear to apply to Hetzner"
type: S
task: "0063"
status: mature
spawned_from: ["G-provisioning-plan"]
spawns: []
related_notes:
  - "G-provisioning-plan.md"
links:
  - "../../../../../../stellar-prices-api/packages/prices-clickhouse/schema/init.sql"
---

# Pre-deploy schema verification (2026-06-22)

> Last check before applying the `prices.*` schema to Hetzner (Step 4 / task
> 0051 apply). Verdict: **GREEN — clear to deploy.** Source of truth:
> `packages/prices-clickhouse/schema/` (clean tree; latest commits are the 0061
> `close_usd` + 0051 seed work).

## Conclusion

`close_usd` (the historical USD close BE requested, task 0061) is defined,
propagated, and exposed consistently across the whole apply chain. The two
failure modes most likely to silently break it — MV positional mismatch and
views not being applied — were both checked and are clean. This exact schema set
was already applied successfully on local CH 25.6 in the 0063 128k-ledger test
and has not changed since.

## Evidence

| Layer | File | `close_usd` status |
|-------|------|--------------------|
| Base table + 7 grains | `init.sql` | ✅ `price_ohlcv_1m` **pos 12** (after `volume_quote_usd`); `CREATE … AS` copies inherit it; idempotent `ALTER … ADD COLUMN IF NOT EXISTS … AFTER volume_quote_usd` on all 7 tables (no-op fresh, back-fills pre-0061) |
| Live rollup MV chain | `rollups.sql` | ✅ `argMax(close_usd, timestamp)` through all 6 steps 1m→15m→1h→4h→1d→1w→1M |
| Backfill pre-roll | `preroll.sql` | ✅ same `argMax(close_usd, …)` through all 6 steps |
| Read-surface views | `views.sql` | ✅ `price_usd_series` / `_1h` read `close_usd` from `_1d`/`_1h` |

### Two silent-failure checks

1. **MV positional matching.** CH `TO table AS SELECT` matches by *position*,
   not name. Table DDL and the `rollups.sql`/`preroll.sql` SELECTs agree exactly:
   `…, volume_quote_usd, close_usd, vwap, …` (close_usd at position 12). No
   off-by-one that would fill the rolled column from the wrong source.
2. **Apply coverage.** `prices-clickhouse-init` applies `init.sql` → `seed.sql`
   → **`views.sql` (always, not gated)** → `rollups.sql` (only with `--rollups`).
   The read-surface views ship on every apply.

### USDC issuer literal (load-bearing)

`views.sql` hard-codes the USDC issuer as a hand-synced copy of
`prices_clickhouse::USDC_ISSUER`. Byte-for-byte compared 2026-06-22 — **exact
match** (`GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN`, 56 chars),
and exactly one distinct issuer literal in the file (lines 99 + 145). The
`usd_reference` XLM→USD pivot — and thus the `no_reference` blackout
discriminator — keys off the correct issuer.

## Caveat — schema ready, values are an ingestion concern

The DDL creates/propagates/exposes `close_usd`, but it is **DEFAULT 0 until the
enrichment pass fills it** (`close_usd = oracle_usd × close`), and
`price_usd_series` filters `WHERE close_usd > 0`. So immediately post-apply
`price_usd_series` is **empty** — expected, not a schema gap. It fills once the
writer/enrichment populates `close_usd` (tasks 0026 / 0038 / backfill), not via
this DDL apply.
