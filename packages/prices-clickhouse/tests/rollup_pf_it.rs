//! Price-forming rollup semantics, end-to-end on the real shipped SQL
//! (task 0286 / ADR 0287 §2–§5).
//!
//!     cargo test -p prices-clickhouse --test rollup_pf_it -- --ignored --test-threads=4
//!
//! A candle's prices come only from the price-forming trades of its own bucket.
//! One level up that means a child with `pf_trade_count = 0` — a minute whose
//! every fill was too small for its price to mean anything, written with
//! `open = high = low = close = 0` — must contribute to NO price aggregate of
//! its parent. Before task 0286 it contributed to all four: it was the `min` of
//! the bucket's `low` (a zero low on every coarse tier) and, whenever it landed
//! last, the `argMax` `close` as well.
//!
//! Every test here drives the FULL-RANGE pre-roll (`schema/preroll.sql`), not
//! the refreshable MVs: it is a deterministic front-to-back `INSERT … SELECT`
//! chain with no refresh scheduling to wait on, and it renders from the same
//! `rollup_sql::rollup_select` as the MV bodies — `lib.rs`'s file == generator
//! tests are what make "same SELECT" a fact rather than a hope. The MV form is
//! exercised by `rollup_chain_it.rs` and `rollup_append_it.rs`.
//!
//! Each test owns a scratch database and drops it (pattern:
//! `rollup_chain_it.rs::rewrite`). Runs on the pinned prod engine, ClickHouse
//! 26.3.10.60.

use clickhouse::Client;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Point the `prices.*` schema at an isolated scratch database.
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// Every coarse grain the pre-roll writes, in chain order.
const COARSE: &[&str] = &[
    "price_ohlcv_15m",
    "price_ohlcv_1h",
    "price_ohlcv_4h",
    "price_ohlcv_1d",
    "price_ohlcv_1w",
    "price_ohlcv_1M",
];

/// One `_1m` row, every one of the eighteen columns given explicitly.
///
/// Explicit on purpose: `pf_trade_count DEFAULT trade_count` means an omitted
/// pf column comes back as "every fill formed price" (BRIEF F6b), so a fixture
/// that leaves them out cannot express a dust-only minute at all.
struct Minute {
    ts: &'static str,
    open: &'static str,
    high: &'static str,
    low: &'static str,
    close: &'static str,
    volume_base: &'static str,
    volume_quote: &'static str,
    close_usd: &'static str,
    trade_count: u32,
    pf_trade_count: u32,
}

impl Minute {
    /// A minute with price-forming fills: prices meaningful, `pf_*` non-zero.
    const fn priced(
        ts: &'static str,
        open: &'static str,
        high: &'static str,
        low: &'static str,
        close: &'static str,
        close_usd: &'static str,
    ) -> Self {
        Self {
            ts,
            open,
            high,
            low,
            close,
            volume_base: "10",
            volume_quote: "12",
            close_usd,
            trade_count: 2,
            pf_trade_count: 2,
        }
    }

    /// The LEGACY shape task 0286's migration leaves behind (review B WR-01 /
    /// WR-02, C1): a pre-0286 row whose price underflowed `Decimal(38, 14)` and
    /// stored as `0`, reading `pf_trade_count` from its `DEFAULT trade_count`.
    /// It claims to be price-forming and has no price — the whole reason the
    /// coarse gate cannot be `pf_trade_count > 0` alone. The real row: local
    /// `price_ohlcv_1m`, 2026-04-02 06:39, asset 2643 / native / sdex,
    /// `volume_base = 33 387 840 110.63`.
    const fn legacy_unrepresentable(ts: &'static str) -> Self {
        Self {
            ts,
            open: "0",
            high: "0",
            low: "0",
            close: "0",
            volume_base: "33387840110.63",
            volume_quote: "0.0001622",
            close_usd: "0",
            trade_count: 1,
            pf_trade_count: 1,
        }
    }

    /// Review WR-03: a 1m child whose whole price band sits UNDER the precision
    /// floor — a handful of `Decimal(38, 14)` ticks, which is quantisation noise
    /// rather than a measurement. It passed a `close > 0` gate, so `minIf` gave
    /// its `low` to every coarse tier while the read path, which has always
    /// floored at `1e-12`, refused the same number. 73 such rows on the local
    /// verification database.
    const fn sub_floor(ts: &'static str) -> Self {
        Self {
            ts,
            open: "0.00000000000091",
            high: "0.00000000000091",
            low: "0.00000000000009",
            close: "0.00000000000091",
            volume_base: "1000",
            volume_quote: "0.00000000091",
            close_usd: "0",
            trade_count: 19,
            pf_trade_count: 19,
        }
    }

