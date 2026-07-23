---
id: "0117"
title: "Define the Tranche 2 (M2) task set — author 11 new tasks, tag 4 existing (+0116 on merge)"
type: DOCS
status: backlog
related_adr: ["0006", "0007", "0008"]
related_tasks:
  ["0072", "0080", "0101", "0104", "0116", "0118", "0119", "0120", "0121", "0122", "0123", "0124", "0125", "0126", "0127", "0128"]
tags: [layer-docs, priority-high, effort-small, milestone-M2, meta, lore-admin, task-definition, tranche-2-scope]
milestone: 2
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../../docs/scf/milestone-1-evidence.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Meta-task owning the lore commit that defines the Tranche 2
      (milestone-M2) task set, following the 0057 precedent used for M1.
      Authored 11 new tasks (0118-0128) and applied `milestone-M2` +
      `milestone: 2` to the 4 existing tasks reachable on `develop`
      (0072, 0080, 0101, 0104); 0116 is M2 scope but lives on an unmerged
      branch and is tagged on merge.
      Scope drawn from overview §9 "Tranche 2 — Public API (Weeks 5–9)",
      §4 (endpoint contracts), §5.5 (VWAP layering), §6 (cache TTLs), and
      the "deferred to Tranche 2" rows of `docs/scf/milestone-1-evidence.md`
      Table 4.
---

# Define the Tranche 2 (M2) task set

## Summary

Author the lore task set that fully covers the Milestone 2 / Tranche 2
deliverable — design-doc §9 *"Tranche 2 — Public API (Weeks 5–9)"* of
`docs/prices-api-general-overview.md` — plus the four items the Milestone 1
submission explicitly deferred to Tranche 2 in
`docs/scf/milestone-1-evidence.md` Table 4.

Output: **11 new backlog tasks (0118–0128)** and the `milestone-M2` tag applied
to **5 existing tasks** (0072, 0080, 0101, 0104, 0116).

Milestone 3 (Tranche 3 — Production Launch & Validation) is deliberately **out
of scope** for this task set and gets its own definition pass later.

## Context

M1 shipped considerably more than Tranche 1 strictly required. A survey of the
delivered state (2026-07-23) found:

| §9 Tranche 2 work bullet | Actual state after M1 |
|---|---|
| 7 endpoint groups implemented | **All routed and deployed** (`packages/prices-api/src/{assets,batch,oracles,backfill}`), but only `GET /backfill/status` was verified for M1 |
| API Gateway caching, usage plans, API keys, throttling | **Built in CDK** (`infra/src/lib/stacks/api-gateway-stack.ts`; `apiGatewayCacheEnabled: true` in `envs/production.json`) — **not verified** end-to-end |
| Full §5.5 VWAP in the current-price producer | **Partial.** `mv_current_prices` (`packages/prices-clickhouse/schema/current.sql`) writes only 6 of 10 columns; `vwap_24h` has no `min_volume_usd` gate |
| Outlier detection (inter-source median) | **Not implemented** — noted as a follow-up in `current.sql:22` |
| Aquarius as a named source in VWAP | Aquarius **extraction** works (venue-routed, `pool-registry-seed`), but `sources` is `''` for every asset, so no source is named anywhere |
| Input validation | Asset-identifier parsing validates; **query/body param validation is uneven** |

So M2 is **less "build the API" and more "complete, harden, and prove it"** —
which is what the §9 acceptance criteria actually ask for (correct responses for
20 assets, a load-test report, cache hits, VWAP reconciled against raw rows,
backfill depth ≤ 2022-01-01).

Two scope decisions were taken when defining the set (2026-07-23):

1. **Tranche-2 boundary follows the M1 evidence doc, not §9 alone.** §9 lists
   the OpenAPI spec and the onboarding portal under Tranche 3, but
   `milestone-1-evidence.md` Table 4 promised a reviewer that Swagger, the
   custom domain / WAF / CORS, and a real CloudWatch dashboard land in
   Tranche 2. M2 therefore includes **exposing the OpenAPI spec through the
   gateway** (0124), the **dashboard** (0125), and **CORS + custom domain**
   (0126). The full Swagger **UI** and the self-service onboarding **portal**
   remain Tranche 3.
2. **Existing backlog tasks are tagged rather than duplicated.** 0072 already
   owns the four deferred `current_prices` columns *and* the §5.5 median-outlier
   filter (absorbed from 0068), so 0118 was scoped to the parts 0072 does not
   cover — the `min_volume_usd` threshold and its request-level override.

## What this task produces

**New tasks** (`backlog/`, all `milestone-M2`):

