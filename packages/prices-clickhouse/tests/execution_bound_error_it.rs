//! A statement killed by `max_execution_time` must reach the caller carrying
//! ClickHouse's own error — the code, the elapsed time and the bound crossed
//! (task 0281).
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-clickhouse --test execution_bound_error_it -- --ignored
//!
//! WHY THIS EXISTS
//! ---------------
//! On 2026-09-11 the enrichment worker's bound was induced on production. The
//! bound fired exactly — ClickHouse killed the statement at 1002.3 ms against
//! a 1000 ms limit and recorded
//!
//!   Code: 159. DB::Exception: Timeout exceeded: elapsed 1002.319598 ms,
//!   maximum: 1000 ms. (TIMEOUT_EXCEEDED)
//!
//! while the worker logged `clickhouse: bad response: ` — an EMPTY string.
//! Every fact needed to diagnose the failure existed on the wire and was lost
//! one layer above it. That empty error is indistinguishable from a network
//! blip, which is exactly the signature that hid task 0215's outage for 26
//! days.
//!
//! ⚠️ The statement must be an `INSERT … SELECT`, not a `SELECT`. The two take
//! different paths, and the difference is what made this invisible: on
//! 2026-09-10 twenty-three `TIMEOUT_EXCEEDED` events were observed arriving
//! complete and well-formed — every one of them a read.

use clickhouse::Client;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// The URL of a **reverse proxy** in front of ClickHouse.
///
/// ⛔ This test is worthless without one, and would pass anyway — which is why
/// its absence fails rather than skips. The defect only appears through a
/// proxy: straight to ClickHouse the crate's LZ4 fallback fires and the error
/// survives even when the bug is present. Production always has Caddy in the
/// path, so the proxy IS the production shape.
///
///     scripts/ch-proxy-0281.sh up      # Caddy on :8124 -> ClickHouse :8123
fn proxy_url() -> String {
    std::env::var("CLICKHOUSE_PROXY_URL").expect(
        "CLICKHOUSE_PROXY_URL is unset — this test cannot detect the defect without a \
         reverse proxy in front of ClickHouse, and would pass vacuously. Run \
         `scripts/ch-proxy-0281.sh up` and re-run with \
         CLICKHOUSE_PROXY_URL=http://localhost:8124",
    )
}

/// An `INSERT … SELECT` guaranteed to outrun a one-second bound: it generates
/// far more rows than any machine can insert in that time, from `numbers` so
/// the cost is CPU rather than disk.
const SLOW_INSERT: &str = "INSERT INTO it_0281.sink SELECT number FROM numbers(5000000000)";

async fn setup() -> Client {
    let client = Client::default().with_url(ch_url());
    client
        .query("DROP DATABASE IF EXISTS it_0281")
        .execute()
        .await
        .unwrap();
    client
        .query("CREATE DATABASE it_0281")
        .execute()
        .await
        .unwrap();
    client
        .query("CREATE TABLE it_0281.sink (n UInt64) ENGINE = MergeTree ORDER BY n")
        .execute()
        .await
        .unwrap();
    client
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn an_exceeded_bound_reaches_the_caller_as_a_clickhouse_exception() {
    setup().await;

    // Built exactly as production builds it: through `with_readable_errors`,
    // the same function `mtls::client_with_mtls` uses. A change there fails
    // this test.
    let client = prices_clickhouse::with_readable_errors(Client::default().with_url(proxy_url()));
    let bounded = prices_clickhouse::with_execution_bound(client, 1);

    let err = bounded
        .query(SLOW_INSERT)
        .execute()
        .await
        .expect_err("a 5-billion-row INSERT cannot finish inside a 1s bound");

    let text = err.to_string();

    // The three facts that make a failure diagnosable after the fact. Asserted
    // separately so a regression says WHICH one was lost.
    assert!(
        !text
            .trim_end_matches(|c: char| c == ':' || c.is_whitespace())
            .ends_with("bad response"),
        "the error carries no message at all — this is the 0281 defect: {text:?}"
    );
    assert!(text.contains("159"), "the error code is missing: {text:?}");
    assert!(
        text.contains("TIMEOUT_EXCEEDED"),
        "the error name is missing: {text:?}"
    );
}