    /// A child whose `close` AND `close_usd` both sit under the precision floor —
    /// the row `PRICE_FLOOR_SQL`'s doc comment quotes from prod: five ticks over
    /// four. Its ratio (0.8 here) looks like an ordinary rate and is quantisation
    /// noise; it is "priced" only in the sense that enrichment wrote something.
    const fn noise_priced(ts: &'static str) -> Self {
        Self {
            ts,
            open: "0.00000000000005",
            high: "0.00000000000005",
            low: "0.00000000000005",
            close: "0.00000000000005",
            volume_base: "1000",
            volume_quote: "0.00000000005",
            close_usd: "0.00000000000004",
            trade_count: 19,
            pf_trade_count: 19,
        }
    }

    /// A DUST-ONLY minute: fills happened and count in volume, but not one of
    /// them formed a price, so the candle has none (ADR 0287 §3).
    const fn dust(ts: &'static str) -> Self {
        Self {
            ts,
            open: "0",
            high: "0",
            low: "0",
            close: "0",
            volume_base: "1",
            volume_quote: "0.05",
            close_usd: "0",
            trade_count: 3,
            pf_trade_count: 0,
        }
    }

    fn values(&self) -> String {
        let Self {
            ts,
            open,
            high,
            low,
            close,
            volume_base,
            volume_quote,
            close_usd,
            trade_count,
            pf_trade_count,
        } = self;
        // vwap = volume_quote / volume_base; pf_volume / pf_price_volume are the
        // price-forming shares, which for these fixtures are all-or-nothing.
        let (pf_volume, pf_price_volume) = if *pf_trade_count == 0 {
            ("0", "0")
        } else {
            (*volume_base, *volume_quote)
        };
        format!(
            "(toDateTime('{ts}'), 1, 2, 'sdex', {open}, {high}, {low}, {close}, \
             {volume_base}, {volume_quote}, 0, {close_usd}, 0, {trade_count}, 1, \
             {pf_trade_count}, {pf_volume}, {pf_price_volume})"
        )
    }
}

fn insert_minutes(db: &str, minutes: &[Minute]) -> String {
    let values: Vec<String> = minutes.iter().map(Minute::values).collect();
    format!(
        "INSERT INTO {db}.price_ohlcv_1m (timestamp, asset_id, quote_asset_id, source, \
         open, high, low, close, volume_base, volume_quote, volume_quote_usd, close_usd, \
         vwap, trade_count, version, pf_trade_count, pf_volume, pf_price_volume) VALUES {}",
        values.join(", ")
    )
}

/// A fresh scratch database holding the real init schema.
async fn setup(db: &str) -> Client {
    let admin = Client::default().with_url(ch_url());
    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop scratch");
    admin
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .expect("create scratch");
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("apply init schema");
    admin
}

async fn teardown(admin: &Client, db: &str) {
    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .expect("drop scratch");
}

async fn preroll(admin: &Client, db: &str) {
    prices_clickhouse::apply_sql(admin, &rewrite(prices_clickhouse::PREROLL_SQL, db))
        .await
        .expect("run the pre-roll chain");
}

async fn scalar(client: &Client, sql: &str) -> f64 {
    client
        .query(sql)
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("query failed: {sql}\n{e}"))
}

/// A coarse row's five price-ish columns, as f64.
async fn candle(client: &Client, db: &str, table: &str, where_clause: &str) -> [f64; 6] {
    let row: (f64, f64, f64, f64, f64, f64) = client
        .query(&format!(
            "SELECT toFloat64(open), toFloat64(high), toFloat64(low), toFloat64(close), \
             toFloat64(close_usd), toFloat64(vwap) \
             FROM {db}.{table} FINAL {where_clause}"
        ))
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("candle {table}: {e}"));
    [row.0, row.1, row.2, row.3, row.4, row.5]
}

fn approx(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() < 1e-6,
        "{what}: expected {want}, got {got}"
    );
}

