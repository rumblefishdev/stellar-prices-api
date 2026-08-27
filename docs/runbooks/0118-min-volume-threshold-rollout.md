# Runbook — rolling out the §5.5 `min_volume_usd` threshold (task 0118)

What changes: the `mv_current_prices` SELECT only (a `src_volume > 100`
predicate in `per_source_kept`, plus comments). No table schema change, no
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

## 0118-specific counterfactual (run BEFORE the DROP, read-only)

The threshold blanks `vwap_24h`/`sources` for every asset whose **every**
priced source is at or below $100 of trailing-24h volume. Measure that
population first, so the post-apply delta is a prediction checked rather than
a surprise explained:

```sql
SELECT
    countIf(max_vol <= 100)  AS will_blank,
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

Record both numbers in the task. `will_blank` is expected to be a meaningful
slice of the low-volume tail — that is the spec working as written (§5.5 is
unconditional), not a defect. If the number looks wrong by an order of
magnitude, stop and re-derive before applying.

Also capture one **multi-source asset with a sub-$100 venue** in today's
`sources` (the verification subject for after):

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
2. **The blank population matches the prediction.** Count assets with
   `sources = '{}'` and a non-zero `price_usd`; the delta against the same
   count captured before the apply should be ≈ `will_blank` (modulo one
   minute of market drift).
3. **API override** (after the API deploy, over the gateway with a key):
   - default path stability: `GET /v1/assets/native/price` and the same with
     `?min_volume_usd=100` must return identical bodies;
   - narrowing: on a captured multi-source subject, a high
     `?min_volume_usd=` (e.g. `10000`) returns strictly fewer `sources` keys
     and a reweighted `vwap_24h`;
   - `?min_volume_usd=abc` → 400 with the standard envelope.

⚠️ Cache note (task 0122): `min_volume_usd` is part of the API Gateway cache
key. The common path must send **no** param — `min_volume_usd=100` is a
different cache entry with identical content. Nothing to operate here; just do
not "helpfully" add the default to dashboards or examples.
