---
id: "0130"
title: "Coarse sweep can't scan price_ohlcv_15m — FINAL scan exceeds the ~30s mTLS-proxy timeout"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0114", "0111"]
tags: [clickhouse, enrichment, coarse-sweep, mtls, infra, priority-medium, effort-medium]
links:
  - "../../../packages/enrichment-worker/src/repair.rs"
  - "../../../infra/src/lib/stacks/eventbridge-stack.ts"
history:
  - date: 2026-07-24
    status: backlog
    who: okarcz
    note: "Spawned from 0114 — the recurring sweep's 15m coverage 504s on the mTLS proxy; 15m dropped at runtime as mitigation."
---

# Coarse sweep can't scan `price_ohlcv_15m` — FINAL scan exceeds the ~30s mTLS-proxy timeout

## Summary

The recurring coarse-USD sweep (task [[0114]], folded into the hourly
`prices-production-enrichment` Lambda) **cannot scan `price_ohlcv_15m`**. Its
first query per table — a single `FINAL` aggregation (`months_with_zeros`) — takes
longer than the **~30-second response timeout on the mTLS front proxy (Caddy)** on
the loaded shared cluster, so the proxy returns **HTTP 504** with an empty body,
which the ClickHouse client surfaces as `BadResponse("")`. Result: every sweep run
logs `coarse sweep: table failed (continuing) table=price_ohlcv_15m` and
`tables_failed=1`. The five forever-tables (`_1h/_4h/_1d/_1w/_1M`) are smaller and
finish under the timeout, so they sweep cleanly.

## Context

- Discovered **2026-07-24**, immediately after the 0114 sweep was deployed
  (EventBridge stack, squash `1f636f7`). `15m` is the biggest coarse table (~4×
  `_1h` rows), and it's the only one that fails.
- **Measured as `prices_writer` over the same mTLS path the Lambda uses** (curl
  replay, the documented technique):
  - `SELECT count() FROM prices.price_ohlcv_15m` → **HTTP 200 in 0.17s** (table
    fine, connectivity fine).
  - `months_with_zeros` `FINAL` scan over 202606–202607 → **HTTP 504 in 30.14s.**
- The `504`, not a ClickHouse `500`/exception, pins it to the **proxy**, not CH or
  a privilege issue. CH is likely still computing when the proxy cuts the
  connection.
- **Load matters.** The 504 happened while the cluster was heavily loaded (a
  manual invoke storm + the 1m pass's 100–185s `FINAL` scans + the 0088 backfill).
  Per 0111, loaded queries run ~80× slower than idle. So 15m may be *borderline*
  rather than fundamentally impossible — **must be re-measured on a quiet cluster.**
- Not damaging: the sweep is best-effort, so the invocation still succeeds, the 1m
  pass is unaffected, and there is no alarm wired on `CoarseSweepTableFailures`
  yet. But the hourly run keeps logging the 15m failure.

## ⚠️ CDK drift to reconcile

As the immediate mitigation, `COARSE_SWEEP_TABLES` was changed **at runtime**
(Lambda console) to drop `15m`:

```
price_ohlcv_1h,price_ohlcv_4h,price_ohlcv_1d,price_ohlcv_1w,price_ohlcv_1M
```

The **CDK default (`eventbridge-stack.ts`) still lists all 6 incl `15m`**, so the
next `Prices-production-EventBridge` deploy would re-add 15m and reintroduce the
failure. This task must reconcile the two — either re-enable 15m once its queries
fit, or remove it from the CDK default with a recorded decision.

## Implementation — options to evaluate

1. **Re-measure on a quiet cluster first.** If 15m's `FINAL` scan is <30s when
   idle, the fix may be "make it resilient under load" rather than "make it
   cheaper". Do this before writing code.
2. **Drop `FINAL` from `months_with_zeros`.** It only decides *which months* to
   process; a non-`FINAL` count over-counts zeros (harmless false-positive, never
   a miss), and is far cheaper. But the enrich-batch queries still need `FINAL`
   for correctness and could 504 on 15m too — so this alone may not be enough.
3. **Per-query `max_execution_time` (< proxy timeout).** Make a slow query fail
   *fast and legibly* (a real CH error) instead of a 30s hang → 504 → empty
   `BadResponse`. Improves diagnosis for ALL tables (any could 504 under peak
   load), even if it doesn't by itself let 15m through.
4. **Tighter bounds for 15m** — smaller `batch_size`, and note 15m's 30-day
   retention means the lookback only ever needs ~1 month.
5. **Raise the Caddy proxy response timeout** for the writer route — an infra
   change on the shared CH front; coordinate with the cluster owner.

## Acceptance Criteria

- [ ] Root cause characterized on a **quiet** cluster — is 15m's `FINAL` scan
      fundamentally >30s, or only under load?
- [ ] The sweep covers `price_ohlcv_15m` with `tables_failed=0`, **or** a recorded
      decision to permanently exclude it (15m's 30-day self-expiry is the
      rationale — its zeros can't become permanent, unlike the forever-tables).
- [ ] CDK `COARSE_SWEEP_TABLES` reconciled with the deployed runtime env — no
      drift either way.
- [ ] Consider the `max_execution_time` hardening so a future proxy-timeout on
      *any* coarse table fails fast + legibly instead of as an opaque empty body.
