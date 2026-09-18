//! Pre-roll `close_usd` guard regression test (task 0145).
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-clickhouse --test preroll_close_usd_guard_it -- --ignored --test-threads=1
//!
//! `close_usd` is baked by a separate, LAGGING enrichment pass onto a
//! non-nullable `Decimal(38,14) DEFAULT 0` column, so "not yet enriched" and
//! "no USD price exists" are the same value: zero. An unguarded
//! `argMax(close_usd, t.timestamp)` therefore hands a coarse bucket that zero
//! whenever its NEWEST sub-bucket happens to be un-enriched — throwing away
//! every priced sub-bucket underneath it.
//!
//! BE's 0199 report found this in the six rollup MVs; task 0144's full-schema
//! audit found it in every pre-roll script too (121 further sites), which is the
//! dangerous instance: pre-rolls run over historical spans where enrichment is
//! incomplete *by definition*, at backfill scale, and the rows they zero then
//! age out of the MV re-aggregation windows where only the 0114 sweep can still
//! reach them.
//!
//! Task 0286 / ADR 0287 §5 SUPERSEDED the guard with something stronger: the
//! coarse `close_usd` is now this bucket's own `close` re-priced by the latest
//! priced child's RATE (`close_usd / close`). The un-enriched sentinel is
//! skipped exactly as before — that is what the rate predicate does (both legs at
//! or above the precision floor, `rollup_sql::RATE_BEARING_CHILD`) —
//! but `close` and `close_usd` are no longer allowed to come from different
//! sub-buckets, which is the consequence task 0145 had to accept. The
//! expectation below is therefore the rate-derived product, recomputed rather
//! than deleted.
//!
//! This test pins the fix end-to-end through the real shipped `preroll.sql`
//! chain, and — critically — first proves the fixture actually reproduces the
//! defect. A guard test that asserts `close_usd == 7.0` against input that would
//! have produced 7.0 anyway is worth nothing.
//!
//! Runs on the pinned prod engine (docker-compose: ClickHouse 26.3.10.60).