/// The headline. A dust-only minute sits in a bucket with two priced ones; its
/// zero price must reach none of the parent's four price aggregates — most
/// visibly `low`, which the old `min(low)` took straight to 0 on every tier.
///
/// RED on the pre-0286 SQL: `min(low)` returns 0 and `argMax(close, …)` returns
/// the dust minute's 0, on all six tiers.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn a_dust_only_child_contributes_to_no_price_aggregate_of_its_parent() {
    let db = "it_rollup_pf_dust";
    let admin = setup(db).await;

    admin
        .query(&insert_minutes(
            db,
            &[
                Minute::priced("2026-04-02 00:00:00", "1.0", "1.5", "0.9", "1.1", "0"),
                Minute::dust("2026-04-02 00:01:00"),
                Minute::priced("2026-04-02 00:02:00", "1.1", "1.6", "1.0", "1.2", "0"),
            ],
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let [open, high, low, close, _, _] = candle(&admin, db, table, "").await;
        approx(open, 1.0, &format!("{table}: open"));
        approx(high, 1.6, &format!("{table}: high"));
        approx(low, 0.9, &format!("{table}: low"));
        approx(close, 1.2, &format!("{table}: close"));
        assert!(
            low > 0.0,
            "{table}: the dust minute's zero became the bucket's low"
        );

        // The dust fills still happened: they count in volume and trade_count,
        // and only in the pf_* columns are they absent (ADR 0287 §1).
        let trade_count = scalar(
            &admin,
            &format!("SELECT toFloat64(sum(trade_count)) FROM {db}.{table} FINAL"),
        )
        .await;
        approx(trade_count, 7.0, &format!("{table}: trade_count"));
        let pf = scalar(
            &admin,
            &format!("SELECT toFloat64(sum(pf_trade_count)) FROM {db}.{table} FINAL"),
        )
        .await;
        approx(pf, 4.0, &format!("{table}: pf_trade_count"));
        let vbase = scalar(
            &admin,
            &format!("SELECT toFloat64(sum(volume_base)) FROM {db}.{table} FINAL"),
        )
        .await;
        approx(vbase, 21.0, &format!("{table}: volume_base"));
    }

    teardown(&admin, db).await;
}

/// A bucket with NO price-forming child has no price — every conditional
/// aggregate matches nothing and returns the type default 0 (F6c), which is the
/// same "no price" encoding the 1m tier writes. The volume is still there.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn a_bucket_of_nothing_but_dust_has_no_price_and_keeps_its_volume() {
    let db = "it_rollup_pf_all_dust";
    let admin = setup(db).await;

    admin
        .query(&insert_minutes(
            db,
            &[
                Minute::dust("2026-04-02 00:00:00"),
                Minute::dust("2026-04-02 00:01:00"),
            ],
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let [open, high, low, close, close_usd, _] = candle(&admin, db, table, "").await;
        for (value, what) in [
            (open, "open"),
            (high, "high"),
            (low, "low"),
            (close, "close"),
            (close_usd, "close_usd"),
        ] {
            approx(value, 0.0, &format!("{table}: {what}"));
        }
        let (tc, pf, vbase): (f64, f64, f64) = admin
            .query(&format!(
                "SELECT toFloat64(sum(trade_count)), toFloat64(sum(pf_trade_count)), \
                 toFloat64(sum(volume_base)) FROM {db}.{table} FINAL"
            ))
            .fetch_one()
            .await
            .unwrap();
        approx(tc, 6.0, &format!("{table}: trade_count"));
        approx(pf, 0.0, &format!("{table}: pf_trade_count"));
        approx(vbase, 2.0, &format!("{table}: volume_base"));
    }

    teardown(&admin, db).await;
}

/// The close, and the USD close beside it (BRIEF §4.5).
///
/// The bucket's last child is dust, so the close is the last PRICE-FORMING
/// child's close. And `close_usd` is that close re-priced by the latest priced
/// child's RATE — not the latest priced child's `close_usd`, which belongs to a
/// different child's close.
///
/// The fixture separates the two deliberately: the last priced minute
/// (`close = 1.2`) is not yet enriched (`close_usd = 0`), so the latest rate
/// comes from the minute before it (`2.2 / 1.1 = 2.0`). The rate form gives
/// `1.2 × 2.0 = 2.4`; the carried-product form task 0145 shipped would give
/// `2.2`, a USD close belonging to a price the candle no longer reports.
///
/// RED on the pre-0286 SQL, twice over: `rollups.sql` closes at the dust
/// minute's 0, and `preroll.sql` carries 2.2.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn a_coarse_bucket_whose_last_child_is_dust_closes_at_the_last_priced_child() {
    let db = "it_rollup_pf_close";
    let admin = setup(db).await;

    admin
        .query(&insert_minutes(
            db,
            &[
                Minute::priced("2026-04-02 00:00:00", "1.0", "1.5", "0.9", "1.1", "2.2"),
                Minute::priced("2026-04-02 00:01:00", "1.1", "1.6", "1.0", "1.2", "0"),
                Minute::dust("2026-04-02 00:02:00"),
            ],
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let [_, _, _, close, close_usd, _] = candle(&admin, db, table, "").await;
        approx(close, 1.2, &format!("{table}: close"));
        assert!(
            (close_usd - 2.4).abs() < 1e-6,
            "{table}: close_usd must be this bucket's close re-priced by the \
             latest priced child's rate (1.2 x 2.0 = 2.4), got {close_usd}. \
             2.2 is the carried product — the other child's close_usd."
        );
    }

    teardown(&admin, db).await;
}

