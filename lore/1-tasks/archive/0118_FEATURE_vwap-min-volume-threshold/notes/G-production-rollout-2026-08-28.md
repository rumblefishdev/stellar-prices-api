---
title: "Production rollout 2026-08-28 — measurements, the cache defect, and the verification that closed AC 3"
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [rollout, clickhouse, materialized-view, api-gateway, verification]
links:
  - "../../../../../docs/runbooks/0118-min-volume-threshold-rollout.md"
  - "../../../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-08-28
    status: mature
    who: stkrolikiewicz
    note: "Rollout log written as the work happened; the AC-3 evidence lives here."
---

# Rollout log, 2026-08-28

## Pre-merge measurement that reversed the design

Run before applying anything, against prod (26.3.10.60). The **unconditional**
threshold would have blanked `vwap_24h`/`sources` on **2,960 of 3,068 priced
assets (96.5%)**. Per-venue 24h volume distribution over 3,180 priced venues:

| band | venues | share |
|---|---|---|
| ≤ $1 | 2,620 | 82% |
| $1–10 | 275 | 9% |
| $10–100 | 153 | 5% |
| $100–1k | 79 | 2.5% |
| $1k–10k | 38 | 1.2% |
| > $10k | 15 | 0.5% |

Largest casualty would have been USDx at **$124/day** across 4 venues; the rest
were single-venue exotics at $70–97. This is the 2026-08-21 liveness-rollback
shape, so the threshold was made **conditional** (see `S-design-decisions.md`,
decision 6). The distribution also justifies leaving MIN_VOLUME_USD at the
spec's $100: 91% of venues are dust ≤ $10 and real liquidity starts in the
hundreds, so anything in the $10–1k saddle cuts the same population.

## MV apply

`DROP VIEW` + re-`CREATE` from `packages/prices-clickhouse/schema/current.sql`
via `clickhouse-client --multiquery`. Rollback artifact captured first
(`SHOW CREATE TABLE`, 137 lines with a prepended `DROP`), confirming prod ran
the pre-0118 definition. Forced refresh succeeded at 12:18:00, no exception,
scheduler back to `Scheduled`.

Post-apply, against the pre-apply baseline:

- assets with `sources = '{}'` beside a non-zero `price_usd`: **0 → 0** — the
  conditional arm holds; the unconditional form would have driven this toward
  2,960;
- rows 3,616 → 3,620 and non-zero `vwap_24h` 3,242 → 3,298 (ordinary churn);
- **8 of the 15 mixed assets** lost exactly their sub-$100 venue: AQUA's
  soroswap at $0.02, then $2.21, $46.27, $61.36, $56.28, $12.35, $9.21, $60.87.
  Nothing above $100 was dropped.

⚠️ A first comparison of `price_usd`/`volume_24h_usd` across the apply showed
"CHANGED" almost everywhere and **proved nothing**: the two snapshots were
minutes apart and the MV refreshes every minute over a sliding 24h window. The
honest check is pinned to one tick, the way [[0123]] does it — recomputing both
columns from raw `price_ohlcv_1m` rows within `[updated_at − 24h, updated_at]`.
On XLM and on three assets that had just lost a venue, both columns matched the
**unfiltered** raw aggregates exactly. That is the real evidence the threshold
is a weighting rule only.

## The defect the verification found

Deploying the API made the parameter live but **invisible to the gateway
cache**. API Gateway does not key on the query string — it keys on the
parameters declared in `cacheKeyParameters`, and collapses every value of an
undeclared one onto one entry. `/price` declared only the path id; `/assets`
its pre-0118 list.

Measured, in this order, after the TTL expired: `?min_volume_usd=200000` on
`native` returned `{aquarius}` — correct — and the **param-less** request
immediately after returned the same narrowed body. So any caller using the
parameter imposed their cut on every other consumer for the TTL window. Fixed
by declaring `qs('min_volume_usd')` on both routes; the rule is now written
into `api-gateway-stack.ts` so the next query parameter does not repeat it.

The task's own planning note had asserted the opposite ("API Gateway caches on
query params, so `min_volume_usd` becomes part of the key") and worried only
about hit-rate dilution. That note is corrected in the README.

## AC 3 evidence (after the cache fix)

All four responses carry `updated_at 2026-08-28T13:32:00Z` — one MV tick — so
the deltas are the parameter's, not drift.

| request | sources | vwap_24h |
|---|---|---|
| `/price` (no param) | aquarius, sdex, soroswap | 0.18374529364442 |
| `?min_volume_usd=100` | aquarius, sdex, soroswap | 0.18374529364442 |
| `?min_volume_usd=5000` | aquarius, sdex | 0.18374945623882 |
| `?min_volume_usd=200000` | aquarius | 0.18383237183385 |

`price_usd` was **0.18383237183385 in all four** — the weighting rule holds
end to end. The param-less request run immediately after the filtered one
returned the full source set, so the poisoning is gone.

`GET /assets` narrows per row the same way; EURC reaches the all-excluded
sentinel (`sources {}`, `vwap_24h 0`) at `=200000`, while USDCAllow is
untouched at every threshold (its single venue carries $36M). Invalid values
(`abc`, `-1`, `1e16`) return `400` with `invalid_query`.