use clickhouse::Client;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Rewrite the `prices.*` schema onto an isolated scratch database (same trick
/// as `rollup_chain_it.rs` / `views_it.rs`) so the test never touches the real
/// `prices` tables and can be dropped wholesale at the end.
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// Fixed bucket boundary as a `toDateTime(…)` literal, computed once so every
/// row shares one `_15m`/`_1h`/…/`_1M` bucket regardless of wall-clock drift
/// during the test.
async fn bucket_anchor(client: &Client) -> String {
    let epoch: u64 = client
        .query("SELECT toUInt64(toUnixTimestamp(toStartOfInterval(now(), INTERVAL 15 MINUTE)))")
        .fetch_one()
        .await
        .expect("fetch bucket anchor");
    format!("toDateTime({epoch})")
}

/// Every coarse grain `preroll.sql` writes, in chain order.
const COARSE: &[&str] = &[
    "price_ohlcv_15m",
    "price_ohlcv_1h",
    "price_ohlcv_4h",
    "price_ohlcv_1d",
    "price_ohlcv_1w",
    "price_ohlcv_1M",
];

/// The enrichment sawtooth in miniature: three consecutive `_1m` candles in one
/// bucket, the two older ones priced, the NEWEST one still un-enriched
/// (`close_usd = 0`) — exactly the state a pre-roll finds when it runs over a
/// span the enrichment pass has not caught up with.
///
/// `close` rises monotonically to 1.30 so the un-enriched row is unambiguously
/// the argMax-by-time winner; `close_usd` peaks at 7.0 on the middle row, which
/// is therefore the latest *priced* close the guard must carry forward.
fn insert_partly_enriched_bucket(db: &str, anchor: &str) -> String {
    let b = anchor;
    format!(
        "INSERT INTO {db}.price_ohlcv_1m \
         (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
          volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
         ({b} - INTERVAL 14 MINUTE, 1,2,'sdex', 1.00,1.50,0.90,1.10, 10,50,50,5.0,1.00,1,1), \
         ({b} - INTERVAL 13 MINUTE, 1,2,'sdex', 1.10,1.60,1.00,1.20, 10,50,50,7.0,1.10,1,1), \
         ({b} - INTERVAL 12 MINUTE, 1,2,'sdex', 1.20,1.40,0.80,1.30, 10,50, 0,0.0,1.20,1,1)"
    )
}

async fn scalar(client: &Client, sql: &str) -> f64 {
    client
        .query(sql)
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("query failed: {sql}\n{e}"))
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn preroll_carries_the_latest_priced_close_usd_not_the_unenriched_zero() {
    let db = "it_preroll_close_usd_guard";
    let admin = Client::default().with_url(ch_url());

    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("apply init schema");

    let anchor = bucket_anchor(&admin).await;
    admin
        .query(&insert_partly_enriched_bucket(db, &anchor))
        .execute()
        .await
        .expect("insert partly-enriched _1m bucket");

    // ---------------------------------------------------------------------
    // 1. The fixture must actually reproduce the defect, or everything below
    //    is vacuous. Run BOTH expressions over the same `_1m` input.
    // ---------------------------------------------------------------------
    let unguarded = scalar(
        &admin,
        &format!(
            "SELECT toFloat64(argMax(close_usd, t.timestamp)) \
             FROM {db}.price_ohlcv_1m AS t FINAL GROUP BY asset_id"
        ),
    )
    .await;
    assert_eq!(
        unguarded, 0.0,
        "fixture does not reproduce the defect: the OLD unguarded expression \
         should return the un-enriched sentinel 0 for this input. If this ever \
         stops being 0, the fixture no longer models the enrichment sawtooth and \
         the assertions below prove nothing."
    );

    let guarded = scalar(
        &admin,
        &format!(
            "SELECT toFloat64(argMaxIf(close_usd, t.timestamp, close_usd > 0)) \
             FROM {db}.price_ohlcv_1m AS t FINAL GROUP BY asset_id"
        ),
    )
    .await;
    assert_eq!(
        guarded, 7.0,
        "the guarded expression must carry the latest PRICED close_usd (7.0 at \
         t-13), not the newest row's un-enriched 0"
    );

    // ---------------------------------------------------------------------
    // 2. End-to-end through the real shipped chain. `preroll.sql` is a plain
    //    front-to-back INSERT…SELECT chain (no refresh): one apply populates
    //    _15m … _1M from _1m FINAL.
    // ---------------------------------------------------------------------
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::PREROLL_SQL, db))
        .await
        .expect("run preroll chain");

    for target in COARSE {
        let rows = scalar(
            &admin,
            &format!("SELECT toFloat64(count()) FROM {db}.{target} FINAL"),
        )
        .await;
        assert_eq!(rows, 1.0, "{target}: expected exactly one rolled bucket");

        let close_usd = scalar(
            &admin,
            &format!("SELECT toFloat64(argMax(close_usd, timestamp)) FROM {db}.{target} FINAL"),
        )
        .await;
        // The latest PRICED child is t-13: close 1.20, close_usd 7.0, so a rate
        // of 7.0 / 1.20. This bucket closes at 1.30, so its USD close is
        // 1.30 x (7.0 / 1.20) = 7.5833…. A 0 here is the task 0145 defect (the
        // bucket inherited its newest sub-bucket's un-enriched sentinel and
        // discarded the priced rows underneath it); a flat 7.0 is the carried
        // product task 0286 replaced — a USD value for a price this candle no
        // longer reports.
        let want = 1.30 * (7.0 / 1.20);
        assert!(
            (close_usd - want).abs() < 1e-6,
            "{target}: pre-roll must re-price its own close at the latest \
             priced child's rate ({want}), got {close_usd}"
        );

        // The consequence task 0145 accepted — `close` and `close_usd` from
        // DIFFERENT sub-buckets — is what the rate form ENDS: the two are
        // same-bucket again by construction. `close` itself is untouched by
        // either: it is still the true last-by-time price-forming value.
        let close = scalar(
            &admin,
            &format!("SELECT toFloat64(argMax(close, timestamp)) FROM {db}.{target} FINAL"),
        )
        .await;
        assert_eq!(
            close, 1.30,
            "{target}: `close` must still be the true last-by-time value — the \
             close_usd rule must not perturb OHLC"
        );
    }

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The complement: when EVERY sub-bucket is un-enriched there is genuinely no
/// priced value to carry, `argMaxIf` matches no rows, and the Decimal default
/// puts a 0 back in the column. That is correct behaviour, not a regression —
/// but it means a 0 in these tables still cannot be read as "worth nothing",
/// which is the representational problem task 0151 owns. Pinned here so nobody
/// later reads the guard as a stronger promise than it makes.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn preroll_still_writes_zero_when_no_sub_bucket_was_ever_priced() {
    let db = "it_preroll_close_usd_all_unpriced";
    let admin = Client::default().with_url(ch_url());

    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("apply init schema");

    let anchor = bucket_anchor(&admin).await;
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({anchor} - INTERVAL 14 MINUTE, 1,2,'sdex', 1.00,1.50,0.90,1.10, 10,50,0,0.0,1.00,1,1), \
             ({anchor} - INTERVAL 12 MINUTE, 1,2,'sdex', 1.20,1.40,0.80,1.30, 10,50,0,0.0,1.20,1,1)"
        ))
        .execute()
        .await
        .expect("insert fully un-enriched _1m bucket");

    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::PREROLL_SQL, db))
        .await
        .expect("run preroll chain");

    for target in COARSE {
        let close_usd = scalar(
            &admin,
            &format!("SELECT toFloat64(argMax(close_usd, timestamp)) FROM {db}.{target} FINAL"),
        )
        .await;
        assert_eq!(
            close_usd, 0.0,
            "{target}: with nothing priced underneath it, the guard has nothing \
             to carry and the Decimal default 0 is the honest result"
        );
    }

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
