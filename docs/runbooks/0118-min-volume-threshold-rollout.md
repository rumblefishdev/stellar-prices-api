# Runbook — rolling out the §5.5 `min_volume_usd` threshold (task 0118)

What changes: the `mv_current_prices` SELECT only (an `asset_has_funded`
window in `per_source_kept` and the conditional `src_volume > 100` filter in
the new `per_source_funded` CTE, plus comments). No table schema change, no
read-surface view change. The API deploy adds `?min_volume_usd=` to
`GET /assets/{id}/price` and `GET /assets` — a handler-side reweight of the
row's own `sources` JSON, no new queries.

**Procedure: follow
[0072-current-prices-mv-rollout.md](0072-current-prices-mv-rollout.md) steps
0–3 verbatim** (rollback artifact, cost probe, counterfactual, DROP +
re-CREATE, verify). Skip its steps 4–5 (no view change this time). The
exposure is the same staleness window: a refreshable MV's definition is fixed
at create time, so the change is `DROP VIEW` + re-`CREATE`; `current_prices`
keeps serving its last-written rows in the gap (≤ ~1 min + one refresh
duration) — staleness, not an outage.

## The counterfactual that changed the design (recorded, not to re-run)

The original (unconditional) form was measured on prod **2026-08-27, before
merge**: it would have blanked `vwap_24h`/`sources` on **2,960 of 3,068
priced assets (96.5%)** — ~85% of the table has a max per-venue 24h volume of
≤ $1, and the largest casualty traded $124/day. That is the 2026-08-21
liveness-guard rollback shape, caught by measurement instead of incident. The
threshold is therefore **conditional**: a below-threshold source is dropped
only when the asset still has a source above the threshold. Expected visible
delta at rollout: **no asset loses all its sources to the threshold**; only
mixed assets (dust venue beside a funded one) lose their dust entries.

Query used (kept for re-measurement if the conditional decision is ever
revisited):

```sql
SELECT
    countIf(max_vol <= 100)  AS would_blank_if_unconditional,
    count()                  AS priced_assets_today
FROM (
    SELECT asset_id, max(src_volume) AS max_vol
    FROM (
        SELECT asset_id, source, sum(volume_quote_usd) AS src_volume
        FROM prices.price_ohlcv_1m FINAL
        WHERE timestamp >= now() - INTERVAL 24 HOUR
        GROUP BY asset_id, source
        HAVING argMaxIf(close_usd, timestamp, close_usd > 0) > 0
    )
    GROUP BY asset_id
)
```

## Pre-apply capture (run BEFORE the DROP, read-only)

Count the population the conditional threshold WILL touch — assets with a
dust venue beside a funded one — and capture a few as verification subjects:

```sql
SELECT asset_id, sources FROM prices.current_prices FINAL
WHERE sources LIKE '%volume_24h%'
  AND asset_id IN (
      SELECT asset_id FROM (
          SELECT asset_id, source, sum(volume_quote_usd) AS v
          FROM prices.price_ohlcv_1m FINAL
          WHERE timestamp >= now() - INTERVAL 24 HOUR
          GROUP BY asset_id, source
      ) GROUP BY asset_id
        HAVING min(v) <= 100 AND max(v) > 100 AND count() >= 2
  )
LIMIT 5
```

## Post-apply verification

1. **The dust venue is gone, the asset keeps its vwap.** Re-read the captured
   subjects: the sub-$100 source must be absent from `sources`, `vwap_24h`
   non-zero, and `price_usd` / `volume_24h_usd` **unchanged** against the
   captured row (the threshold is a weighting rule only).
2. **No threshold blanking.** Count assets with `sources = '{}'` and a
   non-zero `price_usd` before and after: the delta must be ≈ 0 (the
   conditional arm keeps all-dust assets' sources; any jump toward the
   recorded 2,960 means the unconditional form shipped by mistake).
3. **API override** (after the API deploy, over the gateway with a key):
   - default path stability **on a funded asset**: `GET /v1/assets/native/price`
     and the same with `?min_volume_usd=100` must return identical bodies —
     the producer already made that cut, so the strict filter drops nothing.
     Do not run this check on an all-dust asset: there an explicit `=100`
     correctly empties `sources` while the param-less call keeps them (the
     conditional default), and the two bodies are _supposed_ to differ;
   - narrowing: on a captured multi-source subject, a high
     `?min_volume_usd=` (e.g. `10000`) returns strictly fewer `sources` keys
     and a reweighted `vwap_24h`;
   - `?min_volume_usd=abc` → 400 with the standard envelope.

⚠️ Cache note (task 0122): `min_volume_usd` is part of the API Gateway cache
key. The common path must send **no** param — `min_volume_usd=100` buys a
separate cache entry whose content merely happens to match on funded assets,
and differs on all-dust ones (an explicit value filters strictly). Nothing to
operate here; just do not "helpfully" add the default to dashboards or
examples.
