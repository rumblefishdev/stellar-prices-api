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
-- ### current_price_usd only (task 0072)
-- These carry SENTINELS, not NULLs — `current_prices`' columns are
-- non-nullable, so "unavailable" and a real value share a type and can only be
-- told apart by value. That is a weaker contract than the value-or-absent one
-- above, and consumers have to handle it explicitly:
--   price_xlm        Decimal(38,14). 0 = unavailable (no XLM market, or an
--                    un-enriched tip) — indistinguishable from a true 0.
--   change_24h_pct / change_7d_pct
--                    Decimal(10,4) percent, clamped to ±999999.9999 (an
--                    overflow would poison the whole MV refresh). 0 =
--                    unavailable AND 0 = a genuinely flat 24h/7d; the two are
--                    NOT distinguishable. Treat 0 as "no signal".
--   volume_24h_usd / market_cap_usd / vwap_24h
--                    Decimal(38,14). 0 = unavailable. market_cap_usd is 0
--                    whenever circulating supply is absent (best-effort join).
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
-- time). Only priced rows (close_usd > 0) — status 'ok'; misses are absent and
-- classified by the reader against usd_reference (see header).
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.price_usd_series AS
SELECT
    multiIf(
        a.contract_address != '', 'contract',
        a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
        'credit') AS asset_kind,
    if(a.contract_address != '', '', a.asset_code)     AS asset_code,
    if(a.contract_address != '', '', a.issuer_address) AS issuer_address,
    a.contract_address AS contract_address,
    p.timestamp        AS bucket,
    CAST(sum(toFloat64(p.close_usd) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS close_usd
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.close_usd > 0
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

CREATE OR REPLACE VIEW prices.price_usd_series_1h AS
SELECT
    multiIf(
        a.contract_address != '', 'contract',
        a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
        'credit') AS asset_kind,
    if(a.contract_address != '', '', a.asset_code)     AS asset_code,
    if(a.contract_address != '', '', a.issuer_address) AS issuer_address,
    a.contract_address AS contract_address,
    p.timestamp        AS bucket,
    CAST(sum(toFloat64(p.close_usd) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS close_usd
FROM prices.price_ohlcv_1h AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.close_usd > 0
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
-- price + `updated_at` for the consumer's own staleness policy. Reads
-- current_prices, which is written by the Current Price Updater (task 0039) —
-- this view is the read surface; it is empty until that writer runs.
--
-- Task 0072 forwards the remaining current_prices columns. BE reads this surface
-- IN-CLUSTER (named views, no HTTP — see their 0199 contract), so until the view
-- named them, `sources` / `price_xlm` / `change_*_pct` / `vwap_24h` were
-- unreachable to that consumer no matter what the MV wrote.
--
-- ⚠️ NEW COLUMNS ARE APPENDED, NEVER INSERTED — which protects column ORDER,
-- not ARITY. The first six keep the positions they shipped with (hence
-- `updated_at` sitting mid-list rather than last), so nothing is re-ordered
-- underneath a consumer; but every consumer now gets 13 columns where it got 6.
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
    c.sources          AS sources
FROM prices.current_prices AS c FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = c.asset_id;
