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
-- Columns (0072 completed the original ten; 0178 appends `method` for eleven):
--   price_usd       — latest PRICED close in the 24h window (argMaxIf, 0135);
--                     NOT age-bounded — see the unfiltered CTE for why. The
--                     per-venue pipeline IS bounded, but conditionally: see 1b
--   price_xlm       — price_usd re-expressed in XLM (÷ the XLM/USD close)
--   change_24h_pct  — vs the oldest close inside the 24h window
--   change_7d_pct   — vs the oldest priced close in the [7d, 5d] band of
--                     price_ohlcv_1h (a real ~7-day baseline or the sentinel)
--   volume_24h_usd  — trailing-24h USD volume, ALL sources (a total, never
--                     filtered). Counts BOTH legs since 0178 — see `vol_all`
--   method          — price provenance: 'traded' / 'oracle' / '' (0178)
--   market_cap_usd  — price_usd × circulating supply from prices.asset_supply
--   vwap_24h        — USD volume-weighted close across sources, with the §5.5
--                     min_volume_usd threshold (task 0118) and inter-source
--                     median-outlier filter applied
--   sources         — JSON per-source {price, volume_24h}; sources excluded by
--                     min_volume_usd or outlier detection are ABSENT from the
--                     object (general-overview §3.3)
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
-- ⚠️ At >= 3 it can, however, clear EVERY source. `median` is `quantile(0.5)`
-- and INTERPOLATES on an even count, so a set straddling the midpoint by more
-- than OUTLIER_PCT leaves nothing behind. Verified on prod 26.3.10.60:
--
--   [1.00, 1.00, 3.00, 3.00] -> median 2.00 -> every element deviates 50%
--                            -> sources = '{}', vwap_24h = 0
--
-- while `price_usd` — venue-blind and taken from `unfiltered` — still
-- publishes 1.00. So an asset CAN carry a price with no sources beside it, and
-- the three-way coherence 0135 measures on prod (empty_sources = zero_price_usd
-- = zero_vwap, 259 each on 2026-08-25) is an observed property of the current
-- data, NOT an invariant of this SQL. Raised in the PR #241 review, finding 2,
-- against a PR-body claim of "by construction" that was too strong.
--
-- It matters more after 0135's C2 carry than before it: assets whose every
-- venue has an un-enriched tip used to arrive here with zero kept sources and
-- never armed the mask. They now arrive with their carried prices, so the
-- whole all-dead population — 65% of prod assets on 2026-08-25 — is newly
-- routed through it. Pinned by fixture asset 15; whether interpolation is the
-- right median for an even count is 0238's call, not this file's.
--
-- OUTLIER_PCT = 0.20 (20%). A starting value, deliberately loose. The asymmetry
-- matters: wrongly EXCLUDING a legitimate venue silently removes it from the
-- `sources` JSON and from the VWAP weighting (a visible, wrong answer), while
-- wrongly INCLUDING a mildly-divergent venue only nudges a volume-weighted mean
-- (a small, self-limiting error). Thin markets legitimately spread several
-- percent between venues, so a tight threshold costs more than it saves. Tune
-- against real multi-source assets — see task 0123's reconciliation.
--
-- ── §5.5 min_volume_usd inclusion threshold (task 0118) ─────────────────────
-- "Only include sources where volume_24h > configurable_min_threshold_usd
-- (e.g. $100)." MIN_VOLUME_USD = 100, the spec's own worked example. A literal
-- in the WHERE below, not a settings table — the MV is redeployed by DDL
-- anyway, so a second indirection buys nothing. The API-side `?min_volume_usd=`
-- override (same task) re-weights from the `sources` JSON in the handler; this
-- MV always computes at the system default.
--
-- ⚠️ THE THRESHOLD IS CONDITIONAL, exactly like the liveness bound one CTE
-- down, and for the same measured reason. A below-threshold source is dropped
-- ONLY when the asset still has a source ABOVE the threshold. The defect this
-- rule closes — dust venues skewing the median and the weighting — needs a
-- funded venue to victimise; on an asset whose every venue is dust there is
-- nothing to defend, and dropping them all is pure loss. Measured on prod
-- 2026-08-27 before rollout: the unconditional form would have blanked
-- vwap_24h/sources on 2,960 of 3,068 priced assets (96.5%; ~85% of the table
-- has a max per-venue 24h volume of <= $1) while the largest casualty traded
-- $124/day — the same shape as the 2026-08-21 liveness-guard rollback, found
-- by measurement instead of by incident this time. Decided with the team
-- 2026-08-27; the spec's $100 is an "e.g.", the conditional form is ours,
-- recorded in the task's Design Decisions.
--
-- ORDER MATTERS, twice:
--   * threshold BEFORE the median — a dust venue must not be able to skew the
--     vote it is not allowed to weight in;
--   * threshold BEFORE the liveness window: `asset_has_live` is computed over
--     the threshold's SURVIVORS (see per_source_funded), so the guard cannot
--     defend a venue the threshold is about to erase. Concretely: a live $50
--     venue beside a stale $10k venue must NOT evict the $10k one and then
--     vanish itself — with the threshold first, the asset counts as all-quiet
--     and keeps the stale-but-real price.
--
-- The threshold is a WEIGHTING rule only: `price_usd` and `volume_24h_usd`
-- read from `unfiltered` and are untouched by construction.
--
-- NOTE the deliberate asymmetry with the API override: an EXPLICIT
-- `?min_volume_usd=` filters strictly (the caller asked for exactly that);
-- only this MV's ambient system default is conditional.

