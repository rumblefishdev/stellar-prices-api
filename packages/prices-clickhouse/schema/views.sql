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
-- `prices_clickhouse::USDT_ISSUER` IS referenced by these views again, by ONE
-- thing and one only: task 0147's `eligible_quotes` set, which names the
-- canonical USDT identity as a quote leg we can in principle price in USD. It
-- is a hand-synced copy under exactly the same rule as the USDC literal above —
-- change it here AND in that const together, or the views and the writer
-- diverge.
-- ⚠️ It is NOT back in the PEG-FILL arm and must never be. Task 0172 removed it
-- from there because the canonical Stellar USDT depegged in June 2022 and
-- trades at ~$0.13, so a $1 placeholder overstated close_usd by ~7.4x on
-- 44,657 candles across 495 base assets (see below). Being an ELIGIBLE QUOTE
-- says only "a USDT-quoted candle's volume belongs in the coverage
-- denominator"; it makes no claim about USDT's own price and creates no row
-- for it.
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
-- ⚠️ The `1` is the FALLBACK, not the answer (task 0168, shipped). The
-- placeholder carries the MEASURED rate from prices.usd_rate (task 0167, fed by
-- oracle_worker's Reflector poll) wherever an observation exists in the bucket,
-- and falls back to `1` only where none does — deep history before the oracle
-- window (measured on prod: no reading before 2026-03-11), or a bucket the
-- oracle sat out. The two cases are told apart by `method`: 'oracle' is
-- measured, 'peg' means "no measured rate was available" and NEVER "$1 is
-- correct". Without that column a consumer cannot tell a real 1.0000 from a
-- fallback 1.0000 — the `close_usd = 0` mistake in a new surface.
--
-- Why it matters that the fallback is rare: a flat $1 is a ~0.1% systematic
-- error, not a rare-depeg approximation (USDC measured 1.00066784838102 on
-- prod 2026-08-10 — an ordinary Sunday, stable to four decimals across five
-- consecutive readings). It also CONTRADICTED OUR OWN CANDLES: the oracle
-- enrichment tier already prices a TF/USDC candle at `close × 0.9993`
-- (ch_enrich.rs:20), so the same bucket read 0.9993 in the candles and 1.0000
-- here. That disagreement is what this arm now closes.
--
-- ⚠️ STILL OPEN, deliberately: the enrichment PEG TIER bakes the same flat $1
-- into `close_usd` itself for everything the oracle tier did not win (all deep
-- history, plus anything outside its staleness bound). Fixing that is a
-- write-path change plus re-enrichment, not a view swap — see task 0168's
-- "Known adjacent gap".
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
-- ⚠️ That contract covers THESE views — `price_usd_series*` and
-- `usd_reference*` — and no other surface. ADR 0292 §5 writes down what each
-- surface publishes when a bucket has no USD price: here the row is absent;
-- `/ohlcv` returns the bucket with `null` price fields and its volume;
-- `current_price_usd` (below) carries a SENTINEL 0. The underlying
-- `close_usd = 0` has four meanings — pending, unpriceable, genuinely zero, and
-- (ADR 0287) "this bucket has no price" — and `no_asset_price` above does not
-- tell them apart; the reason is a read-time classification (ADR 0292 §3).
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
--             'peg'    — the peg placeholder supplied the value AND no
--                        measured rate existed for that identity in that
--                        bucket, so it is the $1 FALLBACK. Read it as "no
--                        measured rate available", NOT as "$1 is correct".
--                        On prod this is deep history (before 2026-03-11) and
--                        buckets the oracle sat out.
--             'oracle' — the peg placeholder supplied the value from a MEASURED
--                        depeg-aware reading in prices.usd_rate (task 0168):
--                        the bucket's last observation, method = 'oracle'
--                        there too. A row reading exactly 1.0000 under this
--                        label is a measurement that happened to be at par.
--   Without this column a consumer cannot tell a real 1.0000 from a fallback
--   1.0000 — the `close_usd = 0` mistake (one value meaning several things) in
--   a new surface.
--
--   ⚠️ THIS ENUM IS NOT THE /ohlcv CANDLE ENUM. Task 0268 split the candle-path
--   vocabulary into 'assumed-par' (the literal 1.0 was the input) and 'external'
--   (an imported measured USDC/USD series was the input), and retired 'peg'
--   there. The three values above are unchanged and keep 0165's meanings: this
--   view labels how a BUCKET's close_usd was arrived at, not how one candle's
--   quote leg was priced. Do not assume the two surfaces share one enum.
--
--   ⚠️ After 0268's re-enrichment the deep-history USDC-quoted rows change which
--   arm of THIS view they arrive through. Today they reach 'peg' via arm B, the
--   $1 fallback, because their stored close_usd is close x 1.00 and no measured
--   rate exists for the bucket. Once the external tier has scaled them they
--   carry a real USD value and aggregate through arm A as 'traded' instead. The
--   'peg' arms below are therefore expected to SHRINK on prod after that pass,
--   without their definition changing.
--
--   ⚠️ This is an APPENDED column: arity changed, order did not.
--   Anything decoding POSITIONALLY off `SELECT *` gets an extra column; pin an
--   explicit column list.
--
-- ### price_usd_series* only — `priced_volume_share` (task 0147, APPENDED LAST)
--   priced_volume_share  Decimal(10, 6) in [0, 1]. The fraction of the
--                        bucket's ELIGIBLE price-forming volume (`pf_volume`)
--                        that some pricing tier had actually priced when this
--                        row was computed — i.e. how much of what traded the
--                        published close_usd is an average OF.
--                        1 means every eligible unit was priced.
--                        ⚠️ The denominator is `eligible OR priced`, never
--                        `eligible` alone: a PRICED row belongs in it whatever
--                        its quote leg is, and making that true by
--                        construction is what keeps the ratio inside [0, 1]
--                        (see the `rew` note in arm A). A peg-arm row
--                        publishes a literal 1: its value comes from a measured
--                        rate, not from traded weight, so there is no
--                        population for the share to describe.
--   ⚠️ NEVER NULL. BE renders a NULL as a dash and drops the pool, so the
--   division is guarded with an `if`, never a `nullIf` (which a non-Nullable
--   CAST turns into Decimal128::MIN or code 349 anyway — see below).
--   ⚠️ This is an APPENDED column: arity changed, order did not. `method` is
--   still seventh; this is eighth. Anything decoding POSITIONALLY off
--   `SELECT *` gets one more column than it did — pin an explicit column list.
--
-- ### THE COVERAGE GATE (task 0147) — what is published, and what is withheld
-- Until 2026-09 arm A filtered `close_usd > 0` BEFORE the weighted average, so
-- whichever rows enrichment happened to have reached became 100 % of the
-- weight. Measured by BE on yXLM, 2026-08-04 13:00: a single 0.764-unit print
-- at 1.3085 was the only enriched row in the bucket and this view published
-- 1.3085 against a true ~0.170 — a 7.7x overstatement in the column they
-- multiply into TVL. The arithmetic was right; the POPULATION was wrong.
--
-- A bucket is now published only when
--   priced_volume_share >= X            -- most of what traded is priced
-- and is otherwise ABSENT — the value-or-absent contract is unchanged — while
-- `price_usd_series_coverage{,_1h}` says why.
--
--   X = 0.5
--
-- X is spelled in FOUR executable places — the publish WHERE of both series
-- grains and the `priced` branch of both coverage grains' `status` — and
-- `views_sql_gate_constants_agree_between_the_header_the_gates_and_the_coverage_status`
-- pins all four to THIS line.
--
-- Measured on prod 2026-09-23 (task 0147 phase 2, dev_read, read-only), with
-- this file's own coverage body over four windows: 1h 09-23 06:00→15:00
-- (13,363 eligible buckets, incl. the hour in progress), 1h 09-22 12:00→09-23
-- 12:00 post-0286 (24,311), 1h 09-15→09-22 (122,166) and 1d 08-23→09-22
-- (106,671). NOT ONE bucket had 0 < share < 1: enrichment prices a bucket
-- whole or not at all, so today X withholds nothing a different X in (0, 1]
-- would not. It is a guard for the yXLM shape (a bucket half-enriched, e.g.
-- USDT-quoted candles waiting on the sweep, task 0209), and 0.5 is the
-- volume-weighted-median rule: a price resting on less than half of what
-- traded is not that bucket's price. BE reads the share on every row and sets
-- its own bar on it.
--
-- There is NO absolute USD floor, by measurement (same day, same windows).
-- Median `priced_volume_usd` per bucket is $0.01–0.02 (p90 ≈ $5): a $1 floor
-- keeps 15.7–17.6 % of today's published buckets, $100 keeps 3.2–3.7 %. Nor
-- does volume predict a bad price: against the median of the same asset's
-- other hourly buckets within ±12 h (274,754 buckets, 09-01→09-22), the median
-- error is ~0.6 % in every volume bin from < $0.01 to ≥ $1k, and `usd < 100`
-- catches 93 % of > 2x outliers by withholding 96 % of all buckets — no
-- information. 0118 reached the same verdict for an unconditional floor
-- (current.sql:130-150). `priced_volume_usd` stays on the coverage views for
-- a consumer that wants a floor of its own.
--
-- ⚠️ Two caveats about `volume_quote_usd`, which `priced_volume_usd` reads:
--   * It is summed over EVERY fill, not just the price-forming subset — task
--     0286 added no `pf_volume_quote_usd`. So a row's dust volume is counted
--     alongside its real volume. Accepted approximation.
--   * It is denominated on the QUOTE leg: "USD that changed hands".
--
-- The priced predicate itself is `/ohlcv`'s `valid` for the SAME row
-- (queries_ch.rs::usd_projection) PLUS a positive price-forming weight
-- (`p.pf_volume > 0`, tasks 0171/0198 — this surface weights by that column
-- and `/ohlcv` does not). One rule, spelled once, pinned by
-- `views_sql_every_weighted_surface_spells_the_one_priced_predicate`. It is
-- NOT term-for-term `valid`; that extra conjunct is decided by brief D-01.
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
--                    signal — as_of below is, and price_status says what kind
--                    of price this is. 0 = no priced candle in the window (an
--                    un-enriched tip alone no longer produces 0).
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
--   as_of            DateTime. The timestamp of the candle price_usd was read
--                    from — the price's own age, as opposed to updated_at,
--                    which is when this snapshot was refreshed. For a row
--                    priced from the measured USDC rate it is that reading's
--                    own timestamp.
--                    1970-01-01 00:00:00 means there is no price: treat epoch
--                    as ABSENT, never as an age. It is written deliberately
--                    whenever price_usd is the 0 sentinel, so it is a decision,
--                    not an artefact of an empty aggregate.
--                    It is NOT by itself "no price", though: the epoch is also
--                    this column's table DEFAULT, so a row the current MV has
--                    not rewritten yet carries it beside a real price — see
--                    price_status's '' below, which is the same window. Read
--                    "no price" as as_of = epoch AND price_status = 'unpriced'.
--                    price_xlm divides price_usd by an XLM/USD close dated
--                    independently and market_cap_usd multiplies it by a supply
--                    figure with its own fetch time, so both are no fresher
--                    than as_of and may be older. as_of bounds price_usd only.
--                    USD values are computed by an HOURLY pass, so an as_of up
--                    to about an hour behind updated_at is the ordinary state
--                    of an actively traded asset, not a fault.
--   price_status     LowCardinality(String). priced | carried | unpriced.
--                    priced = as_of IS the asset's newest price-forming candle
--                    in the window (every measured-rate row reads this too).
--                    carried = a real priced close, but a newer price-forming
--                    candle has not been priced yet — the price is real and
--                    stale, and as_of says how stale. On the hourly enrichment
--                    cadence that is the ordinary state of an actively traded
--                    asset for up to about an hour; it also covers a newer
--                    trade on a pair with NO USD conversion path, where no
--                    newer USD price is coming at all and as_of is the newest
--                    convertible minute.
--                    unpriced = price_usd is the 0 sentinel; method is '' for
--                    the same reason and as_of is the epoch.
--                    '' = a row the current MV has not rewritten yet (table
--                    DEFAULT); it can only be seen between the ALTER that adds
--                    the column and the MV re-CREATE. Not a vocabulary word.

----------------------------------------------------------------------
-- prices.usd_reference — per-bucket USD reference availability.
-- The volume-weighted XLM/USDC close (= XLM's USD price under the USDC≡$1 peg)
-- per day bucket. A bucket's PRESENCE is the durable "USD reference is up at T"
-- signal the reader LEFT JOINs for systemic-blackout detection. Identity-resolved
-- (not asset_id) so it survives asset-id reassignment. Reads `close` (always
-- present from the backfill), independent of enrichment timing.
--
-- ⚠️ `volume_base > 0` is REQUIRED, not a tidy-up (task 0171). The reference
-- is a weighted average; a bucket whose only XLM/USDC candles carry zero volume
-- has sum(volume_base) = 0, and CAST(… / nullIf(…, 0) AS Decimal(38, 14)) on
-- 26.3.10.60 either raises code 349 (interpreted) or publishes Decimal128::MIN
-- as XLM's USD price (JIT-compiled; see the price_usd_series header). Such a
-- bucket is ABSENT instead — `no_reference` is a legitimate §12.3 state, a
-- garbage reference is not. Same rule as arm A of the series.
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.usd_reference AS
SELECT
    p.timestamp AS bucket,
    CAST(sum(toFloat64(p.close) * toFloat64(p.pf_volume)) / nullIf(sum(toFloat64(p.pf_volume)), 0) AS Decimal(38, 14)) AS xlm_usd
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS base  FINAL ON base.asset_id  = p.asset_id
INNER JOIN prices.assets AS quote FINAL ON quote.asset_id = p.quote_asset_id
WHERE base.asset_code = 'XLM' AND base.issuer_address = '' AND base.contract_address = ''
  AND quote.asset_code = 'USDC'
  AND quote.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  -- Task 0147, D-01 — the SAME priced predicate the series arm A spells, minus
  -- the close_usd half: this surface reads `close` (always present from the
  -- backfill) and is independent of enrichment timing, so there is no USD leg
  -- to floor and no convertibility to test. What it does inherit is the
  -- precision floor, the price-forming trade count and the price-forming
  -- weight.
  -- ⚠️ `pf_volume > 0` is the 0171 guard and is NOT optional: the CAST below
  -- still divides by the weight, and a bucket whose only XLM/USDC candles
  -- carry no weight must be ABSENT (`no_reference`), never a sentinel.
  AND p.close >= toDecimal128('0.000000000001', 14)
  AND p.pf_trade_count > 0
  AND p.pf_volume > 0
GROUP BY p.timestamp;

----------------------------------------------------------------------
-- prices.price_usd_series — one USD close per (natural identity, day bucket).
-- The cross-source/cross-quote collapse: volume-weighted close_usd over every
-- candle of the asset in the bucket (ADR 0004 per-source rows merge at read
-- time). Arm A reads EVERY candle of the bucket, priced or not, and weights the
-- mean on the PRICED subset while counting the ELIGIBLE one — the denominator
-- of `priced_volume_share`. Misses are absent and classified by the reader
-- against usd_reference (see header); since task 0147 a bucket can also be
-- absent because the gate withheld it, and `price_usd_series_coverage` is where
-- a consumer reads which.
--
-- ⚠️ Until 2026-09 arm A applied `WHERE close_usd > 0 AND volume_base > 0`,
-- which removed the unpriced rows BEFORE the weighting and is what let a dust
-- print become the whole of a bucket's weight (task 0147; the gate section in
-- the file header carries the measurement). The positive-weight half of that
-- predicate survives inside the priced condition as `pf_volume > 0` — see the
-- tasks 0171/0198 note below, which is why it is not optional — but the
-- price half is gone: a zero-`close_usd` row is now counted, unpriced, in the
-- denominator, which is the entire point.
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
--   peg, quote-only bucket .... no arm-A rows → the bucket's MEASURED rate
--                               ('oracle'), or $1 where none exists ('peg')
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
-- ⚠️ The residual that guard could not reach — CLOSED by tasks 0171/0198.
-- The fallback fires only where arm B emitted a placeholder, i.e. where the
-- peg asset is a QUOTE leg in that bucket. Any asset appearing ONLY as a
-- zero-volume BASE has max(is_peg) = 0, reached the CAST with sum(w) = 0, and
-- published Decimal128::MIN flagged `traded` — NOT peg-specific, and
-- pre-existing (the historical view carried the identical expression). BE
-- decided 2026-08-11: OMIT THE ROW — "misses are absent" is the contract their
-- whole read path assumes, and a published sentinel is a magic constant every
-- consumer must know forever. Arm A therefore admits weight only from a row
-- with `pf_volume > 0`; task 0147 moved that from a `WHERE` into the priced
-- condition of a conditional sum, so a zero-weight group CAN now form — and
-- the guard moved with it, INSIDE the CAST's argument
-- (`if(sum(rpw) > 0, sum(rpv) / sum(rpw), 0)`), with the outer `WHERE` gate
-- withholding the row on top. The peg placeholder (rpw = 0 by construction) is
-- untouched because it lives in arm B and the gate's first disjunct is its.
-- Task 0198 recorded the same case as RAISING CANNOT_INSERT_NULL_IN_ORDINARY_COLUMN
-- (code 349). BOTH readings are right on 26.3.10.60; the expression JIT picks
-- one. Interpreted, the CAST raises 349 and the WHOLE query fails. Once
-- `compile_expressions` (default 1) has compiled it — after
-- `min_count_to_compile_expression` (default 3) executions — the compiled
-- CAST publishes Decimal128::MIN instead. A cold server raises, a warm one
-- lies. Pinned in both modes by
-- `a_zero_volume_only_base_is_absent_and_its_neighbours_still_publish`.
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.price_usd_series AS
SELECT
    asset_kind,
    asset_code,
    issuer_address,
    contract_address,
    bucket,
    close_usd,
    method,
    priced_volume_share
FROM
(
SELECT
    asset_kind,
    asset_code,
    issuer_address,
    contract_address,
    bucket,
    if(max(is_peg) = 1 AND sum(rpw) = 0,
       if(max(peg_rate) > 0, max(peg_rate), CAST(1 AS Decimal(38, 14))),
       CAST(if(sum(rpw) > 0, sum(rpv) / sum(rpw), toFloat64(0)) AS Decimal(38, 14))) AS close_usd,
    CAST(if(max(is_peg) = 1 AND sum(rpw) = 0,
            -- Three-way since task 0267, because a measured rate can now arrive
            -- from two provenances. ⚠️ The PEG DISCRIMINATOR STAYS FIRST and
            -- stays `max(peg_rate) <= 0`: arm B's join_use_nulls note explains
            -- that an unmatched LEFT JOIN yields the column DEFAULT rather than
            -- NULL on prod, so "this bucket has no rate at all" reads as 0 in
            -- peg_rate AND as 0 in rate_rank, and only the peg_rate test reads
            -- identically under both join_use_nulls settings. Ordering the rank
            -- test first would work by accident, not by rule.
            -- Only once a rate EXISTS does the rank say which provenance it came
            -- from: 2 = 'oracle', 1 = 'external'. See arm B for why the rank is
            -- numeric rather than a max() over the method string.
            multiIf(max(peg_rate) <= 0, 'peg', max(rate_rank) = 2, 'oracle', 'external'),
            'traded') AS LowCardinality(String)) AS method,
    -- Task 0147, D-05 — APPENDED LAST, after `method`. Arity changes, order
    -- does not; see the JOIN interop contract above for what that costs a
    -- consumer decoding positionally off `SELECT *`.
    -- ⚠️ NEVER NULL (D-06). BE renders a NULL as a dash and drops the pool, so
    -- the guard is an `if`, not a `nullIf`: a peg-arm bucket publishes a
    -- literal 1 (it has no traded weight to be short of), and the division is
    -- guarded INSIDE the CAST's argument.
    CAST(if(max(is_peg) = 1 AND sum(rpw) = 0,
            toFloat64(1),
            if(sum(rew) > 0, sum(rpw) / sum(rew), toFloat64(0))) AS Decimal(10, 6)) AS priced_volume_share,
    -- The gate's inputs, consumed by the outer WHERE and projected away there.
    max(is_peg) AS peg_present,
    sum(rpv)    AS pv,
    sum(rpw)    AS pw,
    sum(rew)    AS ew
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
        -- ONE priced predicate (task 0147, D-01): `/ohlcv`'s `valid` for the
        -- SAME row (queries_ch.rs::usd_projection) PLUS a positive
        -- price-forming WEIGHT. The shared terms are the 1e-12 precision floor
        -- on BOTH price columns, a price-forming trade and convertibility;
        -- `p.pf_volume > 0` is the extra one (tasks 0171/0198), and it is here
        -- because this surface WEIGHTS by that column and `/ohlcv` does not —
        -- a zero-weight row cannot move a weighted mean, but it CAN empty the
        -- denominator. Decided by brief D-01, so the two are the same rule and
        -- not the same expression: do not call them identical.
        -- Pinned against prices-api by
        -- views_sql_every_weighted_surface_spells_the_one_priced_predicate.
        (p.close >= toDecimal128('0.000000000001', 14)
            AND p.close_usd >= toDecimal128('0.000000000001', 14)
            AND p.pf_trade_count > 0
            AND p.pf_volume > 0
            AND (p.quote_asset_id IN
                 (
                     SELECT u.asset_id
                     FROM prices.assets AS u FINAL
                     WHERE u.asset_code = 'USDC'
                       AND u.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
                       AND u.contract_address = ''
                 )
                 OR p.close_usd != p.close))                AS is_priced,
        -- ELIGIBLE (task 0147, D-02) — computed here, no new table. The
        -- denominator of the coverage share is every unit traded against a
        -- quote we could IN PRINCIPLE price in USD: the canonical USDC, native
        -- XLM and the canonical USDT identities, plus every asset that has a
        -- row in prices.usd_rate under ANY method.
        -- ⚠️ usd_rate keys on the NATURAL IDENTITY, never on asset_id, so this
        -- is a four-column identity join and the usd_rate side needs
        -- CAST(asset_kind AS String) exactly as arm B's rate subquery does.
        -- ⚠️ Membership is RETROACTIVE: a bucket stops reading `unpriceable`
        -- the moment a rate appears for its quote leg. Accepted by ADR 0292.
        -- ⚠️ This flag is the QUOTE-SET half of the denominator only. The
        -- weight below is `is_priced OR is_eligible`, because a row we DID
        -- price is eligible whatever its quote leg is — see the note on `rew`.
        (p.quote_asset_id IN
         (
             SELECT e.asset_id
             FROM prices.assets AS e FINAL
             WHERE (e.asset_code = 'USDC' AND e.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN' AND e.contract_address = '')
                OR (e.asset_code = 'XLM'  AND e.issuer_address = '' AND e.contract_address = '')
                OR (e.asset_code = 'USDT' AND e.issuer_address = 'GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V' AND e.contract_address = '')
             UNION ALL
             SELECT ei.asset_id
             FROM
             (
                 SELECT
                     ea.asset_id AS asset_id,
                     multiIf(
                         ea.contract_address != '', 'contract',
                         ea.asset_code = 'XLM' AND ea.issuer_address = '', 'native',
                         'credit') AS asset_kind,
                     if(ea.contract_address != '', '', ea.asset_code)     AS asset_code,
                     if(ea.contract_address != '', '', ea.issuer_address) AS issuer_address,
                     ea.contract_address AS contract_address
                 FROM prices.assets AS ea FINAL
             ) AS ei
             INNER JOIN
             (
                 SELECT DISTINCT
                     CAST(asset_kind AS String) AS asset_kind,
                     asset_code,
                     issuer_address,
                     contract_address
                 FROM prices.usd_rate FINAL
             ) AS er
                 ON  er.asset_kind       = ei.asset_kind
                 AND er.asset_code       = ei.asset_code
                 AND er.issuer_address   = ei.issuer_address
                 AND er.contract_address = ei.contract_address
         ))                                                 AS is_eligible,
        -- Conditional sums, NOT a WHERE (task 0147). The unpriced rows are real
        -- trades and belong in the denominator — filtering them out before the
        -- weighting is exactly what let a 0.764-unit dust print become the
        -- whole of a bucket's weight.
        -- ⚠️ toFloat64 PER ROW before multiplying: Decimal x Decimal / Decimal
        -- raises code 407 (DECIMAL_OVERFLOW) on 26.3.10.60, measured twice.
        if(is_priced,   toFloat64(p.close_usd) * toFloat64(p.pf_volume), toFloat64(0)) AS rpv,
        if(is_priced,   toFloat64(p.pf_volume),                          toFloat64(0)) AS rpw,
        if(is_priced,   toFloat64(p.volume_quote_usd),                   toFloat64(0)) AS rpusd,
        -- ⚠️ `is_priced OR is_eligible`, NOT `is_eligible` alone. The priced set
        -- is not a subset of the eligible one by definition: `is_priced` admits
        -- a row on `close_usd != close` for ANY quote, while `is_eligible` names
        -- a QUOTE SET. A row priced against a quote outside that set would land
        -- in the numerator and not in the denominator, so the share could leave
        -- [0, 1] entirely (measured on 26.3.10.60: 5000000 published in a
        -- Decimal(10, 6) column this file documents as [0, 1] — CAST does not
        -- range-check P, so nothing raises). Not reachable through today's
        -- write path; made unreachable BY CONSTRUCTION here rather than by a
        -- clamp, so `priced ⊆ eligible` holds whatever a future writer does.
        -- Pinned by views_it.rs::
        -- a_priced_row_outside_the_eligible_quote_set_stays_inside_the_share.
        if(is_priced OR is_eligible, toFloat64(p.pf_volume),             toFloat64(0)) AS rew,
        toUInt8(0)                                        AS is_peg,
        CAST(0 AS Decimal(38, 14))                        AS peg_rate,
        -- UNION ALL matches arms POSITIONALLY and requires an identical column
        -- count, so this placeholder is structural, not decoration (task 0267).
        toUInt8(0)                                        AS rate_rank
    FROM prices.price_ohlcv_1d AS p FINAL
    INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id

    UNION ALL

    -- Arm B — zero-weight peg placeholder, keyed on the QUOTE leg, carrying
    -- that identity's MEASURED USD rate for the bucket (task 0168; 0 = none).
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
        b.asset_kind,
        b.asset_code,
        b.issuer_address,
        b.contract_address,
        b.bucket,
        toUInt8(0)   AS is_priced,
        toUInt8(0)   AS is_eligible,
        toFloat64(0) AS rpv,
        toFloat64(0) AS rpw,
        toFloat64(0) AS rpusd,
        toFloat64(0) AS rew,
        toUInt8(1)   AS is_peg,
        -- 0 = "no measured rate for this identity in this bucket" -> the $1
        -- fallback, flagged method = 'peg'. ⚠️ NOT NULL: prod runs
        -- join_use_nulls = 0, so an unmatched LEFT JOIN yields the column
        -- DEFAULT (0 for a Decimal), never NULL. A coalesce()/IS NULL form here
        -- would be DEAD CODE on prod. ifNull() only covers a session that sets
        -- join_use_nulls = 1; the `> 0` test in the outer SELECT is the real
        -- discriminator, and it reads the same under both settings.
        ifNull(r.usd_rate, CAST(0 AS Decimal(38, 14))) AS peg_rate,
        -- Task 0267. NUMERIC rank, never a max() over the method STRING: that
        -- would only work by the lexicographic accident that 'oracle' sorts
        -- above 'external', which is a property of the English words and not of
        -- the precedence rule. 2 beats 1 by arithmetic.
        -- Same join_use_nulls rule as peg_rate above: an unmatched LEFT JOIN
        -- yields the column DEFAULT, so the rank arrives as 0 (not NULL) and the
        -- ifNull() covers only a session that sets join_use_nulls = 1.
        multiIf(ifNull(r.rate_method, '') = 'oracle', 2,
                ifNull(r.rate_method, '') = 'external', 1,
                0) AS rate_rank
    FROM
    (
        SELECT
            multiIf(
                q.contract_address != '', 'contract',
                q.asset_code = 'XLM' AND q.issuer_address = '', 'native',
                'credit') AS asset_kind,
            if(q.contract_address != '', '', q.asset_code)     AS asset_code,
            if(q.contract_address != '', '', q.issuer_address) AS issuer_address,
            q.contract_address AS contract_address,
            p.timestamp        AS bucket
        FROM prices.price_ohlcv_1d AS p FINAL
        INNER JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
        WHERE q.contract_address = ''
          -- USDC only. USDT was removed here by task 0172: the canonical Stellar
          -- USDT depegged in June 2022 and trades at ~$0.13, so the $1 placeholder
          -- published a 7.4x overstatement. It is priced by measurement instead.
          -- ⚠️ THIS PREDICATE IS THE PEG SET. The rate join below follows
          -- whatever identity passes it, so adding a member here is a claim that
          -- the oracle prices THAT ISSUER — see the fence at the top of this file
          -- and `peg_identities_is_exactly_canonical_usdc` (oracle-worker).
          AND (q.asset_code = 'USDC' AND q.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN')
    ) AS b
    -- Task 0168 — the measured rate, one row per (identity, bucket).
    --
    -- This IS task 0167's resolution rule (newest observation at or before the
    -- bucket's END, never an average), written as an argMax inside the bucket
    -- rather than as an ASOF JOIN. The two are the same value: the newest
    -- observation <= bucket end that is not older than the bucket start IS the
    -- bucket's last observation. Writing it this way makes the STALENESS WINDOW
    -- the bucket itself and costs nothing — the right side collapses ~87k
    -- observations to one row per bucket before the join, instead of ASOF-ing
    -- every candle row on the left. An unbounded ASOF would forward-fill a dead
    -- oracle's last reading across years of buckets; this cannot.
    --
    -- argMax is a LAST, not a mean — averaging is forbidden by 0167 because it
    -- does not compose across the six grains. This form composes WHERE THE
    -- ORACLE OBSERVED: when the day's last candle-bearing hour holds a reading,
    -- the daily close and that hour's close are the SAME observation, and are
    -- therefore the same value.
    --
    -- ⚠️ It does NOT compose across an oracle gap, and that is a property of the
    -- bucket-width window rather than an oversight. If the day's last reading
    -- falls in an EARLIER hour than the day's last candle, the daily bucket
    -- still contains that reading while the final hourly bucket does not — so
    -- the day publishes the measured rate ('oracle') while the last hour falls
    -- back to $1 ('peg'). Measured on the prod pin: a single reading at 09:05
    -- with hourly candles at 09:00 and 23:00 gives daily 0.9993/'oracle'
    -- against a last-hourly 1/'peg'. Both values are correct under the rule
    -- above and both are labelled, but a consumer comparing the two grains
    -- across an outage will see them differ. Closing that needs a rule that
    -- reaches OUTSIDE the bucket — i.e. the forward-fill this deliberately
    -- refuses. Pinned as expected behaviour by views_it.rs::
    -- a_day_whose_last_candle_hour_holds_no_reading_diverges_between_grains.
    --
    -- The method predicate selects a MEASURED reading. usd_rate keys on
    -- (identity, timestamp, method) precisely so a task 0154 'pivot' row cannot
    -- silently replace a measurement; the consumer chooses, and this consumer
    -- chooses measured or nothing. Same choice as current.sql's tip surface.
    --
    -- Task 0267 widened "measured" from one word to two: 'external' is a reading
    -- IMPORTED from an outside USD series (init.sql's method vocabulary), which
    -- is evidence of the same standing as a poll and is admitted here. A DERIVED
    -- 'pivot'/'pivot2' rate still is not, and neither is the pre-promotion
    -- 'external-candidate' staging word, which no read predicate names.
    -- Where one bucket holds BOTH an oracle row and an imported one, oracle wins
    -- by the explicit rank in the argMax tuple below, never by timestamp.
    --
    -- Flooring uses toStartOfInterval, the SAME function the rollup MVs use to
    -- build these buckets (rollups.sql) — so the two agree under any server
    -- timezone rather than only under UTC.
    --
    -- ⚠️ TWO PROPERTIES OF THE BUCKET-WIDTH STALENESS WINDOW, both deliberate:
    --   * A bucket the oracle sat out entirely falls back to $1/'peg' rather
    --     than carrying the previous bucket's rate forward. Honest and labelled;
    --     the alternative is an unbounded forward-fill that would publish a dead
    --     oracle's last reading as a measurement for years.
    --   * The NEWEST bucket reads 'peg' until the first poll lands inside it —
    --     up to ~5 minutes (the oracle cadence) at the top of each hour for the
    --     _1h grain, and at the top of each UTC day for the daily one. It then
    --     flips to 'oracle'. That is a ~0.07% step on a partial bucket, and it
    --     is visible in `method`, so a consumer that cares can wait for it.
    --
    -- ✅ `/v1/assets/{id}/ohlcv`'s USDC peg series (task 0170,
    -- queries_ch.rs::ohlcv_peg_series) reads the same table and now applies the
    -- SAME rule — task 0246. It used to ASOF at the bucket's START with NO
    -- staleness bound, so (a) its daily "close" was the PREVIOUS day's last
    -- reading and (b) after an oracle outage it forward-filled the last known
    -- rate indefinitely, still labelled 'oracle'. Both surfaces now take the
    -- last observation inside the bucket, per the rule init.sql states for this
    -- consumer by name ("T is the BUCKET'S END") — which is also the only one
    -- under which a daily close can equal the last hourly close of that day at
    -- all (subject to the gap caveat above).
    --
    -- ⚠️ TWO DELIBERATE DIFFERENCES from `/ohlcv`, and neither is drift.
    --
    -- (1) THE ORACLE WINDOW. `/ohlcv` serves grains this view does not, down to
    -- `1m`, and a 1-minute bucket is NARROWER than the oracle's 5-minute poll
    -- cadence — so scoping strictly to the bucket there would leave ~4 buckets
    -- in 5 on the $1 fallback and turn the series into a square wave. Its
    -- window is therefore max(bucket, 300 s), 300 s being enrichment's own
    -- FORWARD_FILL_WINDOW_S. At `1h` and `1d` — the only grains these views
    -- have — max(bucket, 300 s) IS the bucket, so the two agree exactly
    -- wherever they are comparable.
    --
    -- (2) THE IMPORTED ROW'S WINDOW (task 0267). `/ohlcv` additionally floors
    -- an `external` row at `toStartOfDay(bkt, 'UTC')`, so ONE imported row is
    -- valid for the whole UTC day it is stamped on. These views do not: they
    -- bucket an imported row exactly like a poll, at the bucket it falls in.
    --
    -- ⚠️ That is a SAFETY NET on `/ohlcv`, not a difference in what production
    -- serves. Since the 2026-09-09 hourly decision the loader runs at BOTH
    -- grains (`load-external-rate --grain daily|hourly`), so `usd_rate` holds
    -- an `external` row for EVERY HOUR of every covered day and both surfaces
    -- resolve the same row for the same bucket. The two therefore agree, which
    -- is what task 0246's cross-surface criterion asserts. The net is
    -- observable in two cases. One: someone loads the daily file alone and not
    -- the hourly one — in which case `/ohlcv` publishes the imported rate for
    -- all 24 hours of a day and `price_usd_series_1h` publishes it for the 00:00
    -- hour and `1`/'peg' for the other twenty-three. Two, and the reason
    -- `/ohlcv` now bounds its net by the bucket's END: on the oracle epoch's own
    -- day the import ends at 13:00 and the epoch is 14:00, so a bucket reaching
    -- past 14:00 with no poll inside it fell into the day-wide net and was
    -- published as a measurement the imported series does not hold — at 1h, and
    -- at 1d/1w/1M for any bucket the oracle rank did not take. That bound is
    -- `bo.bend <= toDateTime(USDC_ORACLE_EPOCH_S)` (rendered as the literal
    -- 1773237600) in `peg_series_sql`, which builds `ohlcv_peg_series`.
    -- The net exists because task 0268's
    -- external enrichment tier prices every candle of an imported day from that
    -- one daily row, and `/ohlcv` must not contradict the candles beside it.
    --
    -- Widening this view with a UNION ALL / ARRAY JOIN over the 24 hours of an
    -- imported day was considered and REJECTED (task 0267, review round 2
    -- WR-09): hourly rows make it unnecessary, and it would have added a second
    -- rate shape to a view whose whole job is to be the boring one.
    --
    -- Pinned by ohlcv_agrees_with_price_usd_series_on_the_same_bucket
    -- (ohlcv_it.rs), which compares the two surfaces against each other rather
    -- than against literals, over a fixture that now holds imported hours as
    -- well as polls, and — without a ClickHouse — by
    -- the_views_and_the_peg_series_admit_the_same_external_rows
    -- (prices-api queries_ch.rs), which pins that the two spell the predicate
    -- the same way.
    LEFT JOIN
    (
        SELECT
            CAST(asset_kind AS String) AS asset_kind,
            asset_code,
            issuer_address,
            contract_address,
            toStartOfInterval(timestamp, INTERVAL 1 DAY) AS bucket,
            -- ⚠️ THE RANK COMES FIRST IN THE TUPLE, and that ordering is the
            -- whole preference rule (task 0267). argMax over a TUPLE compares
            -- element by element, so `(rank, timestamp)` means: any 'oracle' row
            -- in the bucket beats EVERY imported row regardless of when each was
            -- observed, and the timestamp only breaks ties WITHIN one method.
            -- Keying on the timestamp alone -- the shape this was before the
            -- widening -- would let a backfilled import land later in the day
            -- than the last poll and silently outrank a measured reading.
            -- argMax over a tuple is an idiom this codebase already ships; see
            -- the two-key argMaxIf in prices-api queries_ch.rs.
            argMax(usd_rate, (if(method = 'oracle', 1, 0), timestamp)) AS usd_rate,
            argMax(method,   (if(method = 'oracle', 1, 0), timestamp)) AS rate_method
        FROM prices.usd_rate FINAL
        WHERE method IN ('oracle', 'external')
        GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket
    ) AS r
        ON  r.asset_kind       = b.asset_kind
        AND r.asset_code       = b.asset_code
        AND r.issuer_address   = b.issuer_address
        AND r.contract_address = b.contract_address
        AND r.bucket           = b.bucket
)
GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket
)
-- THE GATE (task 0147, D-04). A bucket is published only when its priced,
-- convertible volume is most of what traded AND is worth something absolute.
-- The peg disjunct comes FIRST and is not subject to the gate: arm B's
-- placeholder has ew = 0 and pw = 0 by construction, so a naive `ew > 0`
-- would delete USDC's fallback row. It tests `pw = 0` — NO PRICED WEIGHT —
-- and not "no traded weight", which splits a peg member that ALSO trades as a
-- base into two cases:
--   * base volume PARTLY OR WHOLLY PRICED -> pw > 0, the disjunct does not
--     fire, the gate governs the bucket, and if the gate withholds it the row
--     is ABSENT rather than falling back to $1 (a regression dressed as a fix).
--   * base volume ENTIRELY UNPRICED -> pw = 0, so the disjunct DOES fire and
--     the bucket publishes the measured rate or the $1 fallback, method 'peg',
--     share 1 — measured with 100,000 units of eligible unpriced USDC base
--     volume. Not a regression (pre-0147 that bucket had sum(w) = 0 and took
--     the same fallback), but the coverage row then reads `priced` while none
--     of the traded volume was priced. Making those buckets ABSENT instead
--     means `peg_present = 1 AND ew = 0`; that is a behaviour change, it needs
--     a decision, and phase 1 deliberately did not take it.
-- ⚠️ This is the WITHHOLDING rule, not the arithmetic guard. The guard lives
-- inside the CAST above, because a `nullIf` inside a non-Nullable CAST
-- publishes Decimal128::MIN or raises code 349 depending on which expression
-- JIT the server picked. Write both.
WHERE (peg_present = 1 AND pw = 0)
   OR (ew > 0 AND pw > 0 AND pw / ew >= 0.5);

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
    CAST(sum(toFloat64(p.close) * toFloat64(p.pf_volume)) / nullIf(sum(toFloat64(p.pf_volume)), 0) AS Decimal(38, 14)) AS xlm_usd
FROM prices.price_ohlcv_1h AS p FINAL
INNER JOIN prices.assets AS base  FINAL ON base.asset_id  = p.asset_id
INNER JOIN prices.assets AS quote FINAL ON quote.asset_id = p.quote_asset_id
WHERE base.asset_code = 'XLM' AND base.issuer_address = '' AND base.contract_address = ''
  AND quote.asset_code = 'USDC'
  AND quote.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  -- Task 0147, D-01 — the SAME priced predicate the series arm A spells, minus
  -- the close_usd half: this surface reads `close` (always present from the
  -- backfill) and is independent of enrichment timing, so there is no USD leg
  -- to floor and no convertibility to test. What it does inherit is the
  -- precision floor, the price-forming trade count and the price-forming
  -- weight.
  -- ⚠️ `pf_volume > 0` is the 0171 guard and is NOT optional: the CAST below
  -- still divides by the weight, and a bucket whose only XLM/USDC candles
  -- carry no weight must be ABSENT (`no_reference`), never a sentinel.
  AND p.close >= toDecimal128('0.000000000001', 14)
  AND p.pf_trade_count > 0
  AND p.pf_volume > 0
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
    close_usd,
    method,
    priced_volume_share
FROM
(
SELECT
    asset_kind,
    asset_code,
    issuer_address,
    contract_address,
    bucket,
    if(max(is_peg) = 1 AND sum(rpw) = 0,
       if(max(peg_rate) > 0, max(peg_rate), CAST(1 AS Decimal(38, 14))),
       CAST(if(sum(rpw) > 0, sum(rpv) / sum(rpw), toFloat64(0)) AS Decimal(38, 14))) AS close_usd,
    CAST(if(max(is_peg) = 1 AND sum(rpw) = 0,
            -- Three-way since task 0267, because a measured rate can now arrive
            -- from two provenances. ⚠️ The PEG DISCRIMINATOR STAYS FIRST and
            -- stays `max(peg_rate) <= 0`: arm B's join_use_nulls note explains
            -- that an unmatched LEFT JOIN yields the column DEFAULT rather than
            -- NULL on prod, so "this bucket has no rate at all" reads as 0 in
            -- peg_rate AND as 0 in rate_rank, and only the peg_rate test reads
            -- identically under both join_use_nulls settings. Ordering the rank
            -- test first would work by accident, not by rule.
            -- Only once a rate EXISTS does the rank say which provenance it came
            -- from: 2 = 'oracle', 1 = 'external'. See arm B for why the rank is
            -- numeric rather than a max() over the method string.
            multiIf(max(peg_rate) <= 0, 'peg', max(rate_rank) = 2, 'oracle', 'external'),
            'traded') AS LowCardinality(String)) AS method,
    -- Task 0147, D-05 — APPENDED LAST, after `method`. Arity changes, order
    -- does not; see the JOIN interop contract above for what that costs a
    -- consumer decoding positionally off `SELECT *`.
    -- ⚠️ NEVER NULL (D-06). BE renders a NULL as a dash and drops the pool, so
    -- the guard is an `if`, not a `nullIf`: a peg-arm bucket publishes a
    -- literal 1 (it has no traded weight to be short of), and the division is
    -- guarded INSIDE the CAST's argument.
    CAST(if(max(is_peg) = 1 AND sum(rpw) = 0,
            toFloat64(1),
            if(sum(rew) > 0, sum(rpw) / sum(rew), toFloat64(0))) AS Decimal(10, 6)) AS priced_volume_share,
    -- The gate's inputs, consumed by the outer WHERE and projected away there.
    max(is_peg) AS peg_present,
    sum(rpv)    AS pv,
    sum(rpw)    AS pw,
    sum(rew)    AS ew
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
        -- ONE priced predicate (task 0147, D-01): `/ohlcv`'s `valid` for the
        -- SAME row (queries_ch.rs::usd_projection) PLUS a positive
        -- price-forming WEIGHT. The shared terms are the 1e-12 precision floor
        -- on BOTH price columns, a price-forming trade and convertibility;
        -- `p.pf_volume > 0` is the extra one (tasks 0171/0198), and it is here
        -- because this surface WEIGHTS by that column and `/ohlcv` does not —
        -- a zero-weight row cannot move a weighted mean, but it CAN empty the
        -- denominator. Decided by brief D-01, so the two are the same rule and
        -- not the same expression: do not call them identical.
        -- Pinned against prices-api by
        -- views_sql_every_weighted_surface_spells_the_one_priced_predicate.
        (p.close >= toDecimal128('0.000000000001', 14)
            AND p.close_usd >= toDecimal128('0.000000000001', 14)
            AND p.pf_trade_count > 0
            AND p.pf_volume > 0
            AND (p.quote_asset_id IN
                 (
                     SELECT u.asset_id
                     FROM prices.assets AS u FINAL
                     WHERE u.asset_code = 'USDC'
                       AND u.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
                       AND u.contract_address = ''
                 )
                 OR p.close_usd != p.close))                AS is_priced,
        -- ELIGIBLE (task 0147, D-02) — computed here, no new table. The
        -- denominator of the coverage share is every unit traded against a
        -- quote we could IN PRINCIPLE price in USD: the canonical USDC, native
        -- XLM and the canonical USDT identities, plus every asset that has a
        -- row in prices.usd_rate under ANY method.
        -- ⚠️ usd_rate keys on the NATURAL IDENTITY, never on asset_id, so this
        -- is a four-column identity join and the usd_rate side needs
        -- CAST(asset_kind AS String) exactly as arm B's rate subquery does.
        -- ⚠️ Membership is RETROACTIVE: a bucket stops reading `unpriceable`
        -- the moment a rate appears for its quote leg. Accepted by ADR 0292.
        -- ⚠️ This flag is the QUOTE-SET half of the denominator only. The
        -- weight below is `is_priced OR is_eligible`, because a row we DID
        -- price is eligible whatever its quote leg is — see the note on `rew`.
        (p.quote_asset_id IN
         (
             SELECT e.asset_id
             FROM prices.assets AS e FINAL
             WHERE (e.asset_code = 'USDC' AND e.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN' AND e.contract_address = '')
                OR (e.asset_code = 'XLM'  AND e.issuer_address = '' AND e.contract_address = '')
                OR (e.asset_code = 'USDT' AND e.issuer_address = 'GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V' AND e.contract_address = '')
             UNION ALL
             SELECT ei.asset_id
             FROM
             (
                 SELECT
                     ea.asset_id AS asset_id,
                     multiIf(
                         ea.contract_address != '', 'contract',
                         ea.asset_code = 'XLM' AND ea.issuer_address = '', 'native',
                         'credit') AS asset_kind,
                     if(ea.contract_address != '', '', ea.asset_code)     AS asset_code,
                     if(ea.contract_address != '', '', ea.issuer_address) AS issuer_address,
                     ea.contract_address AS contract_address
                 FROM prices.assets AS ea FINAL
             ) AS ei
             INNER JOIN
             (
                 SELECT DISTINCT
                     CAST(asset_kind AS String) AS asset_kind,
                     asset_code,
                     issuer_address,
                     contract_address
                 FROM prices.usd_rate FINAL
             ) AS er
                 ON  er.asset_kind       = ei.asset_kind
                 AND er.asset_code       = ei.asset_code
                 AND er.issuer_address   = ei.issuer_address
                 AND er.contract_address = ei.contract_address
         ))                                                 AS is_eligible,
        -- Conditional sums, NOT a WHERE (task 0147). The unpriced rows are real
        -- trades and belong in the denominator — filtering them out before the
        -- weighting is exactly what let a 0.764-unit dust print become the
        -- whole of a bucket's weight.
        -- ⚠️ toFloat64 PER ROW before multiplying: Decimal x Decimal / Decimal
        -- raises code 407 (DECIMAL_OVERFLOW) on 26.3.10.60, measured twice.
        if(is_priced,   toFloat64(p.close_usd) * toFloat64(p.pf_volume), toFloat64(0)) AS rpv,
        if(is_priced,   toFloat64(p.pf_volume),                          toFloat64(0)) AS rpw,
        if(is_priced,   toFloat64(p.volume_quote_usd),                   toFloat64(0)) AS rpusd,
        -- ⚠️ `is_priced OR is_eligible`, NOT `is_eligible` alone. The priced set
        -- is not a subset of the eligible one by definition: `is_priced` admits
        -- a row on `close_usd != close` for ANY quote, while `is_eligible` names
        -- a QUOTE SET. A row priced against a quote outside that set would land
        -- in the numerator and not in the denominator, so the share could leave
        -- [0, 1] entirely (measured on 26.3.10.60: 5000000 published in a
        -- Decimal(10, 6) column this file documents as [0, 1] — CAST does not
        -- range-check P, so nothing raises). Not reachable through today's
        -- write path; made unreachable BY CONSTRUCTION here rather than by a
        -- clamp, so `priced ⊆ eligible` holds whatever a future writer does.
        -- Pinned by views_it.rs::
        -- a_priced_row_outside_the_eligible_quote_set_stays_inside_the_share.
        if(is_priced OR is_eligible, toFloat64(p.pf_volume),             toFloat64(0)) AS rew,
        toUInt8(0)                                        AS is_peg,
        CAST(0 AS Decimal(38, 14))                        AS peg_rate,
        -- UNION ALL matches arms POSITIONALLY and requires an identical column
        -- count, so this placeholder is structural, not decoration (task 0267).
        toUInt8(0)                                        AS rate_rank
    FROM prices.price_ohlcv_1h AS p FINAL
    INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id

    UNION ALL

    -- Arm B — zero-weight peg placeholder, keyed on the QUOTE leg, carrying
    -- that identity's MEASURED USD rate for the bucket (task 0168; 0 = none).
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
        b.asset_kind,
        b.asset_code,
        b.issuer_address,
        b.contract_address,
        b.bucket,
        toUInt8(0)   AS is_priced,
        toUInt8(0)   AS is_eligible,
        toFloat64(0) AS rpv,
        toFloat64(0) AS rpw,
        toFloat64(0) AS rpusd,
        toFloat64(0) AS rew,
        toUInt8(1)   AS is_peg,
        -- 0 = "no measured rate for this identity in this bucket" -> the $1
        -- fallback, flagged method = 'peg'. ⚠️ NOT NULL: prod runs
        -- join_use_nulls = 0, so an unmatched LEFT JOIN yields the column
        -- DEFAULT (0 for a Decimal), never NULL. A coalesce()/IS NULL form here
        -- would be DEAD CODE on prod. ifNull() only covers a session that sets
        -- join_use_nulls = 1; the `> 0` test in the outer SELECT is the real
        -- discriminator, and it reads the same under both settings.
        ifNull(r.usd_rate, CAST(0 AS Decimal(38, 14))) AS peg_rate,
        -- Task 0267. NUMERIC rank, never a max() over the method STRING: that
        -- would only work by the lexicographic accident that 'oracle' sorts
        -- above 'external', which is a property of the English words and not of
        -- the precedence rule. 2 beats 1 by arithmetic.
        -- Same join_use_nulls rule as peg_rate above: an unmatched LEFT JOIN
        -- yields the column DEFAULT, so the rank arrives as 0 (not NULL) and the
        -- ifNull() covers only a session that sets join_use_nulls = 1.
        multiIf(ifNull(r.rate_method, '') = 'oracle', 2,
                ifNull(r.rate_method, '') = 'external', 1,
                0) AS rate_rank
    FROM
    (
        SELECT
            multiIf(
                q.contract_address != '', 'contract',
                q.asset_code = 'XLM' AND q.issuer_address = '', 'native',
                'credit') AS asset_kind,
            if(q.contract_address != '', '', q.asset_code)     AS asset_code,
            if(q.contract_address != '', '', q.issuer_address) AS issuer_address,
            q.contract_address AS contract_address,
            p.timestamp        AS bucket
        FROM prices.price_ohlcv_1h AS p FINAL
        INNER JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
        WHERE q.contract_address = ''
          -- USDC only. USDT was removed here by task 0172: the canonical Stellar
          -- USDT depegged in June 2022 and trades at ~$0.13, so the $1 placeholder
          -- published a 7.4x overstatement. It is priced by measurement instead.
          -- ⚠️ THIS PREDICATE IS THE PEG SET. The rate join below follows
          -- whatever identity passes it, so adding a member here is a claim that
          -- the oracle prices THAT ISSUER — see the fence at the top of this file
          -- and `peg_identities_is_exactly_canonical_usdc` (oracle-worker).
          AND (q.asset_code = 'USDC' AND q.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN')
    ) AS b
    -- Task 0168 — the measured rate, one row per (identity, bucket).
    --
    -- This IS task 0167's resolution rule (newest observation at or before the
    -- bucket's END, never an average), written as an argMax inside the bucket
    -- rather than as an ASOF JOIN. The two are the same value: the newest
    -- observation <= bucket end that is not older than the bucket start IS the
    -- bucket's last observation. Writing it this way makes the STALENESS WINDOW
    -- the bucket itself and costs nothing — the right side collapses ~87k
    -- observations to one row per bucket before the join, instead of ASOF-ing
    -- every candle row on the left. An unbounded ASOF would forward-fill a dead
    -- oracle's last reading across years of buckets; this cannot.
    --
    -- argMax is a LAST, not a mean — averaging is forbidden by 0167 because it
    -- does not compose across the six grains. This form composes WHERE THE
    -- ORACLE OBSERVED: when the day's last candle-bearing hour holds a reading,
    -- the daily close and that hour's close are the SAME observation, and are
    -- therefore the same value.
    --
    -- ⚠️ It does NOT compose across an oracle gap, and that is a property of the
    -- bucket-width window rather than an oversight. If the day's last reading
    -- falls in an EARLIER hour than the day's last candle, the daily bucket
    -- still contains that reading while the final hourly bucket does not — so
    -- the day publishes the measured rate ('oracle') while the last hour falls
    -- back to $1 ('peg'). Measured on the prod pin: a single reading at 09:05
    -- with hourly candles at 09:00 and 23:00 gives daily 0.9993/'oracle'
    -- against a last-hourly 1/'peg'. Both values are correct under the rule
    -- above and both are labelled, but a consumer comparing the two grains
    -- across an outage will see them differ. Closing that needs a rule that
    -- reaches OUTSIDE the bucket — i.e. the forward-fill this deliberately
    -- refuses. Pinned as expected behaviour by views_it.rs::
    -- a_day_whose_last_candle_hour_holds_no_reading_diverges_between_grains.
    --
    -- The method predicate selects a MEASURED reading. usd_rate keys on
    -- (identity, timestamp, method) precisely so a task 0154 'pivot' row cannot
    -- silently replace a measurement; the consumer chooses, and this consumer
    -- chooses measured or nothing. Same choice as current.sql's tip surface.
    --
    -- Task 0267 widened "measured" from one word to two: 'external' is a reading
    -- IMPORTED from an outside USD series (init.sql's method vocabulary), which
    -- is evidence of the same standing as a poll and is admitted here. A DERIVED
    -- 'pivot'/'pivot2' rate still is not, and neither is the pre-promotion
    -- 'external-candidate' staging word, which no read predicate names.
    -- Where one bucket holds BOTH an oracle row and an imported one, oracle wins
    -- by the explicit rank in the argMax tuple below, never by timestamp.
    --
    -- Flooring uses toStartOfInterval, the SAME function the rollup MVs use to
    -- build these buckets (rollups.sql) — so the two agree under any server
    -- timezone rather than only under UTC.
    --
    -- ⚠️ TWO PROPERTIES OF THE BUCKET-WIDTH STALENESS WINDOW, both deliberate:
    --   * A bucket the oracle sat out entirely falls back to $1/'peg' rather
    --     than carrying the previous bucket's rate forward. Honest and labelled;
    --     the alternative is an unbounded forward-fill that would publish a dead
    --     oracle's last reading as a measurement for years.
    --   * The NEWEST bucket reads 'peg' until the first poll lands inside it —
    --     up to ~5 minutes (the oracle cadence) at the top of each hour for the
    --     _1h grain, and at the top of each UTC day for the daily one. It then
    --     flips to 'oracle'. That is a ~0.07% step on a partial bucket, and it
    --     is visible in `method`, so a consumer that cares can wait for it.
    --
    -- ✅ `/v1/assets/{id}/ohlcv`'s USDC peg series (task 0170,
    -- queries_ch.rs::ohlcv_peg_series) reads the same table and now applies the
    -- SAME rule — task 0246. It used to ASOF at the bucket's START with NO
    -- staleness bound, so (a) its daily "close" was the PREVIOUS day's last
    -- reading and (b) after an oracle outage it forward-filled the last known
    -- rate indefinitely, still labelled 'oracle'. Both surfaces now take the
    -- last observation inside the bucket, per the rule init.sql states for this
    -- consumer by name ("T is the BUCKET'S END") — which is also the only one
    -- under which a daily close can equal the last hourly close of that day at
    -- all (subject to the gap caveat above).
    --
    -- ⚠️ TWO DELIBERATE DIFFERENCES from `/ohlcv`, and neither is drift.
    --
    -- (1) THE ORACLE WINDOW. `/ohlcv` serves grains this view does not, down to
    -- `1m`, and a 1-minute bucket is NARROWER than the oracle's 5-minute poll
    -- cadence — so scoping strictly to the bucket there would leave ~4 buckets
    -- in 5 on the $1 fallback and turn the series into a square wave. Its
    -- window is therefore max(bucket, 300 s), 300 s being enrichment's own
    -- FORWARD_FILL_WINDOW_S. At `1h` and `1d` — the only grains these views
    -- have — max(bucket, 300 s) IS the bucket, so the two agree exactly
    -- wherever they are comparable.
    --
    -- (2) THE IMPORTED ROW'S WINDOW (task 0267). `/ohlcv` additionally floors
    -- an `external` row at `toStartOfDay(bkt, 'UTC')`, so ONE imported row is
    -- valid for the whole UTC day it is stamped on. These views do not: they
    -- bucket an imported row exactly like a poll, at the bucket it falls in.
    --
    -- ⚠️ That is a SAFETY NET on `/ohlcv`, not a difference in what production
    -- serves. Since the 2026-09-09 hourly decision the loader runs at BOTH
    -- grains (`load-external-rate --grain daily|hourly`), so `usd_rate` holds
    -- an `external` row for EVERY HOUR of every covered day and both surfaces
    -- resolve the same row for the same bucket. The two therefore agree, which
    -- is what task 0246's cross-surface criterion asserts. The net is
    -- observable in two cases. One: someone loads the daily file alone and not
    -- the hourly one — in which case `/ohlcv` publishes the imported rate for
    -- all 24 hours of a day and `price_usd_series_1h` publishes it for the 00:00
    -- hour and `1`/'peg' for the other twenty-three. Two, and the reason
    -- `/ohlcv` now bounds its net by the bucket's END: on the oracle epoch's own
    -- day the import ends at 13:00 and the epoch is 14:00, so a bucket reaching
    -- past 14:00 with no poll inside it fell into the day-wide net and was
    -- published as a measurement the imported series does not hold — at 1h, and
    -- at 1d/1w/1M for any bucket the oracle rank did not take. That bound is
    -- `bo.bend <= toDateTime(USDC_ORACLE_EPOCH_S)` (rendered as the literal
    -- 1773237600) in `peg_series_sql`, which builds `ohlcv_peg_series`.
    -- The net exists because task 0268's
    -- external enrichment tier prices every candle of an imported day from that
    -- one daily row, and `/ohlcv` must not contradict the candles beside it.
    --
    -- Widening this view with a UNION ALL / ARRAY JOIN over the 24 hours of an
    -- imported day was considered and REJECTED (task 0267, review round 2
    -- WR-09): hourly rows make it unnecessary, and it would have added a second
    -- rate shape to a view whose whole job is to be the boring one.
    --
    -- Pinned by ohlcv_agrees_with_price_usd_series_on_the_same_bucket
    -- (ohlcv_it.rs), which compares the two surfaces against each other rather
    -- than against literals, over a fixture that now holds imported hours as
    -- well as polls, and — without a ClickHouse — by
    -- the_views_and_the_peg_series_admit_the_same_external_rows
    -- (prices-api queries_ch.rs), which pins that the two spell the predicate
    -- the same way.
    LEFT JOIN
    (
        SELECT
            CAST(asset_kind AS String) AS asset_kind,
            asset_code,
            issuer_address,
            contract_address,
            toStartOfInterval(timestamp, INTERVAL 1 HOUR) AS bucket,
            -- ⚠️ THE RANK COMES FIRST IN THE TUPLE, and that ordering is the
            -- whole preference rule (task 0267). argMax over a TUPLE compares
            -- element by element, so `(rank, timestamp)` means: any 'oracle' row
            -- in the bucket beats EVERY imported row regardless of when each was
            -- observed, and the timestamp only breaks ties WITHIN one method.
            -- Keying on the timestamp alone -- the shape this was before the
            -- widening -- would let a backfilled import land later in the hour
            -- than the last poll and silently outrank a measured reading.
            -- argMax over a tuple is an idiom this codebase already ships; see
            -- the two-key argMaxIf in prices-api queries_ch.rs.
            argMax(usd_rate, (if(method = 'oracle', 1, 0), timestamp)) AS usd_rate,
            argMax(method,   (if(method = 'oracle', 1, 0), timestamp)) AS rate_method
        FROM prices.usd_rate FINAL
        WHERE method IN ('oracle', 'external')
        GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket
    ) AS r
        ON  r.asset_kind       = b.asset_kind
        AND r.asset_code       = b.asset_code
        AND r.issuer_address   = b.issuer_address
        AND r.contract_address = b.contract_address
        AND r.bucket           = b.bucket
)
GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket
)
-- THE GATE (task 0147, D-04). A bucket is published only when its priced,
-- convertible volume is most of what traded AND is worth something absolute.
-- The peg disjunct comes FIRST and is not subject to the gate: arm B's
-- placeholder has ew = 0 and pw = 0 by construction, so a naive `ew > 0`
-- would delete USDC's fallback row. It tests `pw = 0` — NO PRICED WEIGHT —
-- and not "no traded weight", which splits a peg member that ALSO trades as a
-- base into two cases:
--   * base volume PARTLY OR WHOLLY PRICED -> pw > 0, the disjunct does not
--     fire, the gate governs the bucket, and if the gate withholds it the row
--     is ABSENT rather than falling back to $1 (a regression dressed as a fix).
--   * base volume ENTIRELY UNPRICED -> pw = 0, so the disjunct DOES fire and
--     the bucket publishes the measured rate or the $1 fallback, method 'peg',
--     share 1 — measured with 100,000 units of eligible unpriced USDC base
--     volume. Not a regression (pre-0147 that bucket had sum(w) = 0 and took
--     the same fallback), but the coverage row then reads `priced` while none
--     of the traded volume was priced. Making those buckets ABSENT instead
--     means `peg_present = 1 AND ew = 0`; that is a behaviour change, it needs
--     a decision, and phase 1 deliberately did not take it.
-- ⚠️ This is the WITHHOLDING rule, not the arithmetic guard. The guard lives
-- inside the CAST above, because a `nullIf` inside a non-Nullable CAST
-- publishes Decimal128::MIN or raises code 349 depending on which expression
-- JIT the server picked. Write both.
WHERE (peg_present = 1 AND pw = 0)
   OR (ew > 0 AND pw > 0 AND pw / ew >= 0.5);


----------------------------------------------------------------------
-- prices.price_usd_series_coverage{,_1h} — why a bucket is not published
-- (task 0147, D-05).
--
-- The series views keep their value-or-absent contract, so a bucket the gate
-- withholds is simply MISSING — indistinguishable, on that surface alone, from
-- an asset that never traded. These views close that: one row per (natural
-- identity, bucket) the candles hold, carrying the same `priced_volume_share`
-- the series publishes, the priced USD volume (published for the consumer; no
-- gate reads it), and a status.
--
--   status = priced       -- the series publishes this bucket
--          | pending      -- eligible price-forming volume exists, but the
--                         --   share is below X. PENDING
--                         --   ENRICHMENT, not unpriceable: enrich the rest of
--                         --   the bucket and it publishes.
--          | unpriceable  -- NO ELIGIBLE PRICE-FORMING VOLUME IN THE BUCKET —
--                         --   `sum(rew) = 0`, i.e. the pf_volume of every row
--                         --   we could in principle price sums to zero.
--                         --   TWO different buckets read this one word:
--                         --     * NO USD PATH — every row is quoted in an
--                         --       asset we cannot price. RETROACTIVE: this
--                         --       one flips to `pending` the moment a
--                         --       prices.usd_rate row appears for that quote
--                         --       identity (ADR 0292).
--                         --     * NO PRICE-FORMING VOLUME — the quote IS
--                         --       eligible (canonical USDC, even), but every
--                         --       candle of the bucket is stroop-dust, so
--                         --       pf_volume sums to 0 and there is no
--                         --       population for a share to be OF. No
--                         --       usd_rate row will ever change this verdict;
--                         --       only a real fill will. Task 0286 created
--                         --       this class deliberately — post-0286 it is
--                         --       ordinary, not exotic.
--                         --   So the word means "nothing here we could price",
--                         --   NOT "no USD path". Read `pf_volume` on the
--                         --   candles to tell the two apart; the status word
--                         --   does not.
--
-- ⚠️ A PEG-ARM bucket reads `priced` with `priced_volume_share = 1` and
-- `priced_volume_usd = 0`. Its value comes from the measured rate (or the $1
-- fallback), not from traded weight, so there is no traded population for the
-- share to describe and the gate does not apply to it. Read `method` on the
-- series view, not the share, to tell those buckets apart.
--
-- ⚠️ The peg arm is chosen on `pw = 0` — NO PRICED WEIGHT — not on `ew = 0`,
-- so a peg member that ALSO trades as a base with its base volume entirely
-- UNPRICED lands here too: it reads `priced` with share 1 while none of its
-- traded volume was priced. Measured on 26.3.10.60: 100,000 units of eligible,
-- entirely-unpriced USDC base volume publishes 1/'peg' and reads
-- `priced`/share 1 here. Not a regression — pre-0147 that bucket had
-- sum(w) = 0 and took the same fallback — but the share on such a row is
-- describing the PEG statement about the identity, not the traded population
-- beside it. `priced_volume_usd = 0` is the tell.
--
-- ⚠️ `priced_volume_share` is never NULL here either (D-06): a bucket with no
-- eligible volume reads a literal 0, because BE renders a NULL as a dash and
-- drops the pool.
--
-- ⚠️ These views must never name `prices.price_usd_series` in executable SQL —
-- `series_grains()` in src/lib.rs takes the FIRST statement containing that
-- substring, and would silently start asserting against a coverage body.
----------------------------------------------------------------------

CREATE OR REPLACE VIEW prices.price_usd_series_coverage AS
SELECT
    asset_kind,
    asset_code,
    issuer_address,
    contract_address,
    bucket,
    -- The share and its numerator's USD value, for EVERY (identity, bucket) the
    -- candles hold — published or not. The same expression the series view
    -- publishes, so the two cannot disagree about a bucket they both describe.
    CAST(if(max(is_peg) = 1 AND sum(rpw) = 0,
            toFloat64(1),
            if(sum(rew) > 0, sum(rpw) / sum(rew), toFloat64(0))) AS Decimal(10, 6)) AS priced_volume_share,
    CAST(sum(rpusd) AS Decimal(38, 14)) AS priced_volume_usd,
    -- The peg discriminator comes FIRST, for the same reason it does in the
    -- series' `method`: an arm-B bucket has no eligible weight at all, so the
    -- `ew = 0` arm below would otherwise call a peg fill `unpriceable`.
    -- A peg-arm bucket reads `priced` with share 1 — it has no traded weight to
    -- be short of, and its value does not come from candles.
    -- The third arm restates the outer gate of the series view. It divides by
    -- `sum(rew)` only after the `sum(rew) = 0` arm has taken that case, and a
    -- Float64 division by zero yields inf/nan rather than raising — nothing
    -- here casts that into a Decimal.
    CAST(multiIf(max(is_peg) = 1 AND sum(rpw) = 0, 'priced',
                 sum(rew) = 0, 'unpriceable',
                 sum(rpw) > 0 AND sum(rpw) / sum(rew) >= 0.5, 'priced',
                 'pending') AS LowCardinality(String)) AS status
FROM
(
    -- Arm A — every candle of the bucket, keyed on the BASE leg, priced or not.
    -- Shared VERBATIM with the series view: one predicate, spelled once.
    SELECT
        multiIf(
            a.contract_address != '', 'contract',
            a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
            'credit') AS asset_kind,
        if(a.contract_address != '', '', a.asset_code)     AS asset_code,
        if(a.contract_address != '', '', a.issuer_address) AS issuer_address,
        a.contract_address AS contract_address,
        p.timestamp        AS bucket,
        -- ONE priced predicate (task 0147, D-01): `/ohlcv`'s `valid` for the
        -- SAME row (queries_ch.rs::usd_projection) PLUS a positive
        -- price-forming WEIGHT. The shared terms are the 1e-12 precision floor
        -- on BOTH price columns, a price-forming trade and convertibility;
        -- `p.pf_volume > 0` is the extra one (tasks 0171/0198), and it is here
        -- because this surface WEIGHTS by that column and `/ohlcv` does not —
        -- a zero-weight row cannot move a weighted mean, but it CAN empty the
        -- denominator. Decided by brief D-01, so the two are the same rule and
        -- not the same expression: do not call them identical.
        -- Pinned against prices-api by
        -- views_sql_every_weighted_surface_spells_the_one_priced_predicate.
        (p.close >= toDecimal128('0.000000000001', 14)
            AND p.close_usd >= toDecimal128('0.000000000001', 14)
            AND p.pf_trade_count > 0
            AND p.pf_volume > 0
            AND (p.quote_asset_id IN
                 (
                     SELECT u.asset_id
                     FROM prices.assets AS u FINAL
                     WHERE u.asset_code = 'USDC'
                       AND u.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
                       AND u.contract_address = ''
                 )
                 OR p.close_usd != p.close))                AS is_priced,
        -- ELIGIBLE (task 0147, D-02) — computed here, no new table. The
        -- denominator of the coverage share is every unit traded against a
        -- quote we could IN PRINCIPLE price in USD: the canonical USDC, native
        -- XLM and the canonical USDT identities, plus every asset that has a
        -- row in prices.usd_rate under ANY method.
        -- ⚠️ usd_rate keys on the NATURAL IDENTITY, never on asset_id, so this
        -- is a four-column identity join and the usd_rate side needs
        -- CAST(asset_kind AS String) exactly as arm B's rate subquery does.
        -- ⚠️ Membership is RETROACTIVE: a bucket stops reading `unpriceable`
        -- the moment a rate appears for its quote leg. Accepted by ADR 0292.
        -- ⚠️ This flag is the QUOTE-SET half of the denominator only. The
        -- weight below is `is_priced OR is_eligible`, because a row we DID
        -- price is eligible whatever its quote leg is — see the note on `rew`.
        (p.quote_asset_id IN
         (
             SELECT e.asset_id
             FROM prices.assets AS e FINAL
             WHERE (e.asset_code = 'USDC' AND e.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN' AND e.contract_address = '')
                OR (e.asset_code = 'XLM'  AND e.issuer_address = '' AND e.contract_address = '')
                OR (e.asset_code = 'USDT' AND e.issuer_address = 'GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V' AND e.contract_address = '')
             UNION ALL
             SELECT ei.asset_id
             FROM
             (
                 SELECT
                     ea.asset_id AS asset_id,
                     multiIf(
                         ea.contract_address != '', 'contract',
                         ea.asset_code = 'XLM' AND ea.issuer_address = '', 'native',
                         'credit') AS asset_kind,
                     if(ea.contract_address != '', '', ea.asset_code)     AS asset_code,
                     if(ea.contract_address != '', '', ea.issuer_address) AS issuer_address,
                     ea.contract_address AS contract_address
                 FROM prices.assets AS ea FINAL
             ) AS ei
             INNER JOIN
             (
                 SELECT DISTINCT
                     CAST(asset_kind AS String) AS asset_kind,
                     asset_code,
                     issuer_address,
                     contract_address
                 FROM prices.usd_rate FINAL
             ) AS er
                 ON  er.asset_kind       = ei.asset_kind
                 AND er.asset_code       = ei.asset_code
                 AND er.issuer_address   = ei.issuer_address
                 AND er.contract_address = ei.contract_address
         ))                                                 AS is_eligible,
        -- Conditional sums, NOT a WHERE (task 0147). The unpriced rows are real
        -- trades and belong in the denominator — filtering them out before the
        -- weighting is exactly what let a 0.764-unit dust print become the
        -- whole of a bucket's weight.
        -- ⚠️ toFloat64 PER ROW before multiplying: Decimal x Decimal / Decimal
        -- raises code 407 (DECIMAL_OVERFLOW) on 26.3.10.60, measured twice.
        if(is_priced,   toFloat64(p.close_usd) * toFloat64(p.pf_volume), toFloat64(0)) AS rpv,
        if(is_priced,   toFloat64(p.pf_volume),                          toFloat64(0)) AS rpw,
        if(is_priced,   toFloat64(p.volume_quote_usd),                   toFloat64(0)) AS rpusd,
        -- ⚠️ `is_priced OR is_eligible`, NOT `is_eligible` alone. The priced set
        -- is not a subset of the eligible one by definition: `is_priced` admits
        -- a row on `close_usd != close` for ANY quote, while `is_eligible` names
        -- a QUOTE SET. A row priced against a quote outside that set would land
        -- in the numerator and not in the denominator, so the share could leave
        -- [0, 1] entirely (measured on 26.3.10.60: 5000000 published in a
        -- Decimal(10, 6) column this file documents as [0, 1] — CAST does not
        -- range-check P, so nothing raises). Not reachable through today's
        -- write path; made unreachable BY CONSTRUCTION here rather than by a
        -- clamp, so `priced ⊆ eligible` holds whatever a future writer does.
        -- Pinned by views_it.rs::
        -- a_priced_row_outside_the_eligible_quote_set_stays_inside_the_share.
        if(is_priced OR is_eligible, toFloat64(p.pf_volume),             toFloat64(0)) AS rew,
        toUInt8(0)                                        AS is_peg,
        CAST(0 AS Decimal(38, 14))                        AS peg_rate,
        -- UNION ALL matches arms POSITIONALLY and requires an identical column
        -- count, so this placeholder is structural, not decoration (task 0267).
        toUInt8(0)                                        AS rate_rank
    FROM prices.price_ohlcv_1d AS p FINAL
    INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id

    UNION ALL

    -- Arm B — the same zero-weight peg placeholder the series view unions in,
    -- minus the rate join: coverage reports WHETHER a bucket is priced, never
    -- at what. `peg_rate` and `rate_rank` are structural — UNION ALL matches
    -- its arms positionally, and arm A above is shared verbatim with the
    -- series view.
    SELECT
        b.asset_kind,
        b.asset_code,
        b.issuer_address,
        b.contract_address,
        b.bucket,
        toUInt8(0)   AS is_priced,
        toUInt8(0)   AS is_eligible,
        toFloat64(0) AS rpv,
        toFloat64(0) AS rpw,
        toFloat64(0) AS rpusd,
        toFloat64(0) AS rew,
        toUInt8(1)   AS is_peg,
        CAST(0 AS Decimal(38, 14)) AS peg_rate,
        toUInt8(0)   AS rate_rank
    FROM
    (
        SELECT
            multiIf(
                q.contract_address != '', 'contract',
                q.asset_code = 'XLM' AND q.issuer_address = '', 'native',
                'credit') AS asset_kind,
            if(q.contract_address != '', '', q.asset_code)     AS asset_code,
            if(q.contract_address != '', '', q.issuer_address) AS issuer_address,
            q.contract_address AS contract_address,
            p.timestamp        AS bucket
        FROM prices.price_ohlcv_1d AS p FINAL
        INNER JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
        WHERE q.contract_address = ''
          -- USDC only. USDT was removed here by task 0172: the canonical Stellar
          -- USDT depegged in June 2022 and trades at ~$0.13, so the $1 placeholder
          -- published a 7.4x overstatement. It is priced by measurement instead.
          -- ⚠️ THIS PREDICATE IS THE PEG SET. The rate join below follows
          -- whatever identity passes it, so adding a member here is a claim that
          -- the oracle prices THAT ISSUER — see the fence at the top of this file
          -- and `peg_identities_is_exactly_canonical_usdc` (oracle-worker).
          AND (q.asset_code = 'USDC' AND q.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN')
    ) AS b
)
GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket;

CREATE OR REPLACE VIEW prices.price_usd_series_coverage_1h AS
SELECT
    asset_kind,
    asset_code,
    issuer_address,
    contract_address,
    bucket,
    -- The share and its numerator's USD value, for EVERY (identity, bucket) the
    -- candles hold — published or not. The same expression the series view
    -- publishes, so the two cannot disagree about a bucket they both describe.
    CAST(if(max(is_peg) = 1 AND sum(rpw) = 0,
            toFloat64(1),
            if(sum(rew) > 0, sum(rpw) / sum(rew), toFloat64(0))) AS Decimal(10, 6)) AS priced_volume_share,
    CAST(sum(rpusd) AS Decimal(38, 14)) AS priced_volume_usd,
    -- The peg discriminator comes FIRST, for the same reason it does in the
    -- series' `method`: an arm-B bucket has no eligible weight at all, so the
    -- `ew = 0` arm below would otherwise call a peg fill `unpriceable`.
    -- A peg-arm bucket reads `priced` with share 1 — it has no traded weight to
    -- be short of, and its value does not come from candles.
    -- The third arm restates the outer gate of the series view. It divides by
    -- `sum(rew)` only after the `sum(rew) = 0` arm has taken that case, and a
    -- Float64 division by zero yields inf/nan rather than raising — nothing
    -- here casts that into a Decimal.
    CAST(multiIf(max(is_peg) = 1 AND sum(rpw) = 0, 'priced',
                 sum(rew) = 0, 'unpriceable',
                 sum(rpw) > 0 AND sum(rpw) / sum(rew) >= 0.5, 'priced',
                 'pending') AS LowCardinality(String)) AS status
FROM
(
    -- Arm A — every candle of the bucket, keyed on the BASE leg, priced or not.
    -- Shared VERBATIM with the series view: one predicate, spelled once.
    SELECT
        multiIf(
            a.contract_address != '', 'contract',
            a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
            'credit') AS asset_kind,
        if(a.contract_address != '', '', a.asset_code)     AS asset_code,
        if(a.contract_address != '', '', a.issuer_address) AS issuer_address,
        a.contract_address AS contract_address,
        p.timestamp        AS bucket,
        -- ONE priced predicate (task 0147, D-01): `/ohlcv`'s `valid` for the
        -- SAME row (queries_ch.rs::usd_projection) PLUS a positive
        -- price-forming WEIGHT. The shared terms are the 1e-12 precision floor
        -- on BOTH price columns, a price-forming trade and convertibility;
        -- `p.pf_volume > 0` is the extra one (tasks 0171/0198), and it is here
        -- because this surface WEIGHTS by that column and `/ohlcv` does not —
        -- a zero-weight row cannot move a weighted mean, but it CAN empty the
        -- denominator. Decided by brief D-01, so the two are the same rule and
        -- not the same expression: do not call them identical.
        -- Pinned against prices-api by
        -- views_sql_every_weighted_surface_spells_the_one_priced_predicate.
        (p.close >= toDecimal128('0.000000000001', 14)
            AND p.close_usd >= toDecimal128('0.000000000001', 14)
            AND p.pf_trade_count > 0
            AND p.pf_volume > 0
            AND (p.quote_asset_id IN
                 (
                     SELECT u.asset_id
                     FROM prices.assets AS u FINAL
                     WHERE u.asset_code = 'USDC'
                       AND u.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
                       AND u.contract_address = ''
                 )
                 OR p.close_usd != p.close))                AS is_priced,
        -- ELIGIBLE (task 0147, D-02) — computed here, no new table. The
        -- denominator of the coverage share is every unit traded against a
        -- quote we could IN PRINCIPLE price in USD: the canonical USDC, native
        -- XLM and the canonical USDT identities, plus every asset that has a
        -- row in prices.usd_rate under ANY method.
        -- ⚠️ usd_rate keys on the NATURAL IDENTITY, never on asset_id, so this
        -- is a four-column identity join and the usd_rate side needs
        -- CAST(asset_kind AS String) exactly as arm B's rate subquery does.
        -- ⚠️ Membership is RETROACTIVE: a bucket stops reading `unpriceable`
        -- the moment a rate appears for its quote leg. Accepted by ADR 0292.
        -- ⚠️ This flag is the QUOTE-SET half of the denominator only. The
        -- weight below is `is_priced OR is_eligible`, because a row we DID
        -- price is eligible whatever its quote leg is — see the note on `rew`.
        (p.quote_asset_id IN
         (
             SELECT e.asset_id
             FROM prices.assets AS e FINAL
             WHERE (e.asset_code = 'USDC' AND e.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN' AND e.contract_address = '')
                OR (e.asset_code = 'XLM'  AND e.issuer_address = '' AND e.contract_address = '')
                OR (e.asset_code = 'USDT' AND e.issuer_address = 'GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V' AND e.contract_address = '')
             UNION ALL
             SELECT ei.asset_id
             FROM
             (
                 SELECT
                     ea.asset_id AS asset_id,
                     multiIf(
                         ea.contract_address != '', 'contract',
                         ea.asset_code = 'XLM' AND ea.issuer_address = '', 'native',
                         'credit') AS asset_kind,
                     if(ea.contract_address != '', '', ea.asset_code)     AS asset_code,
                     if(ea.contract_address != '', '', ea.issuer_address) AS issuer_address,
                     ea.contract_address AS contract_address
                 FROM prices.assets AS ea FINAL
             ) AS ei
             INNER JOIN
             (
                 SELECT DISTINCT
                     CAST(asset_kind AS String) AS asset_kind,
                     asset_code,
                     issuer_address,
                     contract_address
                 FROM prices.usd_rate FINAL
             ) AS er
                 ON  er.asset_kind       = ei.asset_kind
                 AND er.asset_code       = ei.asset_code
                 AND er.issuer_address   = ei.issuer_address
                 AND er.contract_address = ei.contract_address
         ))                                                 AS is_eligible,
        -- Conditional sums, NOT a WHERE (task 0147). The unpriced rows are real
        -- trades and belong in the denominator — filtering them out before the
        -- weighting is exactly what let a 0.764-unit dust print become the
        -- whole of a bucket's weight.
        -- ⚠️ toFloat64 PER ROW before multiplying: Decimal x Decimal / Decimal
        -- raises code 407 (DECIMAL_OVERFLOW) on 26.3.10.60, measured twice.
        if(is_priced,   toFloat64(p.close_usd) * toFloat64(p.pf_volume), toFloat64(0)) AS rpv,
        if(is_priced,   toFloat64(p.pf_volume),                          toFloat64(0)) AS rpw,
        if(is_priced,   toFloat64(p.volume_quote_usd),                   toFloat64(0)) AS rpusd,
        -- ⚠️ `is_priced OR is_eligible`, NOT `is_eligible` alone. The priced set
        -- is not a subset of the eligible one by definition: `is_priced` admits
        -- a row on `close_usd != close` for ANY quote, while `is_eligible` names
        -- a QUOTE SET. A row priced against a quote outside that set would land
        -- in the numerator and not in the denominator, so the share could leave
        -- [0, 1] entirely (measured on 26.3.10.60: 5000000 published in a
        -- Decimal(10, 6) column this file documents as [0, 1] — CAST does not
        -- range-check P, so nothing raises). Not reachable through today's
        -- write path; made unreachable BY CONSTRUCTION here rather than by a
        -- clamp, so `priced ⊆ eligible` holds whatever a future writer does.
        -- Pinned by views_it.rs::
        -- a_priced_row_outside_the_eligible_quote_set_stays_inside_the_share.
        if(is_priced OR is_eligible, toFloat64(p.pf_volume),             toFloat64(0)) AS rew,
        toUInt8(0)                                        AS is_peg,
        CAST(0 AS Decimal(38, 14))                        AS peg_rate,
        -- UNION ALL matches arms POSITIONALLY and requires an identical column
        -- count, so this placeholder is structural, not decoration (task 0267).
        toUInt8(0)                                        AS rate_rank
    FROM prices.price_ohlcv_1h AS p FINAL
    INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id

    UNION ALL

    -- Arm B — the same zero-weight peg placeholder the series view unions in,
    -- minus the rate join: coverage reports WHETHER a bucket is priced, never
    -- at what. `peg_rate` and `rate_rank` are structural — UNION ALL matches
    -- its arms positionally, and arm A above is shared verbatim with the
    -- series view.
    SELECT
        b.asset_kind,
        b.asset_code,
        b.issuer_address,
        b.contract_address,
        b.bucket,
        toUInt8(0)   AS is_priced,
        toUInt8(0)   AS is_eligible,
        toFloat64(0) AS rpv,
        toFloat64(0) AS rpw,
        toFloat64(0) AS rpusd,
        toFloat64(0) AS rew,
        toUInt8(1)   AS is_peg,
        CAST(0 AS Decimal(38, 14)) AS peg_rate,
        toUInt8(0)   AS rate_rank
    FROM
    (
        SELECT
            multiIf(
                q.contract_address != '', 'contract',
                q.asset_code = 'XLM' AND q.issuer_address = '', 'native',
                'credit') AS asset_kind,
            if(q.contract_address != '', '', q.asset_code)     AS asset_code,
            if(q.contract_address != '', '', q.issuer_address) AS issuer_address,
            q.contract_address AS contract_address,
            p.timestamp        AS bucket
        FROM prices.price_ohlcv_1h AS p FINAL
        INNER JOIN prices.assets AS q FINAL ON q.asset_id = p.quote_asset_id
        WHERE q.contract_address = ''
          -- USDC only. USDT was removed here by task 0172: the canonical Stellar
          -- USDT depegged in June 2022 and trades at ~$0.13, so the $1 placeholder
          -- published a 7.4x overstatement. It is priced by measurement instead.
          -- ⚠️ THIS PREDICATE IS THE PEG SET. The rate join below follows
          -- whatever identity passes it, so adding a member here is a claim that
          -- the oracle prices THAT ISSUER — see the fence at the top of this file
          -- and `peg_identities_is_exactly_canonical_usdc` (oracle-worker).
          AND (q.asset_code = 'USDC' AND q.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN')
    ) AS b
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
-- Same KEYING as price_usd_series (natural id, joined by the consumer's LEFT
-- JOIN) but for "now": one row per asset with the latest USD price +
-- `updated_at`. ⚠️ NOT the same missing-value contract: an asset with no priced
-- candle in the 24 h window is PRESENT here with `price_usd = 0` and
-- `method = ''` — a sentinel, where the series omit the row (the sentinel table
-- above; ADR 0292 §5, which also decides the additive `price_status` / `as_of`
-- fields that will make the 0 and a carried price legible). ⚠️ **`updated_at` is the MV's refresh time, not the
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
    c.method           AS method,
    c.as_of            AS as_of,
    c.price_status     AS price_status
FROM prices.current_prices AS c FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = c.asset_id;