/// The RATE is held to the same precision floor as the prices (`1e-12`).
///
/// The latest "priced" child is the prod shape `close = 5e-14,
/// close_usd = 4e-14`. The price gate already refuses it, so the bucket closes
/// at 1.1 — but a rate gate of `close_usd > 0 AND close > 0` admits it, takes
/// 4/5 = 0.8 as the latest rate and re-prices that healthy close to 0.88,
/// where the only measured rate in the bucket (2.2 / 1.1 = 2.0) says 2.2.
/// RED on that gate, on every tier: the noise rate is carried up.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn a_rate_between_two_values_under_the_precision_floor_never_re_prices_the_bucket() {
    let db = "it_rollup_pf_rate_floor";
    let admin = setup(db).await;

    admin
        .query(&insert_minutes(
            db,
            &[
                Minute::priced("2026-04-02 00:00:00", "1.0", "1.5", "0.9", "1.1", "2.2"),
                Minute::noise_priced("2026-04-02 00:01:00"),
            ],
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let [_, _, _, close, close_usd, _] = candle(&admin, db, table, "").await;
        approx(close, 1.1, &format!("{table}: close"));
        assert!(
            (close_usd - 2.2).abs() < 1e-6,
            "{table}: close_usd must come from the last child whose rate is a \
             measurement (1.1 x 2.0 = 2.2), got {close_usd}. 0.88 is 1.1 x (4e-14 / \
             5e-14) — a ratio of two values under the precision floor."
        );
    }

    teardown(&admin, db).await;
}

/// …and the floor is on BOTH legs of the rate. Here the latest child's `close`
/// is a real price (1.2, so it closes the bucket) and only its `close_usd` is
/// four ticks — a USD value that cannot carry a rate. `/ohlcv` refuses to
/// convert such a row (`close_usd >= 1e-12`); the rollup must not build every
/// coarse `close_usd` above it from it. RED on a gate that floors `close` alone:
/// the bucket publishes `close_usd = 4e-14` beside a close of 1.2.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn a_close_usd_under_the_precision_floor_carries_no_rate() {
    let db = "it_rollup_pf_rate_floor_usd";
    let admin = setup(db).await;

    admin
        .query(&insert_minutes(
            db,
            &[
                Minute::priced("2026-04-02 00:00:00", "1.0", "1.5", "0.9", "1.1", "2.2"),
                Minute::priced(
                    "2026-04-02 00:01:00",
                    "1.1",
                    "1.6",
                    "1.0",
                    "1.2",
                    "0.00000000000004",
                ),
            ],
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let [_, _, _, close, close_usd, _] = candle(&admin, db, table, "").await;
        approx(close, 1.2, &format!("{table}: close"));
        assert!(
            (close_usd - 2.4).abs() < 1e-6,
            "{table}: close_usd must be 1.2 x the last MEASURED rate (2.0) = 2.4, \
             got {close_usd} — a four-tick close_usd is not a rate"
        );
    }

    teardown(&admin, db).await;
}

