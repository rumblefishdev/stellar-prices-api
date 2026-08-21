-- prices.current_prices refreshable MV (task 0039, extended by task 0072) —
-- replaces the per-minute "Current Price Updater" Lambda (ADR 0007 / 0039 Q#1).
-- The scheduler lives inside ClickHouse: REFRESH EVERY 1 MINUTE re-derives one
-- row per asset from price_ohlcv_1m and writes prices.current_prices
-- (ReplacingMergeTree → latest per asset_id; read with FINAL). The MV is the
-- SOLE writer of current_prices.
--
-- NOT applied by prices-clickhouse-init (kept out of init.sql so schema-apply
-- stays version-agnostic). Refreshable MVs require ClickHouse ≥ 23.12.
--
-- DEPLOY ORDER / DEPENDENCY: every USD column here derives from
-- price_ohlcv_1m.close_usd and .volume_quote_usd, which the ingest path writes
-- as DEFAULT 0 — they are filled by the enrichment pass (task 0026). Until that
-- pass is deployed and has run, this MV emits price_usd / volume_24h_usd /
-- vwap_24h / market_cap_usd = 0 for every asset. Apply this MV only once
-- enrichment is live, otherwise current_prices serves all-zero rows.
--
-- ⚠️ REDEPLOY MECHANICS (task 0068 → 0072): a refreshable MV's definition is
-- FIXED AT CREATE TIME. Changing this SELECT requires DROP VIEW + re-CREATE —
-- an ALTER does not take. No backfill/migration is needed: the MV fully
-- recomputes every row on each refresh, so the first refresh after the change
-- populates the new columns for all assets. current_prices keeps serving its
-- last-written rows in the gap, so the exposure is a staleness window (≤ ~1 min
-- plus refresh duration), not an outage.
--
-- ⚠️ POSITIONAL-INSERT FOOTGUN (0039 review): the TO clause carries an EXPLICIT
-- target column list. A materialised view inserts into its target POSITIONALLY
-- otherwise, and this SELECT's projection order is not the table's column order.
-- EVERY new column must be added to BOTH the TO(...) list and the SELECT, in
-- matching order.
--
-- Columns (task 0072 completes the set; all ten are now written):
--   price_usd       — latest PRICED close in the 24h window (argMaxIf, 0135);
--                     NOT age-bounded — see the unfiltered CTE for why. The
--                     per-venue pipeline IS bounded, but conditionally: see 1b
--   price_xlm       — price_usd re-expressed in XLM (÷ the XLM/USD close)
--   change_24h_pct  — vs the oldest close inside the 24h window
--   change_7d_pct   — vs the oldest priced close in the [7d, 5d] band of
--                     price_ohlcv_1h (a real ~7-day baseline or the sentinel)
--   volume_24h_usd  — trailing-24h USD volume, ALL sources (a total, never filtered)
--   market_cap_usd  — price_usd × circulating supply from prices.asset_supply
--   vwap_24h        — USD volume-weighted close across sources, with the §5.5
--                     inter-source median-outlier filter applied
--   sources         — JSON per-source {price, volume_24h}; outlier-excluded
--                     sources are ABSENT from the object (general-overview §3.3)
--
-- ── Numeric strategy ────────────────────────────────────────────────────────
-- Decimal×Decimal widens scale past Decimal(38,14)'s budget (14+14=28 scale
-- leaves only 10 integer digits → overflow at ~1e10), so arithmetic columns
-- cannot multiply natively:
--   vwap_24h / price_xlm / change_*_pct — all price-magnitude or ratios, so they
--                    cannot overflow; computed in Float64 (≈15-16 sig digits,
--                    ample for a price) and cast back with toDecimal128(…,14) /
--                    toDecimal64(…,4).
--   market_cap_usd — price × supply can be huge, so Float64 would both lose
--                    low-order digits AND throw on overflow (poisoning the whole
--                    refresh). Computed as an EXACT Decimal256 product instead,
--                    then accurateCastOrNull back to Decimal(38,14): out-of-range
--                    → NULL → 0 (the column's "unavailable" sentinel) rather than
--                    a refresh-killing exception.
-- price_usd (argMaxIf, no arithmetic) and volume_24h_usd (a plain sum) stay native
-- Decimal. The `sources` JSON serialises its numbers via toString() on the
-- NATIVE Decimal — never through Float64 — so the JSON preserves full
-- Decimal(38,14) precision, matching general-overview §3.3's "numeric values
-- serialised as strings to preserve precision".
--
-- ── §5.5 inter-source median-outlier filter ─────────────────────────────────
-- "Before a source's price is included in the VWAP, it is compared against the
-- inter-source median. Sources deviating by more than a configurable percentage
-- are excluded from that update cycle." Implemented as an array pipeline per
-- asset (arrayReduce('median', …) + arrayFilter) rather than a window function,
-- because CH's window-function surface does not cover quantile aggregates.
--
-- ⚠️ THE FILTER ONLY ARMS AT >= 3 SOURCES, and that guard is load-bearing.
-- With 2 sources the median IS the midpoint, so both sources deviate from it by
-- exactly the same amount: any threshold either keeps both or drops BOTH, and
-- dropping both leaves the asset with no VWAP at all. With 1 source the median
-- is that source (deviation 0, always kept) — harmless but pointless. So the
-- filter is a no-op below 3 sources by construction, not by luck.
--
-- OUTLIER_PCT = 0.20 (20%). A starting value, deliberately loose. The asymmetry
-- matters: wrongly EXCLUDING a legitimate venue silently removes it from the
-- `sources` JSON and from the VWAP weighting (a visible, wrong answer), while
-- wrongly INCLUDING a mildly-divergent venue only nudges a volume-weighted mean
-- (a small, self-limiting error). Thin markets legitimately spread several
-- percent between venues, so a tight threshold costs more than it saves. Tune
-- against real multi-source assets — see task 0123's reconciliation.
--
-- NOTE: the §5.5 `min_volume_usd` inclusion threshold is deliberately NOT here;
-- it is task 0118, which layers it on top of this filter (cheap sources are
-- dropped BEFORE the median is taken, so they cannot skew it either).

DROP VIEW IF EXISTS prices.mv_current_prices;

CREATE MATERIALIZED VIEW prices.mv_current_prices
REFRESH EVERY 1 MINUTE
TO prices.current_prices
   (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct,
    volume_24h_usd, market_cap_usd, vwap_24h, sources, updated_at) AS
WITH
    -- XLM's own USD close, as a scalar. Resolved by natural key exactly the way
    -- the enrichment worker's resolve_reference_ids() does (ch_enrich.rs:447):
    -- XLM is the row with no issuer AND no contract address. 0 when unknown,
    -- which the nullIf() below turns into a NULL price_xlm → 0.
    (
        SELECT argMaxIf(close_usd, timestamp, close_usd > 0)
        FROM prices.price_ohlcv_1m FINAL
        WHERE asset_id = (
                  SELECT asset_id FROM prices.assets FINAL
                  WHERE asset_code = 'XLM'
                    AND issuer_address = ''
                    AND contract_address = ''
                  LIMIT 1
              )
          AND timestamp >= now() - INTERVAL 24 HOUR
    ) AS xlm_usd,

    -- Level 1 — one row per (asset, source) over the trailing 24h.
    --
    -- src_price is the latest PRICED close (task 0135, scope correction C2): a
    -- source whose newest 1m candle is not yet enriched carries its latest
    -- priced close and stays in `sources` and the vwap weighting, instead of
    -- reading 0 here and being dropped by `src_price > 0` below as a side
    -- effect of enrichment timing.
    --
    -- Freshness is measured but NOT yet applied — level 1b decides that, per
    -- asset. Both columns take the latest priced close; they differ only in
    -- whether a candle older than CARRY_BOUND may supply it, so for a source
    -- that IS fresh the two are identical.
    --
    -- CARRY_BOUND = 2h, derived from the enrichment SCHEDULE — `rate(1 hour)`
    -- in infra/envs/production.json, so two cycles, and a venue quoting
    -- normally survives one missed pass. Deliberately NOT fitted to an
    -- observed lag: as of 2026-08-20 enrichment was failing on every run
    -- (task 0215), so a measured figure would encode a broken pipeline.
    --
    -- Reference is `now()`, NOT the asset's own newest candle. An earlier
    -- revision compared against `max(timestamp)`, which spans quote legs
    -- enrichment can never price — for a venue trading continuously on such a
    -- leg that reference advances forever, the bound becomes unsatisfiable and
    -- the venue drops out with nothing wrong. `now()` asks the question this
    -- column actually needs answered: is the venue's last quote still current?
    per_source AS (
        SELECT
            asset_id                          AS asset_id,
            source                            AS source,
            argMaxIf(close_usd, timestamp, close_usd > 0) AS src_price,
            argMaxIf(close_usd, timestamp,
                     close_usd > 0 AND timestamp >= now() - INTERVAL 2 HOUR) AS src_price_fresh,
            sum(volume_quote_usd)             AS src_volume
        FROM prices.price_ohlcv_1m FINAL
        WHERE timestamp >= now() - INTERVAL 24 HOUR
        GROUP BY asset_id, source
    ),

    -- Level 1b — the bound is CONDITIONAL, and that is the whole design.
    --
    -- What it guards: a venue whose last quote is hours old still votes in the
    -- §5.5 median, which is UNWEIGHTED, so two stale venues can outvote and
    -- evict the one live venue (the 3-source case in PR #228's review). That
    -- defect needs a live venue to victimise.
    --
    -- So: drop stale venues ONLY when a fresh one survives. If every venue on
    -- an asset is stale there is nothing to defend, and dropping them all is
    -- pure loss — it empties `sources` and zeroes `vwap_24h` for an asset we
    -- have perfectly usable, if unfresh, prices for.
    --
    -- ⚠️ This is not a hypothetical refinement. The unconditional form shipped
    -- to prod on 2026-08-21 and was rolled back within the hour: measured
    -- against the same data, it blanked `sources` on **2,284 of 4,365 assets
    -- (52%)** while preventing **zero** evictions — 74% of the table had every
    -- venue stale, because enrichment had been down for two days (0215). The
    -- population where the defect can actually occur (>= 3 sources, mixed
    -- fresh/stale) was **7 assets**, and on all 7 the fresh-only median sat
    -- within 1% of the all-source median, far inside the 20% threshold.
    --
    -- Kept anyway, deliberately: that zero is not evidence of future safety.
    -- The mixed population is small precisely BECAUSE almost nothing is fresh
    -- right now; when 0215 and 0111 land, fresh venues multiply and the mixed
    -- population grows with them. The risk rises as the pipeline recovers.
    -- Conditional costs nothing by construction, so it is the cheap side of
    -- that bet.
    per_source_kept AS (
        SELECT
            asset_id,
            source,
            src_price,
            src_price_fresh,
            src_volume,
            max(src_price_fresh > 0) OVER (PARTITION BY asset_id) AS asset_has_fresh
        FROM per_source
        WHERE src_price > 0
    ),

    -- Level 2 — collapse sources into per-asset arrays so the median filter can
    -- run as an array pipeline. Decimal arrays are kept for the JSON (exact),
    -- Float64 arrays for the arithmetic (see numeric strategy above).
    per_asset AS (
        SELECT
            asset_id,
            groupArray(source)                    AS srcs,
            groupArray(src_price)                 AS prices_dec,
            groupArray(src_volume)                AS vols_dec,
            groupArray(toFloat64(src_price))      AS prices_f,
            groupArray(toFloat64(src_volume))     AS vols_f
        FROM per_source_kept
        WHERE NOT asset_has_fresh OR src_price_fresh > 0
                                        -- explicit rule (0135 C2 + the
                                        -- conditional bound): a source with NO
                                        -- priced candle in the window is
                                        -- already gone at level 1b; here a
                                        -- STALE source is dropped only when the
                                        -- asset still has a fresh one left
        GROUP BY asset_id
    ),

    -- Level 2b — the keep-mask. Below 3 sources the mask is all-true (see the
    -- guard note above); at >= 3 it is |p − median| / median <= OUTLIER_PCT.
    masked AS (
        SELECT
            asset_id,
            srcs,
            prices_dec,
            vols_dec,
            prices_f,
            vols_f,
            arrayReduce('median', prices_f) AS med,
            if(
                length(prices_f) >= 3 AND med > 0,
                arrayMap(p -> abs(p - med) / med <= 0.20, prices_f),
                arrayMap(p -> 1, prices_f)
            ) AS keep
        FROM per_asset
    ),

    -- Level 2c — apply the mask, then build the surviving aggregates.
    kept AS (
        SELECT
            asset_id,
            arrayFilter((s, k) -> k, srcs,       keep) AS k_srcs,
            arrayFilter((p, k) -> k, prices_dec, keep) AS k_prices_dec,
            arrayFilter((v, k) -> k, vols_dec,   keep) AS k_vols_dec,
            arrayFilter((p, k) -> k, prices_f,   keep) AS k_prices_f,
            arrayFilter((v, k) -> k, vols_f,     keep) AS k_vols_f
        FROM masked
    ),

    -- The 7-day reference close. Deliberately read from price_ohlcv_1h, NOT
    -- price_ohlcv_1m: cleanup-worker retains _1m for INTERVAL 7 DAY
    -- (cleanup-worker/src/lib.rs:22), so a 7-day-old row sits exactly on the
    -- retention floor — present or absent depending on where "today" falls in
    -- the month, and wholly unpredictable while cleanup is disabled for a
    -- backfill. _1h is kept forever (general-overview §3.6).
    -- close_usd > 0 excludes un-enriched rows, which would otherwise read as a
    -- -100% change. See task 0114: coarse close_usd was zero across most of
    -- history until the repair, so this guard is what keeps the column honest.
    --
    -- The window has BOTH ends: [now()-7d, now()-5d]. Without the upper cutoff
    -- argMin returns the oldest priced close AVAILABLE — for a freshly-listed
    -- asset (or one whose older 1h closes are still 0 from the pre-0114 gap)
    -- that can be hours old, and the column publishes e.g. a 2-hour move
    -- labelled as 7-day. With the cutoff, no baseline in the band means
    -- close_7d_ago stays 0 and the denominator guard lands change_7d_pct on
    -- the sentinel.
    --
    -- ⚠️ The cutoff BOUNDS the error, it does not remove it: a baseline sitting
    -- at the 5-day edge still measures a 5-day move published as 7-day, i.e.
    -- up to ~28% short on span. Naming the baseline's own timestamp in the
    -- response is the only real fix; that is a schema change and not this task.
    --
    -- ⚠️ SCOPE: this narrowing is a behaviour change beyond 0135's contract —
    -- an illiquid asset that trades most days but has no priced 1h candle in
    -- the [7d, 5d] band now publishes the sentinel where it previously
    -- published a (wrong-span) number. Taken deliberately, because 0135's own
    -- change removes the zero sentinel that was masking the defect for the 396
    -- assets task 0138 measured. Raised in review; revisit if the sentinel
    -- turns out to cost more than the mislabelled span did.
    ref_7d AS (
        SELECT
            asset_id                     AS asset_id,
            argMinIf(close_usd, timestamp, close_usd > 0) AS close_7d_ago
        FROM prices.price_ohlcv_1h FINAL
        WHERE timestamp >= now() - INTERVAL 7 DAY
          AND timestamp <= now() - INTERVAL 5 DAY
        GROUP BY asset_id
    ),

    -- Whole-asset figures deliberately NOT run through the §5.5 outlier mask:
    -- volume_24h_usd is an actual traded total, and price_usd is the newest
    -- priced close regardless of venue — the mask protects vwap_24h only, and
    -- whether price_usd should also be outlier-protected is task 0135's still
    -- open failure-mode-1 decision. open_24h is the oldest priced close inside
    -- the same window, which is what makes change_24h_pct a plain window read
    -- instead of a self-join.
    --
    -- price_usd is the latest PRICED close (task 0135 contract, decided
    -- 2026-08-05) and is deliberately NOT age-bounded, unlike per_source.
    --
    -- The asymmetry is the point. Publishing an old close for an asset that
    -- stopped trading is not a defect this task introduced — the pre-0135
    -- `argMax` published the same close. Bounding it here would be a new
    -- restriction on behaviour nobody reported, and it would trade a known
    -- price for the 0 sentinel, which is the outcome 0135 exists to remove:
    -- measured on prod 2026-08-20, 1,091 of 4,444 assets (24.5%) already
    -- publish a hard zero, and a consumer cannot tell "worthless" from "we do
    -- not know". What 0135 DID introduce is a stale venue voting in the
    -- median, and that is guarded one CTE up.
    --
    -- ⚠️ Honest consequence: this column can be older than it looks, and
    -- `updated_at` is the refresh time, not the price's age. No column carries
    -- that age today; publishing it is a follow-up task and is the real answer
    -- to "how fresh is this?" — not blanking a price we hold.
    --
    -- ⚠️ Any future guard here must emit a SENTINEL, never filter the row out.
    -- This MV is REPLACE, not APPEND (unlike the six rollup MVs), so
    -- current_prices becomes exactly what this SELECT returns — a filtered-out
    -- asset DISAPPEARS from the table rather than showing an empty price.
    --
    -- An asset with NO priced candle in the 24h window reads 0, the documented
    -- "unavailable" value.
    unfiltered AS (
        SELECT
            asset_id                          AS asset_id,
            argMaxIf(close_usd, timestamp, close_usd > 0) AS price_usd,
            argMinIf(close_usd, timestamp, close_usd > 0) AS open_24h,
            sum(volume_quote_usd)             AS volume_24h_usd
        FROM prices.price_ohlcv_1m FINAL
        WHERE timestamp >= now() - INTERVAL 24 HOUR
        GROUP BY asset_id
    )
SELECT
    u.asset_id                                              AS asset_id,
    u.price_usd                                             AS price_usd,

    -- price_xlm — price_usd ÷ XLM/USD. nullIf guards both a missing XLM market
    -- and a zero price; ifNull lands on the column's 0 = "unavailable" sentinel.
    toDecimal128(
        ifNull(toFloat64(u.price_usd) / nullIf(toFloat64(xlm_usd), 0), 0),
        14)                                                 AS price_xlm,

    -- change_24h_pct / change_7d_pct — Decimal(10,4), so ±999999.9999 is the
    -- representable band. A brand-new microcap can genuinely exceed that, and an
    -- overflow would poison the entire refresh, so both are clamped with least()
    -- /greatest() rather than left to throw.
    --
    -- ⚠️ BOTH operands are nullIf-guarded, not just the denominator (task 0138).
    -- Before the 0135 guard, `price_usd` was an UNFILTERED argMax while
    -- `open_24h` filtered `close_usd > 0`, so an un-enriched newest candle
    -- yielded a zero numerator beside a real denominator — and
    -- `(0 - open) / open * 100` is exactly **-100**, a fabricated total price
    -- collapse. Measured on ch-prod-01 2026-08-03: 889 of 4,442 assets (20%)
    -- published -100 on change_24h_pct, 396 on change_7d_pct, XLM among them,
    -- while their `vwap_24h` and `sources` carried the real price. -100 is NOT
    -- a sentinel — it passes every consumer-side "0 means unavailable" guard
    -- the views.sql interop contract documents, because it looks like data.
    -- With 0135's bounded argMaxIf the numerator is 0 when the window is
    -- unpriced OR the carry bound is exceeded. For change_24h_pct that case
    -- usually zeroes open_24h too (same table, same window), but NOT always —
    -- an over-bound asset has priced history and a real open_24h — and for
    -- change_7d_pct the denominator comes from a DIFFERENT table
    -- (price_ohlcv_1h), so a zero numerator beside a real 7d baseline is fully
    -- reachable. The numerator guards are LOAD-BEARING, not belt-and-braces:
    -- delete either and the fabricated -100% returns. Pinned by the
    -- change_7d-sentinel fixture in current_mv_it.rs.
    --
    -- A genuine -100% (a real price falling to a real near-zero) is unaffected:
    -- the guard keys on price_usd being exactly the 0 sentinel, not on the
    -- computed value.
    toDecimal64(
        greatest(-999999.0, least(999999.0,
            ifNull(
                (nullIf(toFloat64(u.price_usd), 0) - toFloat64(u.open_24h))
                    / nullIf(toFloat64(u.open_24h), 0) * 100,
                0))),
        4)                                                  AS change_24h_pct,
    toDecimal64(
        greatest(-999999.0, least(999999.0,
            ifNull(
                (nullIf(toFloat64(u.price_usd), 0) - toFloat64(r.close_7d_ago))
                    / nullIf(toFloat64(r.close_7d_ago), 0) * 100,
                0))),
        4)                                                  AS change_7d_pct,

    u.volume_24h_usd                                        AS volume_24h_usd,

    ifNull(
        accurateCastOrNull(
            toDecimal256(u.price_usd, 14)
                * toDecimal256(ifNull(sup.token_supply, 0), 14),
            'Decimal128(14)'),
        toDecimal128(0, 14))                                AS market_cap_usd,

    -- vwap_24h — volume-weighted across the SURVIVING sources only.
    toDecimal128(
        ifNull(
            arraySum(arrayMap((p, v) -> p * v, k.k_prices_f, k.k_vols_f))
                / nullIf(arraySum(k.k_vols_f), 0),
            0),
        14)                                                 AS vwap_24h,

    -- sources — {"sdex": {"price": "…", "volume_24h": "…"}, …} built from the
    -- NATIVE Decimal arrays via toString(), so no precision is lost. Excluded
    -- sources are absent, per general-overview §3.3.
    toJSONString(
        CAST(
            (
                k.k_srcs,
                arrayMap((p, v) -> map('price', toString(p), 'volume_24h', toString(v)),
                         k.k_prices_dec, k.k_vols_dec)
            ),
            'Map(String, Map(String, String))'
        )
    )                                                       AS sources,

    now()                                                   AS updated_at
FROM unfiltered AS u
LEFT JOIN kept   AS k   ON k.asset_id   = u.asset_id
LEFT JOIN ref_7d AS r   ON r.asset_id   = u.asset_id
LEFT JOIN (
    SELECT asset_id, token_supply FROM prices.asset_supply FINAL
) AS sup ON sup.asset_id = u.asset_id;