DROP VIEW IF EXISTS prices.mv_current_prices;

CREATE MATERIALIZED VIEW prices.mv_current_prices
REFRESH EVERY 1 MINUTE
TO prices.current_prices
   (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct,
    volume_24h_usd, market_cap_usd, vwap_24h, sources, updated_at, method) AS
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

    -- ── Task 0178: the canonical-USDC identity and its measured USD rate ─────
    -- USDC NEVER trades as a base leg (0 candles, by construction — see
    -- views.sql's peg-set block), so it is absent from every base-keyed CTE
    -- below and `/price` returned 404 for it. These two scalars are what let
    -- the `usdc_tip` arm synthesise its row.
    --
    -- 🔴 ALLOWLISTED TO USDC BY NAME. Do NOT generalise this to "the peg set",
    -- to "stablecoins", or to "any asset with a usd_rate row". Reflector prices
    -- the TICKER "USDT" — Tether's own token, genuinely at par — and we file
    -- that rate under the Stellar issuer's address, whose token really is worth
    -- ~$0.13 since it depegged in June 2022 (task 0172, not a defect). Widening
    -- this lookup would publish ~$1.00 for that identity tagged 'oracle', a
    -- label that reads as MORE authoritative than the guess it replaced —
    -- strictly worse than the bug this task fixes. The symbol→issuer mapping
    -- that makes widening safe is TASK 0173. Same fence as views.sql:102-107.
    -- ⚠️ This address is duplicated from prices_clickhouse::USDC_ISSUER (Rust),
    -- which is the single source of truth; a SQL literal cannot reference it.
    -- views.sql carries the same copy under the same rule — keep all three in
    -- step.
    (
        SELECT asset_id FROM prices.assets FINAL
        WHERE asset_code = 'USDC'
          AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
          AND contract_address = ''
        LIMIT 1
    ) AS usdc_asset_id,

    -- The rate itself: ASOF at-or-before `now()`, never averaged (task 0167's
    -- resolution rule), and REFUSED past a staleness window — an unbounded
    -- forward-fill would present a three-day-old reading as the live price.
    --
    -- `method = 'oracle'` selects a MEASURED reading. usd_rate keys on
    -- (identity, timestamp, method) precisely so a 0154 'pivot' row cannot
    -- silently replace a measurement; the consumer chooses, and this consumer
    -- chooses measured or nothing.
    --
    -- 24 HOUR matches this MV's own window. If the oracle goes quiet for longer
    -- the scalar is 0, `usdc_tip` emits NO ROW, and behaviour degrades to
    -- exactly what it is today (USDC absent from the table) rather than to a
    -- confident zero. Absent is honest; 0 tagged 'oracle' would not be.
    (
        SELECT argMax(usd_rate, timestamp)
        FROM prices.usd_rate FINAL
        WHERE asset_kind = 'credit'
          AND asset_code = 'USDC'
          AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
          AND contract_address = ''
          AND method = 'oracle'
          AND timestamp <= now()
          AND timestamp >= now() - INTERVAL 24 HOUR
    ) AS usdc_rate,

    -- Level 1 — one row per (asset, source) over the trailing 24h.
    --
    -- src_price is the latest PRICED close (task 0135, scope correction C2): a
    -- source whose newest 1m candle is not yet enriched carries its latest
    -- priced close and stays in `sources` and the vwap weighting, instead of
    -- reading 0 here and being dropped by `src_price > 0` below as a side
    -- effect of enrichment timing.
    --
    -- Liveness is measured but NOT yet applied — level 1b decides that, per
    -- asset. Note the two columns read DIFFERENT things on purpose: src_price
    -- is a price and skips un-enriched candles; src_is_live is a quoting
    -- signal and counts them.
    --
    -- CARRY_BOUND = 2h, derived from the enrichment SCHEDULE — `rate(1 hour)`
    -- in infra/envs/production.json, so two cycles, and a venue quoting
    -- normally survives one missed pass. Deliberately NOT fitted to an
    -- observed lag: as of 2026-08-20 enrichment was failing on every run
    -- (task 0215), so a measured figure would encode a broken pipeline.
    --
    -- Reference is `now()`, NOT the asset's own newest candle. An earlier
    -- revision compared the venue against `max(timestamp)` ACROSS the asset,
    -- which spans quote legs enrichment can never price — for a venue trading
    -- continuously on such a leg that reference advances forever, the bound
    -- becomes unsatisfiable and the venue drops out with nothing wrong. Below,
    -- `max(timestamp)` is the venue's OWN newest candle against the wall
    -- clock, which is a different question and has no such failure mode.
    --
    -- ⚠️ src_is_live tests the venue's newest CANDLE, and there is deliberately
    -- no `close_usd > 0` in that predicate. It asks "is this venue still
    -- QUOTING?", which is not the same question as "has enrichment reached
    -- this venue lately?" (PR #241 review, finding 1). An earlier revision
    -- tested the newest *priced* close and so conflated a dead venue with one
    -- enrichment simply had not caught up to — reproducing, one level down,
    -- the exact defect C2 above exists to prevent. Measured on the review's
    -- probe: a venue quoting 1 min ago carrying $1,000,000 of volume, last
    -- enriched 3 h ago, was evicted in favour of one carrying $10 that
    -- happened to be enriched 5 min ago — publishing a `vwap_24h` drawn from
    -- 0.001% of the `volume_24h_usd` the same row reports. Liveness is a
    -- property of quoting, so it is measured on candles.
    per_source AS (
        SELECT
            asset_id                          AS asset_id,
            source                            AS source,
            argMaxIf(close_usd, timestamp, close_usd > 0) AS src_price,
            max(timestamp) >= now() - INTERVAL 2 HOUR     AS src_is_live,
            sum(volume_quote_usd)             AS src_volume
        FROM prices.price_ohlcv_1m FINAL
        WHERE timestamp >= now() - INTERVAL 24 HOUR
        GROUP BY asset_id, source
    ),

    -- Level 1b — the bound is CONDITIONAL, and that is the whole design.
    --
    -- What it guards: a venue that stopped quoting hours ago still votes in
    -- the §5.5 median, which is UNWEIGHTED, so two dead venues can outvote and
    -- evict the one live venue (the 3-source case in PR #228's review). That
    -- defect needs a live venue to victimise.
    --
    -- So: drop dead venues ONLY when a live one survives. If every venue on an
    -- asset is dead there is nothing to defend, and dropping them all is pure
    -- loss — it empties `sources` and zeroes `vwap_24h` for an asset we have
    -- perfectly usable, if stale, prices for.
    --
    -- ⚠️ This is not a hypothetical refinement. The unconditional form shipped
    -- to prod on 2026-08-21 and was rolled back within the hour: measured
    -- against the same data, it blanked `sources` on **2,284 of 4,365 assets
    -- (52%)** while preventing **zero** evictions — 74% of the table had every
    -- venue stale, because enrichment had been down for two days (0215).
    --
    -- Sizing, measured twice, and NOT trending the way an earlier revision of
    -- this comment predicted:
    --
    --                          pipeline down (08-21)   healthy (08-25)
    --   mixed live/dead                  35                  15
    --     ...and >= 3 sources             7                   1   <- at-risk
    --
    -- That revision argued the mixed population would GROW as enrichment
    -- recovered, so the guard would be worth more over time. Retracted: it
    -- shrank by more than half. Assets move wholesale from all-dead to
    -- all-live rather than through a mixed state — a venue that trades gets a
    -- fresh candle, one that does not, does not. The mix is a transient, and a
    -- consistent pipeline produces fewer of them.
    --
    -- Kept on the corrected argument: the guard's value is ANTI-correlated
    -- with pipeline health. At 1 at-risk asset it prevents nothing measurable
    -- today; it was worth 7 during an outage that ran 26 days before 0144
    -- noticed it. The defect it blocks is silent and indistinguishable from a
    -- real price downstream, and the conditional form costs nothing by
    -- construction. Removing it is a two-line revert that changes none of the
    -- measured gains, which come from C2's carry rather than from the guard.
    -- Population filter plus the CONDITIONAL §5.5 threshold's window (task
    -- 0118, header note): `asset_has_funded` asks "does any priced venue on
    -- this asset clear MIN_VOLUME_USD?". Strict >, per the spec's
    -- "volume_24h > threshold".
    per_source_kept AS (
        SELECT
            asset_id,
            source,
            src_price,
            src_volume,
            src_is_live,
            max(src_volume > 100) OVER (PARTITION BY asset_id) AS asset_has_funded
        FROM per_source
        WHERE src_price > 0
    ),

    -- Apply the threshold, THEN compute the liveness window over its
    -- survivors. The two-stage order is load-bearing: a dust venue must not
    -- be able to arm the liveness guard and evict a funded-but-stale venue
    -- before vanishing itself (the THL fixture in current_mv_it.rs).
    per_source_funded AS (
        SELECT
            asset_id,
            source,
            src_price,
            src_volume,
            src_is_live,
            max(src_is_live) OVER (PARTITION BY asset_id) AS asset_has_live
        FROM per_source_kept
        WHERE NOT asset_has_funded OR src_volume > 100
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
        FROM per_source_funded
        WHERE NOT asset_has_live OR src_is_live
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
    -- ⚠️ TASK 0178 SPLIT THIS CTE IN TWO, and the split is load-bearing.
    --
    -- It used to compute price_usd, open_24h AND volume_24h_usd in one pass
    -- keyed on the BASE leg. Volume now counts both legs (see `vol_all`), but
    -- price MUST NOT: `close_usd` is the USD price of the row's BASE asset, so
    -- feeding quote-leg rows into these argMax/argMin would hand every quote
    -- asset the price of whatever it was traded against — USDC would inherit
    -- XLM's price. Price and open stay strictly base-keyed. Do not re-merge.
    base_tip AS (
        SELECT
            asset_id                          AS asset_id,
            argMaxIf(close_usd, timestamp, close_usd > 0) AS price_usd,
            argMinIf(close_usd, timestamp, close_usd > 0) AS open_24h,
            toUInt8(0)                        AS is_oracle
        FROM prices.price_ohlcv_1m FINAL
        WHERE timestamp >= now() - INTERVAL 24 HOUR
        GROUP BY asset_id
    ),

    -- The synthesised canonical-USDC row (task 0178). Emits AT MOST one row,
    -- and none at all when the rate is stale or the identity is unknown.
    --
    -- `open_24h` is deliberately the 0 sentinel, NOT the rate: it feeds
    -- change_24h_pct, and there is no measured 24h-ago baseline for an asset
    -- that does not trade as a base. A zero denominator routes through the
    -- existing nullIf guard to the documented 0 sentinel, which is the whole
    -- point — a fabricated `change_24h_pct` beside a real price would be a new
    -- instance of the -100% defect task 0138 removed. NEVER set this to
    -- `usdc_rate` to make the arithmetic "work": that publishes a measured 0%
    -- change that was never measured.
    --
    -- The NOT IN guard keeps this arm mutually exclusive with base_tip. USDC
    -- has no base candles today, but if it ever gains one the union would emit
    -- the asset twice and the ReplacingMergeTree (ORDER BY asset_id) would keep
    -- whichever row merged last — a silent coin-flip between a measured rate
    -- and a traded close. Cheap guard, unbounded downside without it.
    usdc_tip AS (
        SELECT
            usdc_asset_id                     AS asset_id,
            usdc_rate                         AS price_usd,
            toDecimal128(0, 14)               AS open_24h,
            toUInt8(1)                        AS is_oracle
        WHERE usdc_asset_id > 0
          AND usdc_rate > 0
          AND usdc_asset_id NOT IN (SELECT asset_id FROM base_tip)
    ),

    unfiltered AS (
        SELECT asset_id, price_usd, open_24h, is_oracle FROM base_tip
        UNION ALL
        SELECT asset_id, price_usd, open_24h, is_oracle FROM usdc_tip
    ),

    -- ── volume_24h_usd counts BOTH legs (task 0178) ──────────────────────────
    -- `volume_quote_usd` is the USD value of the trade, and that same dollar
    -- figure is true of both sides of it: an XLM/USDC trade moves $5,000 of XLM
    -- AND $5,000 of USDC. Summing the asset over both legs therefore reads
    -- "total 24h USD volume this asset participated in", which is the ordinary
    -- meaning of per-asset volume on a price API.
    --
    -- Before this, the sum grouped on the base leg alone, so canonical USDC —
    -- which never IS a base leg — summed an empty set and published 0. The
    -- arithmetic was never wrong; there was simply nothing to add up.
    --
    -- ⚠️ ONE RULE FOR EVERY ASSET, deliberately. XLM and USDT trade heavily on
    -- both sides and their reported volume ROSE when this shipped. That is the
    -- intended consequence, not drift: a rule that counted both legs for USDC
    -- only would make the column mean different things in different rows — the
    -- 0144 defect class. Two visible effects, both signed off 2026-08-31:
    --   * `GET /assets` sorts on this column BY DEFAULT
    --     (prices-api handlers.rs, SortCol::Volume24h), so the first page of
    --     the listing reorders and USDC rises into it;
    --   * one trade counts toward BOTH of its assets. Standard for per-asset
    --     volume, and not double counting within any single asset.
    --
    -- ⚠️ Bounded by task 0242: a SAC minted as a second identity holds the same
    -- token under two asset_ids, so for those 11 assets the total is still
    -- split across both rows whichever legs are counted. USDC is NOT among
    -- them; do not claim this column is correct for those identities.
    --
    -- NOT run through the §5.5 outlier mask, unchanged from before: this is an
    -- actual traded total, and the mask protects vwap_24h only.
    vol_all AS (
        SELECT
            asset_id                          AS asset_id,
            sum(volume_quote_usd)             AS volume_24h_usd
        FROM
        (
            SELECT asset_id AS asset_id, volume_quote_usd AS volume_quote_usd
            FROM prices.price_ohlcv_1m FINAL
            WHERE timestamp >= now() - INTERVAL 24 HOUR
            UNION ALL
            SELECT quote_asset_id AS asset_id, volume_quote_usd AS volume_quote_usd
            FROM prices.price_ohlcv_1m FINAL
            WHERE timestamp >= now() - INTERVAL 24 HOUR
        )
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
    -- With 0135's guarded argMaxIf the numerator is 0 when the window holds no
    -- priced candle at all — and ONLY then. It is not subject to the liveness
    -- bound: the numerator reads `unfiltered.price_usd`, which is deliberately
    -- venue-blind and unbounded, while the bound lives in `per_source` and has
    -- never touched this projection. An earlier revision of this comment said
    -- "or the carry bound is exceeded", which was never true and became more
    -- misleading once the bound stopped being an argMaxIf predicate at all
    -- (PR #241 review, finding 6). For change_24h_pct the unpriced case
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

    -- volume_24h_usd — from vol_all (BOTH legs, task 0178), not from u.
    -- ⚠️ prod runs join_use_nulls = 0, so an unmatched LEFT JOIN yields the
    -- column DEFAULT (0), never NULL. That is the right sentinel here and needs
    -- no ifNull — but it is also why no `IS NOT NULL` test may be written
    -- against this join: it would be dead code.
    v.volume_24h_usd                                        AS volume_24h_usd,

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

    now()                                                   AS updated_at,

    -- method — price provenance (task 0178). APPENDED LAST, and it must occupy
    -- the same position in the TO(...) list above: this MV inserts POSITIONALLY
    -- and a mismatch writes every column into the wrong slot. See the
    -- POSITIONAL-INSERT FOOTGUN note in this file's header.
    --
    -- '' is the unavailable SENTINEL, not a vocabulary word — an asset with no
    -- priced candle in the window has `price_usd = 0` and no method applies.
    -- Labelling that 'traded' would assert an aggregate that does not exist,
    -- which is the very ambiguity this column was added to remove. Full
    -- vocabulary and the usd_rate.method distinction: init.sql's block on
    -- prices.current_prices.
    -- ⚠️ Keys on WHICH ARM produced the row (`is_oracle`), never on the asset's
    -- identity. An earlier revision tested `u.asset_id = usdc_asset_id` and was
    -- caught by the duplicate-guard fixture: the moment USDC gains a base
    -- candle, `usdc_tip` is correctly suppressed and base_tip supplies a real
    -- TRADED close — which that expression still tagged 'oracle'. A traded
    -- price wearing an oracle label is worse than either alone, because
    -- 'oracle' reads as the more authoritative of the two.
    CAST(
        multiIf(
            u.is_oracle = 1, 'oracle',
            u.price_usd > 0, 'traded',
            ''),
        'LowCardinality(String)')                           AS method
FROM unfiltered AS u
LEFT JOIN vol_all AS v  ON v.asset_id   = u.asset_id
LEFT JOIN kept   AS k   ON k.asset_id   = u.asset_id
LEFT JOIN ref_7d AS r   ON r.asset_id   = u.asset_id
LEFT JOIN (
    SELECT asset_id, token_supply FROM prices.asset_supply FINAL
) AS sup ON sup.asset_id = u.asset_id;