/// `low <= open, close <= high` on every tier, BY CONSTRUCTION — there is no
/// clamp anywhere in the rollup, so this holds only because the four price
/// aggregates all read the same gated subset of children.
///
/// The fixture's month is April 2026, whose 1st is a WEDNESDAY, and the extreme
/// price sits on 2026-04-02 — inside the week that starts 2026-03-30. A week is
/// attributed wholly to the month it STARTS in, so under the pre-0286 1w-fed
/// month that extreme landed in MARCH and April never saw it. RED there; the
/// month rolls from the day now.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn ohlc_ordering_holds_on_every_tier() {
    let db = "it_rollup_pf_ordering";
    let admin = setup(db).await;

    admin
        .query(&insert_minutes(
            db,
            &[
                Minute::priced("2026-03-25 12:00:00", "2.0", "2.1", "1.9", "2.0", "0"),
                Minute::priced("2026-03-31 12:00:00", "2.0", "2.2", "1.8", "2.1", "0"),
                // The extreme, in the straddling week but squarely in April.
                Minute::priced("2026-04-02 12:00:00", "2.1", "9.0", "0.5", "3.0", "0"),
                Minute::dust("2026-04-02 12:05:00"),
                Minute::priced("2026-04-08 12:00:00", "3.0", "3.2", "2.8", "3.1", "0"),
            ],
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let violations = scalar(
            &admin,
            &format!(
                "SELECT toFloat64(countIf(NOT (low <= open AND low <= close \
                 AND open <= high AND close <= high))) FROM {db}.{table} FINAL"
            ),
        )
        .await;
        approx(
            violations,
            0.0,
            &format!("{table}: OHLC ordering violations"),
        );
    }

    // The month boundary, exactly. April holds the extreme; March does not.
    let [_, april_high, april_low, _, _, _] = candle(
        &admin,
        db,
        "price_ohlcv_1M",
        "WHERE timestamp = toDateTime('2026-04-01 00:00:00')",
    )
    .await;
    approx(april_high, 9.0, "April 1M high");
    approx(april_low, 0.5, "April 1M low");

    let [_, march_high, _, _, _, _] = candle(
        &admin,
        db,
        "price_ohlcv_1M",
        "WHERE timestamp = toDateTime('2026-03-01 00:00:00')",
    )
    .await;
    approx(march_high, 2.2, "March 1M high");
    assert!(
        march_high < 9.0,
        "April's extreme reached March — the month is rolling from the week \
         again, and a week starting 2026-03-30 belongs wholly to March"
    );

    teardown(&admin, db).await;
}

/// BRIEF F11. Decimal division silently overflows past a ~1.7e10 dividend on
/// 26.3.10.60 — no exception, a wrong number — so both derived Decimals are
/// computed in Float64 and converted with `toDecimal128OrZero`. And a bucket
/// with no base volume divides by zero, which `divideDecimal` would throw on:
/// `vwap` must read an explicit 0, because `init.sql` declares the column NOT
/// Nullable.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn vwap_and_close_usd_survive_the_decimal_overflow_threshold() {
    let db = "it_rollup_pf_overflow";
    let admin = setup(db).await;

    // Two minutes of 2e10 base / 6e10 quote: the bucket's dividend is 1.2e11,
    // well past the threshold. vwap must still be exactly 3.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m (timestamp, asset_id, quote_asset_id, source, \
             open, high, low, close, volume_base, volume_quote, volume_quote_usd, close_usd, \
             vwap, trade_count, version, pf_trade_count, pf_volume, pf_price_volume) VALUES \
             (toDateTime('2026-04-02 00:00:00'), 1,2,'sdex', 3,3,3,3, \
              20000000000, 60000000000, 0, 30, 3, 2, 1, 2, 20000000000, 60000000000), \
             (toDateTime('2026-04-02 00:01:00'), 1,2,'sdex', 3,3,3,3, \
              20000000000, 60000000000, 0, 30, 3, 2, 1, 2, 20000000000, 60000000000), \
             (toDateTime('2026-04-02 00:00:00'), 3,2,'sdex', 5,5,5,5, \
              0, 0, 0, 0, 0, 1, 1, 1, 0, 0)"
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let [_, _, _, close, close_usd, vwap] =
            candle(&admin, db, table, "WHERE asset_id = 1").await;
        approx(close, 3.0, &format!("{table}: close"));
        approx(vwap, 3.0, &format!("{table}: vwap past 1.7e10"));
        approx(close_usd, 30.0, &format!("{table}: close_usd past 1.7e10"));

        let volume_quote = scalar(
            &admin,
            &format!(
                "SELECT toFloat64(sum(volume_quote)) FROM {db}.{table} FINAL WHERE asset_id = 1"
            ),
        )
        .await;
        approx(volume_quote, 1.2e11, &format!("{table}: volume_quote"));

        // The zero-volume series: no division to do, and an explicit 0 rather
        // than a NULL the non-Nullable column would have to rewrite.
        let [_, _, _, _, _, empty_vwap] = candle(&admin, db, table, "WHERE asset_id = 3").await;
        approx(
            empty_vwap,
            0.0,
            &format!("{table}: vwap with no base volume"),
        );
    }

    teardown(&admin, db).await;
}

