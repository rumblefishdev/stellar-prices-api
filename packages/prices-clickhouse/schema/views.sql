-- prices read-surface VIEWS (task 0061 Step 5 — the USD-close series + the USD
-- reference companion). Applied after init.sql tables; plain views (no special
-- ClickHouse version needed, unlike the refreshable rollup MVs).
--
-- Applier contract (same as init.sql): the Rust splitter splits naively on `;`
-- and strips `-- …` line comments only. Keep this file free of inline string
-- literals containing `;` and of block comments.
--
-- ## Statement form: CREATE OR REPLACE VIEW, never CREATE VIEW IF NOT EXISTS
-- Every view in this file MUST use `CREATE OR REPLACE VIEW` (task 0134; enforced
-- by `views_sql_uses_create_or_replace_for_every_view` in src/lib.rs).
--
-- `CREATE VIEW IF NOT EXISTS` does NOT redefine a view that already exists: on a
-- provisioned target — i.e. ch-prod-01 — editing a body here and re-applying
-- SILENTLY NO-OPS. The apply reports success, the definition does not change,
-- and nothing surfaces the divergence between this repo and the live cluster.
-- The failure is invisible, which is the expensive part; task 0072 hit it on
-- current_price_usd, where the six-column v1 view survived an apply that should
-- have taken it to 13.
--
-- Plain views replace ATOMICALLY, so there is no DROP window and no read-side
-- exposure. This is NOT true of the refreshable MVs in current.sql / rollups.sql,
-- which genuinely require DROP + re-CREATE — leave those on their own form.
--
-- ## ⚠️ This file requires a PRIVILEGED applier (task 0134, decision: option 2)
-- `CREATE OR REPLACE VIEW` needs a `DROP VIEW` grant on CH 26.3.10.60 —
-- UNCONDITIONALLY, even when the view does not exist yet. Otherwise:
--   Code: 497. DB::Exception: … Not enough privileges.
--   (Missing permissions: DROP VIEW ON prices.current_price_usd)
--
-- The scoped runtime users CANNOT apply this file, by design, and never could:
-- measured on ch-prod-01 2026-07-30, `prices_writer` holds only SELECT / INSERT /
-- ALTER DELETE / OPTIMIZE on prices.* and `prices_reader` only SELECT — neither
-- has DROP VIEW, CREATE VIEW, CREATE TABLE or CREATE DATABASE. They are
-- XML-managed in BE's `services.xml` and cannot be SQL-GRANTed by us.
--
-- Schema DDL on ch-prod-01 is therefore an OPERATOR action as the container's
-- `default` user over the loopback native port, which bypasses Caddy and the
-- mTLS CN map entirely:
--   docker exec -i app-clickhouse-1 clickhouse-client   (no --user)
-- Do not add this file to a scoped-user apply path; requesting a broad DDL grant
-- for the ingestion writer was considered and rejected.
--
-- ## Design references
--   - R-historical-usd-close-design.md §8 (view), §12.2 (natural-identity key),
--     §12.3 (NULL + status discriminator + usd_reference companion)
--   - close_usd is BAKED into the candles at enrichment time (§12.1); these views
--     read that column — they do NOT join the retention-capped oracle_prices.
--
-- ## USDC issuer literal (load-bearing)
-- The USDC issuer address below is the same join key the Rust paths use; the
-- single source of truth is `prices_clickhouse::USDC_ISSUER` (re-exported to the
-- backfill + enrichment crates). SQL cannot reference a Rust const, so this
-- literal is a hand-synced copy — if the canonical address ever changes, update
-- it here AND in that const together, or the views and the writer diverge.
-- `prices_clickhouse::USDT_ISSUER` is NO LONGER referenced by these views —
-- task 0172 removed USDT from the peg-fill arm (see below). The const still
-- exists for the writer paths; do not re-add it here.
--
-- ## Peg assets cannot be priced as a base — the peg-fill arm (task 0165)
-- USDC is our top-preference QUOTE, so canonicalisation makes it the quote on
-- essentially every pair it appears in and it is the base of almost nothing.
-- A view that emits one row per BASE asset therefore cannot publish USDC at all:
-- measured on prod, `price_ohlcv_1d WHERE asset_id = <USDC>` returns 0 candles,
-- and 0 of BE's 1,433 USDC-legged pools were priceable in any window (67.8% of
-- every never-priced pool they hold). The control that proves the mechanism is
-- the issuer split: USDC at the canonical issuer is 0/1,433 priceable, USDC at
-- 56 OTHER issuers is 228/233 (97.9%) — same asset code, quote preference the
-- sole variable. It tracks preference, not peg status or asset class.
--
-- price_usd_series* therefore UNION a zero-weight placeholder row per peg asset
-- per bucket, keyed on the QUOTE leg, BEFORE the aggregation. Precedence then
-- falls out of the arithmetic rather than being coded as a rule — see the block
-- comment above price_usd_series for the case table and the three shapes that
-- look simpler and are wrong.
--
-- ⚠️ The `1` is a PLACEHOLDER, not the answer — task 0168 replaces it.
-- oracle_worker already polls Reflector for USDC, so a real depeg-aware rate
-- exists; task 0167 snapshots it into prices.usd_rate and 0168 swaps this
-- constant for it. A flat $1 is a ~0.1% systematic error (small depegs are
-- routine) and it CONTRADICTS OUR OWN CANDLES: the oracle enrichment tier
-- already prices a TF/USDC candle at `close × 0.9993` (ch_enrich.rs:20).
-- Read `method = 'peg'` as "no measured rate was available", never as "$1 is
-- correct" — that is exactly what the `method` column exists to disambiguate.
--
-- ## Why USDC is the ONLY member of the peg set (task 0172)
-- USDT was here until 2026-08-12. The canonical Stellar USDT
-- (GCQTGZQQ…TG6V) DEPEGGED IN JUNE 2022 and has traded at a deep discount ever
-- since — ~$0.13 through 2026-08. That is not a data defect: two markets sharing
-- no legs and no code path agree to within a cent (its own USDC pair, and
-- XLM/USDC ÷ XLM/USDT), four sibling stablecoins held par through the same
-- window in the same pipeline, and trade_count collapsed 140,945 → 805/month as
-- liquidity fled. Pegging it to $1 overstated close_usd by ~7.4x on 44,657
-- candles across 495 base assets.
--
-- The two members were never the same kind of thing, which is why only one was
-- removed: USDC NEVER trades as a base (0 candles, by construction — see above),
-- so the placeholder is the only way it is priced at all. USDT has 2,011
-- effectively gapless daily candles since 2021-02-07, so it needs no placeholder
-- — it is priced by measurement, via the enrichment pivot tier that already
-- prices XLM-quoted candles (ch_enrich.rs, ReferenceIds::pivot_ids).
--
-- ⚠️ Do NOT restore USDT here by pointing 0168 at the oracle. Reflector prices
-- the TICKER "USDT" — Tether's own token, genuinely at par — and we file that
-- rate under this issuer's address, so prices.usd_rate asserts ~$1.00 for an
-- asset worth $0.13. Shipping 0168 for this identity would relabel the same 7.4x
-- error as `method = 'oracle'`, which reads as MORE authoritative. The
-- symbol→issuer mapping is task 0173.
--
-- ## Public key = natural Stellar identity, never asset_id (§12.2)
-- asset_id is an internal UInt32 surrogate. These views resolve it to the
-- portable identity via prices.assets and expose:
--   asset_kind ∈ ('native','credit','contract'),
--   asset_code, issuer_address, contract_address.
-- native XLM → ('native','XLM','',''); classic → ('credit', code, issuer, '');
-- SAC / Soroban token → ('contract','','', contract_address).
-- The 'contract' kind is normalized at read time: asset_code and issuer_address
-- are forced to '' (via if(contract_address != '', '', …)), so the
-- (contract ⇒ asset_code='') interop contract holds even if discovery/metadata
-- ever populates a symbol into a Soroban token's asset_code — the view does not
-- depend on the writer keeping it blank.
--
-- ## Grain variants (1h + 1d)
-- The series + reference are provided at two grains, both on forever-retained
-- OHLCV tables (1h and 1d carry close_usd via the rollup chain):
--   prices.price_usd_series      / prices.usd_reference       — daily buckets
--   prices.price_usd_series_1h   / prices.usd_reference_1h     — hourly buckets
-- Pair a series with its same-grain reference when classifying status. Hourly
-- serves read-time TVL keyed to a ledger's closed_at without collapsing a whole
-- day to one close; daily is cheaper for long-range charts.
--
-- ## Grain-selection ownership (decided 2026-06-15)
-- At the VIEW layer, grain selection is the CALLER's: the consumer JOINs whichever
-- grain its query needs (one consistent grain per chart → no resolution
-- discontinuities; keeps these views a dumb, fast, retention-agnostic data
-- surface). The "finest-retained-for-T" routing (view-picks) is deliberately NOT
-- in the views — it belongs to the point-lookup HTTP endpoint (task 0040,
-- `price_usd_at(id, ts)`), the natural home for that policy. So: views =
-- caller-passes; the 0040 API primitive = view-picks. Confirm with BE that they
-- own grain choice at the JOIN layer.
--
-- ## Read-time status discriminator (§12.3) — computed by the reader
-- A view cannot enumerate (asset × bucket) combinations that never traded, so
-- `no_asset_price` is a read-time condition, not a stored column. For a lookup
-- of (identity I, bucket T), LEFT JOIN price_usd_series against usd_reference
-- (at the matching grain):
--
--   status = ok             -- row present in price_usd_series for (I, T)
--          | no_asset_price  -- (I, T) absent, BUT usd_reference has bucket T
--                            --   (the USD reference IS up; partial TVL is valid)
--          | no_reference    -- (I, T) absent AND usd_reference has no bucket T
--                            --   (systemic blackout — every XLM-pivot asset NULL)
--
-- close_usd is always returned as a value-or-absent: a miss is a missing row
-- (NULL after the reader's LEFT JOIN), never an error and never a dropped row.
--
-- ## JOIN interop contract (exact column forms — avoids silent JOIN mismatch)
--   asset_kind       String, one of 'native' / 'credit' / 'contract'.
--   asset_code       trimmed String (e.g. 'XLM','USDC') — NOT a padded
--                    FixedString; '' for native and for pure Soroban tokens.
--   issuer_address   String G-strkey; '' for native and Soroban tokens.
--   contract_address String C-strkey; '' for native/classic.
--   bucket           DateTime, already floored to the grain. Join hourly on
--                    toStartOfHour(closed_at), daily on toStartOfDay(closed_at).
--   close_usd / price_usd  Decimal(38, 14).
-- native XLM key is ('native','XLM','',''). One canonical XLM row (the native
-- identity); the writer's stored asset_type='classic' is mapped to 'native' here.
--
-- ### price_usd_series* only — `method` (task 0165, APPENDED LAST)
--   method  LowCardinality(String). How this row's close_usd was arrived at:
--             'traded' — a volume-weighted aggregate of candles that some
--                        pricing tier already priced. The view cannot know
--                        WHICH tier, which is why this is a distinct value and
--                        not a reuse of task 0167's rate-provenance enum.
--             'peg'    — no traded candle existed for this identity in this
--                        bucket; the peg placeholder supplied the value. Read
--                        it as "no measured rate available", NOT as "$1 is
--                        correct".
--             'oracle' — RESERVED for task 0168, once a measured depeg-aware
--                        rate replaces the placeholder.
--   Without this column a consumer cannot tell a real 1.0000 from a fallback
--   1.0000 — the `close_usd = 0` mistake (one value meaning several things) in
--   a new surface. ⚠️ This is an APPENDED column: arity changed, order did not.
--   Anything decoding POSITIONALLY off `SELECT *` gets an extra column; pin an
--   explicit column list.
--
-- ### current_price_usd only (task 0072)
-- These carry SENTINELS, not NULLs — `current_prices`' columns are
-- non-nullable, so "unavailable" and a real value share a type and can only be
-- told apart by value. That is a weaker contract than the value-or-absent one
-- above, and consumers have to handle it explicitly:
--   price_usd        Decimal(38,14). Since task 0135 this is the latest
--                    PRICED close in the 24h window, NOT age-bounded: for an
--                    asset that stopped trading it is simply its last priced
--                    close, and updated_at (refresh time) is NOT a price-age
--                    signal. No column carries that age yet. 0 = no priced
--                    candle in the window (an un-enriched tip alone no longer
--                    produces 0).
--   price_xlm        Decimal(38,14). 0 = unavailable (no XLM market, or
--                    price_usd on its 0 sentinel per the 0135 rule above) —
--                    indistinguishable from a true 0. An un-enriched tip by
--                    itself no longer zeroes it.
--   change_24h_pct / change_7d_pct
--                    Decimal(10,4) percent, clamped to ±999999.9999 (an
--                    overflow would poison the whole MV refresh). 0 =
--                    unavailable AND 0 = a genuinely flat 24h/7d; the two are
--                    NOT distinguishable. Treat 0 as "no signal".
--                    change_7d_pct's baseline is the oldest priced 1h close in
--                    the [7d, 5d] band — no baseline there means the sentinel,
--                    never a shorter-span move mislabelled as 7d.
--   volume_24h_usd / market_cap_usd / vwap_24h
--                    Decimal(38,14). 0 = unavailable. market_cap_usd is 0
--                    whenever circulating supply is absent (best-effort join);
--                    it multiplies price_usd and inherits its (unbounded) age.
--                    vwap_24h is different: per-source closes ARE age-bounded
--                    (2h), because a venue in `sources` asserts "quoting now"
--                    — so an asset can legitimately show a price_usd with an
--                    empty sources/zero vwap when no venue is currently live.
--   sources          String holding a JSON object — NOT a JSON-typed column.
--                    THREE states, and '' is the trap:
--                      ''   — the MV has never rewritten this row (table
--                             DEFAULT). NOT VALID JSON; a parser will throw.
--                      '{}' — refreshed, but no source had a priced 24h candle
--                             or survived the §5.5 outlier filter.
--                      '{"sdex":{"price":"…","volume_24h":"…"}, …}' — populated.
--                    Numbers are serialised as STRINGS to preserve
--                    Decimal(38,14) precision (general-overview §3.3).
--                    Outlier-excluded venues are ABSENT from the object, so the
--                    volumes here can sum to LESS than volume_24h_usd (a total
--                    across all sources) — that asymmetry is intentional.
--                    Guard the '' case explicitly; our own API does, in
--                    `prices-api/src/assets/dto.rs::parse_sources`.

----------------------------------------------------------------------
-- prices.usd_reference — per-bucket USD reference availability.
-- The volume-weighted XLM/USDC close (= XLM's USD price under the USDC≡$1 peg)
-- per day bucket. A bucket's PRESENCE is the durable "USD reference is up at T"
-- signal the reader LEFT JOINs for systemic-blackout detection. Identity-resolved
-- (not asset_id) so it survives asset-id reassignment. Reads `close` (always
-- present from the backfill), independent of enrichment timing.
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.usd_reference AS
SELECT
    p.timestamp AS bucket,
    CAST(sum(toFloat64(p.close) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS xlm_usd
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS base  FINAL ON base.asset_id  = p.asset_id
INNER JOIN prices.assets AS quote FINAL ON quote.asset_id = p.quote_asset_id
WHERE base.asset_code = 'XLM' AND base.issuer_address = '' AND base.contract_address = ''
  AND quote.asset_code = 'USDC'
  AND quote.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND p.close > 0
GROUP BY p.timestamp;

----------------------------------------------------------------------
-- prices.price_usd_series — one USD close per (natural identity, day bucket).
-- The cross-source/cross-quote collapse: volume-weighted close_usd over every
-- candle of the asset in the bucket (ADR 0004 per-source rows merge at read
-- time). Arm A emits only priced rows (close_usd > 0) — status 'ok'; misses are
-- absent and classified by the reader against usd_reference (see header).
--
-- ⚠️ ARM B IS NOT SUBJECT TO THAT PREDICATE, which changes the read-time status
-- contract for peg identities. A peg asset gets its fallback row in any bucket
-- where it was merely a QUOTE leg — including buckets where nothing was priced
-- at all. So a peg identity can read `status = ok` in a bucket where
-- usd_reference is EMPTY, which §12.3 otherwise calls `no_reference`. That is
-- intended: a peg asset's USD value does not depend on the XLM/USDC reference
-- being up, so suppressing it during a blackout would withhold the one price we
-- still know. But §12.3 is therefore not universal — for peg identities read
-- `method`, do not infer provenance from usd_reference.
--
-- ## Two arms, unioned before the GROUP BY (task 0165)
-- Arm A is the historical definition: one row per priced candle, contributing
-- v = close_usd × volume_base and w = volume_base.
-- Arm B emits a ZERO-WEIGHT placeholder (v = 0, w = 0, is_peg = 1) keyed on the
-- QUOTE leg wherever a peg asset is the quote — the only way a peg asset ever
-- appears. Adding 0 to both numerator and denominator cannot perturb a weighted
-- average, so the placeholder's only effects are to CREATE THE GROUP KEY and to
-- FLAG it. Precedence is then arithmetic, not policy:
--
--   any non-peg asset ......... arm B contributes nothing → reduces to the
--                               historical expression byte-identically
--   peg, quote-only bucket .... no arm-A rows → the fallback → 1
--   peg, also traded as base .. arm-A rows exist → market value wins; the
--                               zero-weight row adds 0/0 and cannot shift it
--
-- ⚠️ Three shapes that look simpler and are WRONG:
--   * Appending a $1 row AFTER the GROUP BY emits TWO rows for the same key
--     wherever a peg asset also trades as a base. BE joins on (identity,
--     bucket), so duplicate keys silently DOUBLE every downstream aggregate.
--   * Letting the peg arm OWN the peg identities flattens a genuinely priceable
--     asset from its market rate to $1 — a regression dressed as a fix.
--     ⚠️ The original control for this was USDT (102 priceable pools, a peg
--     member that is not the preferred quote, so it does trade as a base). Task
--     0172 REMOVED USDT from the peg set — it depegged in June 2022 and trades at
--     ~$0.13 — so the peg set is now USDC alone, and USDC never trades as a base.
--     The shape is therefore currently unreachable on prod, which is exactly why
--     it is pinned synthetically in
--     `peg_member_that_also_trades_as_a_base_keeps_its_market_value`: the guard
--     has to survive the next peg member being added (tasks 0173/0196).
--   * Expressing precedence as an anti-join (`WHERE key NOT IN (SELECT … FROM
--     traded)`) costs TWO full price_ohlcv_1d FINAL scans, because ClickHouse
--     substitutes CTEs textually rather than materialising them.
--
-- ⚠️ The guard is `sum(w) = 0` — total weight. A `countIf(is_peg = 0) = 0`
-- form ("no traded rows at all") was written first and was WRONG; review caught
-- it. Its premise — that the historical expression yields NULL at zero volume,
-- which countIf would preserve — is FALSE on 26.3.10.60. `close_usd` is a
-- NON-NULLABLE Decimal(38,14), so CAST strips the Nullable that `nullIf`
-- introduces and a zero denominator never surfaces as NULL. It lands as
-- Decimal128::MIN:
--   toTypeName(CAST(sum(v)/nullIf(sum(w),0) AS Decimal(38,14))) -> Decimal(38,14)
--   value -> -1701411834604692317316873.03715884105728
-- countIf would publish that garbage for a peg asset whose only candles carry
-- volume_base = 0, FLAGGED `method = 'traded'` — a catastrophic number labelled
-- as measured, in the column BE multiplies into TVL. `sum(w) = 0` returns the
-- fallback instead: correct, and the safer failure.
--
-- ⚠️ KNOWN RESIDUAL, deliberately NOT fixed here. The fallback can only fire
-- where arm B emitted a placeholder, i.e. where the peg asset is a QUOTE leg in
-- that bucket. A peg asset appearing ONLY as a zero-volume BASE has
-- max(is_peg) = 0 and still publishes Decimal128::MIN. That is PRE-EXISTING
-- (the historical view carried the identical expression) and NOT peg-specific:
-- any asset whose only priced candles carry zero volume hits it. Fixing it
-- means deciding whether such a row should be omitted entirely — a change to
-- the "misses are absent" contract that needs BE input. Its own task.
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.price_usd_series AS
SELECT
    asset_kind,
    asset_code,
    issuer_address,
    contract_address,
    bucket,
    if(max(is_peg) = 1 AND sum(w) = 0,
       CAST(1 AS Decimal(38, 14)),
       CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14))) AS close_usd,
    CAST(if(max(is_peg) = 1 AND sum(w) = 0, 'peg', 'traded') AS LowCardinality(String)) AS method
FROM
(
    -- Arm A — every priced candle, keyed on the BASE leg.
    SELECT
        multiIf(
            a.contract_address != '', 'contract',
            a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
            'credit') AS asset_kind,
        if(a.contract_address != '', '', a.asset_code)     AS asset_code,
        if(a.contract_address != '', '', a.issuer_address) AS issuer_address,
        a.contract_address AS contract_address,
        p.timestamp        AS bucket,
        toFloat64(p.close_usd) * toFloat64(p.volume_base) AS v,
        toFloat64(p.volume_base)                          AS w,
        toUInt8(0)                                        AS is_peg
    FROM prices.price_ohlcv_1d AS p FINAL
    INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
    WHERE p.close_usd > 0

    UNION ALL

    -- Arm B — zero-weight peg placeholder, keyed on the QUOTE leg.
    -- ⚠️ COST: this IS a second full FINAL pass over the candle table. An
    -- earlier comment claimed a cheap narrow projection; that was wrong and was
    -- corrected in review. The peg predicate sits on the JOINED `assets` side,
    -- so every candle row is read and hash-joined before it can be discarded.
    -- It reads fewer COLUMNS than arm A, not fewer ROWS. If this measures badly
    -- on prod, push the peg set onto the primary key instead
    -- (`WHERE p.quote_asset_id IN (SELECT asset_id FROM prices.assets FINAL
    -- WHERE …)` — quote_asset_id is the second ORDER BY column), or materialise
    -- the series per task 0150. NOT MEASURED at prod scale.
    SELECT
        multiIf(
            q.contract_address != '', 'contract',
            q.asset_code = 'XLM' AND q.issuer_address = '', 'native',
            'credit') AS asset_kind,
        if(q.contract_address != '', '', q.asset_code)     AS asset_code,
        if(q.contract_address != '', '', q.issuer_address) AS issuer_address,
        q.contract_address AS contract_address,
        p.timestamp        AS bucket,
        toFloat64(0)       AS v,
        toFloat64(0)       AS w,
        toUInt8(1)         AS is_peg
    FROM prices.price_ohlcv_1d AS p FINAL
    INNER JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
    WHERE q.contract_address = ''
      -- USDC only. USDT was removed here by task 0172: the canonical Stellar
      -- USDT depegged in June 2022 and trades at ~$0.13, so the $1 placeholder
      -- published a 7.4x overstatement. It is priced by measurement instead.
      AND (q.asset_code = 'USDC' AND q.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN')
)
GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket;

----------------------------------------------------------------------
-- Hourly-grain variants — identical shape/semantics to the daily views above,
-- reading price_ohlcv_1h (also forever-retained, also carries close_usd). For
-- read-time TVL keyed to a ledger's closed_at at hourly resolution. Filter by
-- bucket range so the predicate pushes down to the _1h scan (bounded by the
-- window, not full history); promote to a materialized table only if measured
-- read latency demands it (design note §6).
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.usd_reference_1h AS
SELECT
    p.timestamp AS bucket,
    CAST(sum(toFloat64(p.close) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS xlm_usd
FROM prices.price_ohlcv_1h AS p FINAL
INNER JOIN prices.assets AS base  FINAL ON base.asset_id  = p.asset_id
INNER JOIN prices.assets AS quote FINAL ON quote.asset_id = p.quote_asset_id
WHERE base.asset_code = 'XLM' AND base.issuer_address = '' AND base.contract_address = ''
  AND quote.asset_code = 'USDC'
  AND quote.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND p.close > 0
GROUP BY p.timestamp;

-- Peg-fill arm mirrors price_usd_series exactly (task 0165) — same two arms,
-- same countIf guard, same method values. See that view's block comment for the
-- case table and the three wrong-looking-simpler shapes. Keep the two bodies in
-- step: a fix applied to only one grain is the defect the hourly variant had
-- before 0165 found it in both.
CREATE OR REPLACE VIEW prices.price_usd_series_1h AS
SELECT
    asset_kind,
    asset_code,
    issuer_address,
    contract_address,
    bucket,
    if(max(is_peg) = 1 AND sum(w) = 0,
       CAST(1 AS Decimal(38, 14)),
       CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14))) AS close_usd,
    CAST(if(max(is_peg) = 1 AND sum(w) = 0, 'peg', 'traded') AS LowCardinality(String)) AS method
FROM
(
    SELECT
        multiIf(
            a.contract_address != '', 'contract',
            a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
            'credit') AS asset_kind,
        if(a.contract_address != '', '', a.asset_code)     AS asset_code,
        if(a.contract_address != '', '', a.issuer_address) AS issuer_address,
        a.contract_address AS contract_address,
        p.timestamp        AS bucket,
        toFloat64(p.close_usd) * toFloat64(p.volume_base) AS v,
        toFloat64(p.volume_base)                          AS w,
        toUInt8(0)                                        AS is_peg
    FROM prices.price_ohlcv_1h AS p FINAL
    INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
    WHERE p.close_usd > 0

    UNION ALL

    -- Arm B — zero-weight peg placeholder, keyed on the QUOTE leg.
    -- ⚠️ COST: this IS a second full FINAL pass over the candle table. An
    -- earlier comment claimed a cheap narrow projection; that was wrong and was
    -- corrected in review. The peg predicate sits on the JOINED `assets` side,
    -- so every candle row is read and hash-joined before it can be discarded.
    -- It reads fewer COLUMNS than arm A, not fewer ROWS. If this measures badly
    -- on prod, push the peg set onto the primary key instead
    -- (`WHERE p.quote_asset_id IN (SELECT asset_id FROM prices.assets FINAL
    -- WHERE …)` — quote_asset_id is the second ORDER BY column), or materialise
    -- the series per task 0150. NOT MEASURED at prod scale.
    SELECT
        multiIf(
            q.contract_address != '', 'contract',
            q.asset_code = 'XLM' AND q.issuer_address = '', 'native',
            'credit') AS asset_kind,
        if(q.contract_address != '', '', q.asset_code)     AS asset_code,
        if(q.contract_address != '', '', q.issuer_address) AS issuer_address,
        q.contract_address AS contract_address,
        p.timestamp        AS bucket,
        toFloat64(0)       AS v,
        toFloat64(0)       AS w,
        toUInt8(1)         AS is_peg
    FROM prices.price_ohlcv_1h AS p FINAL
    INNER JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
    WHERE q.contract_address = ''
      -- USDC only. USDT was removed here by task 0172: the canonical Stellar
      -- USDT depegged in June 2022 and trades at ~$0.13, so the $1 placeholder
      -- published a 7.4x overstatement. It is priced by measurement instead.
      AND (q.asset_code = 'USDC' AND q.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN')
)
GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket;

----------------------------------------------------------------------
-- prices.identity_by_contract — SAC read-seam resolver (§12.4).
-- The §12.4 SAC→classic collapse is WRITE-TIME: a SAC-wrapped leg's candles are
-- stored under the underlying classic identity, so price_usd_series has NO row
-- keyed by the SAC contract address. A read-time consumer with a Soroban-DEX pool
-- leg (a contract address) resolves it here to the natural identity to look up in
-- price_usd_series: a pure Soroban token maps to itself (`contract`); a SAC maps
-- to its classic underlying (`native`/`credit`). Join your leg's contract
-- address on `contract`, then join the resulting identity to price_usd_series.
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.identity_by_contract AS
SELECT
    contract_address AS contract,
    'contract'       AS asset_kind,
    ''               AS asset_code,
    ''               AS issuer_address,
    contract_address AS contract_address
FROM prices.assets FINAL
WHERE contract_address != ''
UNION ALL
SELECT
    sac_address AS contract,
    multiIf(asset_code = 'XLM' AND issuer_address = '', 'native', 'credit') AS asset_kind,
    asset_code      AS asset_code,
    issuer_address  AS issuer_address,
    ''              AS contract_address
FROM prices.assets FINAL
WHERE sac_address != '';

----------------------------------------------------------------------
-- prices.current_price_usd — live spot (tip) per asset, natural-identity keyed.
-- Same contract as price_usd_series (natural id, NULL-never-error via the
-- consumer's LEFT JOIN) but for "now": one row per asset with the latest USD
-- price + `updated_at`. ⚠️ **`updated_at` is the MV's refresh time, not the
-- price's age** — since task 0135 `price_usd` is the latest *priced* close and
-- is not age-bounded, so a staleness policy keyed on `updated_at` cannot see
-- how old it is. See the sentinel table above; no column carries the price's
-- own timestamp yet. Reads
-- current_prices, which is written by the Current Price Updater (task 0039) —
-- this view is the read surface; it is empty until that writer runs.
--
-- Task 0072 forwards the remaining current_prices columns, so `sources` /
-- `price_xlm` / `change_*_pct` / `vwap_24h` are reachable to an in-cluster
-- consumer at all.
--
-- ⚠️ CORRECTED 2026-08-31 (task 0178). This block used to assert "BE reads this
-- surface IN-CLUSTER (see their 0199 contract)". **They do not, and they never
-- have.** Verified by reading their repo at origin/develop, not by asking: the
-- only prices objects their code queries are `price_usd_series` and
-- `price_usd_series_1h`, for the identity triple, `bucket` and `close_usd`
-- (crates/api/src/liquidity_pools/queries.rs:573, :1572, :2413). Both mentions
-- of `current_price_usd` in their tree are COMMENTS explaining why they avoid
-- it — box-measured 2026-08-04, `price_usd = 0` for native XLM, so every
-- XLM-leg pool would have read a NULL TVL.
--
-- The stale claim mattered: it is why 0178 nearly sent BE a question about a
-- surface they do not consume. Re-verify against their code before writing a
-- sentence about what any consumer reads.
--
-- Task 0178 appends `method` (14 columns) — the same provenance vocabulary
-- price_usd_series carries, now on the tip. `'oracle'` marks the canonical-USDC
-- row, whose price comes from prices.usd_rate rather than from candles; `''` is
-- the unavailable sentinel. See init.sql's block on prices.current_prices for
-- the full vocabulary and why it is NOT usd_rate.method.
--
-- ⚠️ NEW COLUMNS ARE APPENDED, NEVER INSERTED — which protects column ORDER,
-- not ARITY. The first six keep the positions they shipped with (hence
-- `updated_at` sitting mid-list rather than last), so nothing is re-ordered
-- underneath a consumer; but every consumer now gets 14 columns where it got 6.
-- Anything decoding POSITIONALLY off `SELECT *` — a fixed-arity tuple fetch, a
-- clickhouse-crate row struct, `INSERT INTO t SELECT * FROM …` — breaks on the
-- extra columns. In-cluster consumers (BE's 0199 contract) should pin an
-- explicit column list rather than rely on `SELECT *`. See the sentinel table
-- in the JOIN interop contract above for what each new column means.
--
-- ⚠️ CREATE OR REPLACE — see the "Statement form" section in the file header.
-- This view is the reason that rule exists: its definition changes with the MV's
-- column set, so an `IF NOT EXISTS` apply would silently leave the old shape
-- standing. Task 0134 converted the remaining five views to match.
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.current_price_usd AS
SELECT
    multiIf(
        a.contract_address != '', 'contract',
        a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
        'credit') AS asset_kind,
    if(a.contract_address != '', '', a.asset_code)     AS asset_code,
    if(a.contract_address != '', '', a.issuer_address) AS issuer_address,
    a.contract_address AS contract_address,
    c.price_usd        AS price_usd,
    c.updated_at       AS updated_at,
    c.price_xlm        AS price_xlm,
    c.change_24h_pct   AS change_24h_pct,
    c.change_7d_pct    AS change_7d_pct,
    c.volume_24h_usd   AS volume_24h_usd,
    c.market_cap_usd   AS market_cap_usd,
    c.vwap_24h         AS vwap_24h,
    c.sources          AS sources,
    c.method           AS method
FROM prices.current_prices AS c FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = c.asset_id;