| ID | Type | Title | Drives |
|----|------|-------|--------|
| 0118 | FEATURE | §5.5 `min_volume_usd` threshold + per-request override | §9 "Full VWAP formula", §5.5 |
| 0119 | FEATURE | Input-validation hardening across every param | §9 "Input validation", AC 1 |
| 0120 | TEST | Endpoint conformance — 7 route groups × 20 major assets | AC 1 |
| 0121 | TEST | Load test at 100 req/s + published report | AC 2 |
| 0122 | TEST | API Gateway cache verification (per-endpoint TTLs, `X-Cache: Hit`) | AC 3 |
| 0123 | TEST | VWAP reconciliation against raw `price_ohlcv` rows | AC 4 |
| 0124 | FEATURE | Expose the OpenAPI spec through API Gateway | M1 evidence Table 4 |
| 0125 | FEATURE | CloudWatch dashboard with real data widgets | M1 evidence Table 4 |
| 0126 | FEATURE | API edge — CORS preflight, custom domain, WAF decision | M1 evidence Table 4 |
| 0127 | FEATURE | Tranche 2 backfill-depth gate (≤ 2022-01-01) + USDC spot-check | AC 5, AC 6 |
| 0128 | DOCS | SCF Milestone 2 verification package | submission |

**Existing tasks tagged `milestone-M2`** (no shape changes):

| ID | Why it is M2 |
|----|--------------|
| 0072 | Materializes `sources` / `price_xlm` / `change_24h_pct` / `change_7d_pct` + the §5.5 median-outlier filter — the §4.1/§4.2 response contract and the "Aquarius as a named source" bullet both depend on it |
| 0080 | Aquarius concentrated-pool swap shape — Aquarius source completeness |
| 0101 | Live-era AMM reprice gap (Soroswap 9-day hole, Phoenix ~2%) — already carried `milestone-M2`; listed here for completeness |
| 0104 | Rollup MV cadence vs window — the 1d candles AC 6 spot-checks come out of that chain |

> **0116 could not be tagged in this commit.** `0116_BUG_dust-trade-candles-produce-absurd-usd-prices`
> exists only on the unmerged `fix/0114_repair-preflight-and-runbook-gaps` branch,
> so it is not present on `develop`. It **is** M2 scope — absurd `close_usd`
> values are served verbatim by `GET /assets/{id}/ohlcv` — and must be tagged
> `milestone-M2` / `milestone: 2` when that branch merges. Tracked as an
> acceptance criterion below.

## Dependency shape

```
0072 ─┬─→ 0118 (threshold sits on top of the outlier filter)
      ├─→ 0123 (nothing to reconcile until `sources` is populated)
      └─→ 0120 (AC 1 "correct responses" includes the stubbed columns)

0119 ──→ 0120

0088 (active, backfill run) ──→ 0127 ──→ 0128
0116, 0101, 0080 ──→ 0127 (USD/AMM correctness before the spot-check)

0120, 0121, 0122, 0123, 0124, 0125, 0126, 0127 ──→ 0128 (evidence package)
```

Suggested execution order: **0072 → 0118 → 0119 → 0124 → 0120 → 0122 → 0123 →
0121 → 0125 → 0126 → 0127 → 0128.**

## Acceptance Criteria

- [ ] 11 new backlog tasks (0118–0128) exist, each with `milestone: 2` and the
      `milestone-M2` tag
- [ ] 4 existing tasks reachable on `develop` (0072, 0080, 0101, 0104) carry
      `milestone: 2` + `milestone-M2` with a history entry recording it
- [ ] 0116 tagged `milestone-M2` once `fix/0114_repair-preflight-and-runbook-gaps`
      merges — it is M2 scope but did not exist on `develop` at authoring time
- [ ] Every §9 Tranche 2 work bullet and every Tranche 2 acceptance criterion
      maps to at least one owning task (traceability table above)
- [ ] Every "Tranche 2" row of `milestone-1-evidence.md` Table 4 maps to an
      owning task
- [ ] No task in the set covers Tranche 3 scope (Swagger UI, onboarding portal,
      integration-suite-in-CI, security review, public repo, 7-day report)
- [ ] Index regenerated (`lore-framework_generate-index`) and PR opened against
      `develop`

## Notes

- Budget line in §9 for Tranche 2 is still `$XX,XXX` — a placeholder in the
  design doc, unchanged by this task.
- §9's Tranche 2 text still says `sdex-cloud-push`; per **ADR 0009** that step
  no longer exists — the backfill CLI writes directly to Hetzner. 0127 carries
  the correction rather than re-editing the design doc here.