/// Review WR-03: the coarse gate and the read path draw the SAME line, `1e-12`.
///
/// RED with the `t.close > 0` gate this replaces: the sub-floor child passes it,
/// `minIf` takes its `low = 9e-14` on every tier, and the coarse row publishes
/// that low beside a close of 1.1 — a price the read path would have refused had
/// it been asked about the child directly.
///
/// ⚠️ What this gate does NOT close, and cannot: a LEGACY child whose `close`
/// clears the floor while its `low` does not (21 such `1m` rows locally, feeding
/// 24 `1d` rows). The gate is asked about `close`, and that close is a real
/// price. Post-0286 the shape is unwritable — `low` is the minimum over
/// price-forming fills and every one of those now clears the floor
/// (`price::price_survives_column_scale`) — so it dies with the phase-3
/// re-ingest, not here.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn a_child_priced_under_the_precision_floor_reaches_no_price_aggregate() {
    let db = "it_rollup_pf_sub_floor";
    let admin = setup(db).await;

    admin
        .query(&insert_minutes(
            db,
            &[
                Minute::priced("2026-04-02 00:00:00", "1.0", "1.5", "0.9", "1.1", "0"),
                Minute::sub_floor("2026-04-02 00:01:00"),
            ],
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let [open, high, low, close, _, _] = candle(&admin, db, table, "").await;
        approx(open, 1.0, &format!("{table}: open"));
        approx(high, 1.5, &format!("{table}: high"));
        approx(
            low,
            0.9,
            &format!("{table}: low — 9e-14 is quantisation noise, not this bucket's low"),
        );
        approx(
            close,
            1.1,
            &format!(
                "{table}: close — the sub-floor row landed LAST and must not close the bucket"
            ),
        );

        // Its volume is real and still counted; only its "price" is refused.
        let trade_count = scalar(
            &admin,
            &format!("SELECT toFloat64(sum(trade_count)) FROM {db}.{table} FINAL"),
        )
        .await;
        approx(trade_count, 21.0, &format!("{table}: trade_count"));
    }

    teardown(&admin, db).await;
}

/// Review B WR-01 / WR-02 and C1, on the rollup side: a LEGACY row —
/// `pf_trade_count > 0` from the migration's DEFAULT, `close = 0` because its
/// price underflowed the column — must reach no price aggregate either.
///
/// RED before the `AND t.close > 0` term: `minIf(t.low, t.pf_trade_count > 0)`
/// takes the legacy zero on every tier, and because the row's `volume_base`
/// dwarfs the healthy minute's it is the loudest possible zero — a real bucket
/// with a real price publishing `low = 0`.
#[tokio::test]
#[ignore = "requires the local ClickHouse 26.3.10.60"]
async fn a_legacy_row_that_claims_a_price_it_cannot_print_reaches_no_price_aggregate() {
    let db = "it_rollup_pf_legacy_zero";
    let admin = setup(db).await;

    admin
        .query(&insert_minutes(
            db,
            &[
                Minute::priced("2026-04-02 00:00:00", "1.0", "1.5", "0.9", "1.1", "0"),
                Minute::legacy_unrepresentable("2026-04-02 00:01:00"),
            ],
        ))
        .execute()
        .await
        .expect("insert minutes");

    preroll(&admin, db).await;

    for table in COARSE {
        let [open, high, low, close, _, _] = candle(&admin, db, table, "").await;
        approx(open, 1.0, &format!("{table}: open"));
        approx(high, 1.5, &format!("{table}: high"));
        approx(low, 0.9, &format!("{table}: low"));
        approx(
            close,
            1.1,
            &format!("{table}: close — the legacy row landed LAST and must not close the bucket"),
        );

        // Its volume is real and still counted; only its "price" is refused.
        let vbase = scalar(
            &admin,
            &format!("SELECT toFloat64(sum(volume_base)) FROM {db}.{table} FINAL"),
        )
        .await;
        assert!(
            (vbase / 33_387_840_120.63 - 1.0).abs() < 1e-9,
            "{table}: volume_base — the legacy row's volume is real and stays: {vbase}"
        );
        let trade_count = scalar(
            &admin,
            &format!("SELECT toFloat64(sum(trade_count)) FROM {db}.{table} FINAL"),
        )
        .await;
        approx(trade_count, 3.0, &format!("{table}: trade_count"));
    }

    teardown(&admin, db).await;
}
