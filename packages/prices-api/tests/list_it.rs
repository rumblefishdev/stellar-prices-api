//! Live-ClickHouse integration tests for `GET /v1/assets` (listing). Gated
//! `#[ignore]`:
//!
//!   tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!   cargo test -p prices-api --test list_it -- --ignored --test-threads=1

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clickhouse::Client;
use prices_api::{AppConfig, AppState, app};
use prices_clickhouse::asset_id::fixture::{self, AssetFixture};
use serde_json::Value;
use tower::ServiceExt;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

fn iss() -> &'static str {
    prices_clickhouse::USDC_ISSUER
}

/// `setup`'s assets: XLM, USDC, a Soroban token, FOO.
fn setup_assets() -> [AssetFixture<'static>; 4] {
    [
        AssetFixture::new("XLM", "native", "", ""),
        AssetFixture::new("USDC", "credit", iss(), ""),
        AssetFixture::new("", "contract", "", "CCONTRACTTOKEN"),
        AssetFixture::new("FOO", "credit", iss(), ""),
    ]
}

/// Seed 4 assets with distinct 24h volumes:
///   USDC=3000, token(soroban)=2000, XLM=1000, FOO=500.
async fn setup(db: &str) -> Client {
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
        .unwrap();
    admin
        .query(&fixture::assets_insert(db, &setup_assets()))
        .execute()
        .await
        .unwrap();
    let [xlm, usdc, token, foo_id] = setup_assets().map(|a| a.id());
    admin
        .query(&format!(
            // Task 0216: USDC carries a REAL as_of/price_status pair, dated
            // half an hour behind updated_at so a transposition of the two
            // DateTime columns cannot hide.
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at, as_of, price_status, \
              price_basis) \
             VALUES \
             ({usdc}, 1.0, 1.0, 3000, '2026-02-10 12:00:00', '2026-02-10 11:30:00', 'carried', \
              'offer_dust')"
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            // The other three rows name neither new column, so they really
            // take the table DEFAULTs (the epoch and ''), which is the shape
            // the wire must render as ""/"" — not a hand-written copy of it.
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at) \
             VALUES \
             ({xlm}, 0.5, 0.5, 1000, '2026-02-10 12:00:00'), \
             ({token}, 2.0, 2.0, 2000, '2026-02-10 12:00:00'), \
             ({foo_id}, 9.0, 9.0, 500,  '2026-02-10 12:00:00')"
        ))
        .execute()
        .await
        .unwrap();
    Client::default().with_url(ch_url()).with_database(db)
}

/// Seed `n` assets + `current_prices` rows for the pagination walk. Each asset
/// gets a unique `asset_code` (`A0001`…) — the response item exposes `asset_code`
/// (not the internal `asset_id`), so it is the identity we assert set-completeness
/// on. Volumes are bucketed into 13 tie-groups by row index (`(i % 13) * 100`):
/// the groups are large (~19 rows) and ids are hashes of the identity, so
/// equal-volume rows both *reorder* relative to id and *straddle* the 50-row page boundary —
/// which is exactly what exercises the cursor's `(sort_col, asset_id)` tie-break.
async fn setup_n(db: &str, n: u32) -> Client {
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
        .unwrap();

    let codes: Vec<String> = (1..=n).map(|i| format!("A{i:04}")).collect();
    let assets: Vec<AssetFixture<'_>> = codes
        .iter()
        .map(|code| AssetFixture::new(code, "credit", iss(), ""))
        .collect();
    let mut prices = Vec::with_capacity(n as usize);
    for (i, a) in (1..=n).zip(&assets) {
        let vol = (i % 13) * 100;
        prices.push(format!(
            "({}, 1.0, 1.0, {vol}, '2026-02-10 12:00:00')",
            a.id()
        ));
    }
    admin
        .query(&fixture::assets_insert(db, &assets))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at) VALUES {}",
            prices.join(", ")
        ))
        .execute()
        .await
        .unwrap();
    Client::default().with_url(ch_url()).with_database(db)
}

/// Percent-encode the reserved Base64 characters in an opaque cursor before it
/// goes into a query string. The cursor is STANDARD Base64 (`common/cursor.rs`),
/// whose alphabet includes `+ / =`; in an `application/x-www-form-urlencoded`
/// query, a raw `+` decodes to a space, corrupting the token → a 400. Encoding
/// these three makes the walk robust to whatever bytes the fixture produces.
fn enc_cursor(c: &str) -> String {
    c.replace('+', "%2B")
        .replace('/', "%2F")
        .replace('=', "%3D")
}

async fn teardown(db: &str) {
    let admin = Client::default().with_url(ch_url());
    let _ = admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await;
}

fn config() -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled: false,
        // Sign-in credentials are loaded asynchronously from Secrets Manager
        // (task 0186) and are never part of the environment; `None` is the shape
        // every non-portal test wants.
        portal_oauth: None,
        // Discord endpoints are part of the config now, not read from the
        // process environment per router — see `AppConfig::portal_endpoints`.
        portal_endpoints: Default::default(),
        // Task 0187: the control-plane client for self-service keys. `None`
        // is what every non-portal test wants — with no client in the
        // config there is no code path here that can reach API Gateway.
        portal_keys: None,
        portal_eligibility: None,
        portal_rate_limit: None,
        portal_web_origin: None,
    }
}

async fn get(client: Client, uri: &str) -> (StatusCode, Value) {
    let resp = app(&config(), AppState::new(client))
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn default_sort_volume_desc_paginates() {
    let db = "it_list_paginate_0040";
    let client = setup(db).await;

    // Page 1: top 2 by volume desc → USDC(3000), token(2000).
    let (status, page1) = get(client, "/v1/assets?limit=2").await;
    assert_eq!(status, StatusCode::OK, "body={page1}");
    let d1 = page1["data"].as_array().unwrap();
    assert_eq!(d1.len(), 2);
    assert_eq!(d1[0]["asset_code"], "USDC");
    assert_eq!(d1[1]["asset_type"], "soroban");
    assert_eq!(page1["has_more"], true);

    // Task 0216 — asserted BY VALUE on both arms. In this row `as_of`,
    // `price_status` and the cursor payload `sort_key` are three adjacent
    // Strings decoded positionally, so a reorder would publish the cursor as
    // the status and misparse silently. Checking the keys exist would not see
    // it; checking the values, and then walking the cursor below, does.
    assert_eq!(d1[0]["as_of"], "2026-02-10T11:30:00Z");
    assert_ne!(
        d1[0]["as_of"], d1[0]["updated_at"],
        "the listing must publish the price's own time, not the snapshot's"
    );
    assert_eq!(d1[0]["price_status"], "carried");
    // Task 0274 — price_basis now sits directly before sort_key; a distinct
    // word so a swap with either neighbour shows.
    assert_eq!(d1[0]["price_basis"], "offer_dust");

    let cursor = page1["cursor"].as_str().unwrap().to_string();

    // Page 2: XLM(1000), FOO(500); no more.
    let client = Client::default().with_url(ch_url()).with_database(db);
    let (status, page2) = get(client, &format!("/v1/assets?limit=2&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK, "body={page2}");
    let d2 = page2["data"].as_array().unwrap();
    assert_eq!(d2.len(), 2);
    assert_eq!(d2[0]["asset_code"], "XLM");
    assert_eq!(d2[1]["asset_code"], "FOO");
    assert_eq!(page2["has_more"], false);
    assert!(page2["cursor"].is_null());

    // The other arm: a row on the DEFAULT pair reaches the wire as two empty
    // strings, never as a formatted epoch. That the cursor delivered this page
    // at all is the second half of the positional proof above.
    assert_eq!(d2[0]["as_of"], "");
    assert_eq!(d2[0]["price_status"], "");
    assert_eq!(d2[0]["price_basis"], "");

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn filter_by_type() {
    let db = "it_list_filter_0040";
    let client = setup(db).await;

    let (_, soroban) = get(client, "/v1/assets?type=soroban").await;
    assert_eq!(soroban["data"].as_array().unwrap().len(), 1);
    assert_eq!(soroban["data"][0]["asset_type"], "soroban");

    let client = Client::default().with_url(ch_url()).with_database(db);
    let (_, classic) = get(client, "/v1/assets?type=classic").await;
    assert_eq!(classic["data"].as_array().unwrap().len(), 3);

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn search_prefix() {
    let db = "it_list_search_0040";
    let client = setup(db).await;
    let (_, json) = get(client, "/v1/assets?search=US").await;
    let d = json["data"].as_array().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0]["asset_code"], "USDC");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn invalid_sort_is_400() {
    let db = "it_list_badsort_0040";
    let client = setup(db).await;
    let (status, json) = get(client, "/v1/assets?sort=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    teardown(db).await;
}

/// Task 0074 — walk the FULL default (`volume_24h DESC`) keyset result set over a
/// 250-row fixture at `limit=50`, following `?cursor` until `has_more == false`,
/// and assert every asset appears **exactly once**: no duplicates, no skips. The
/// fixture packs equal-volume rows into large tie-groups that straddle the page
/// boundary (see [`setup_n`]), so a broken `(sort_col, asset_id)` tie-break would
/// surface here as a dropped or repeated row.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn keyset_pagination_250_rows_no_dup_no_skip() {
    let db = "it_list_paginate_250_0074";
    let _ = setup_n(db, 250).await;

    let mut seen: Vec<String> = Vec::new();
    let mut pages = 0;
    let mut uri = "/v1/assets?limit=50".to_string();
    loop {
        // `get` consumes the client, so build a fresh one per request.
        let client = Client::default().with_url(ch_url()).with_database(db);
        let (status, page) = get(client, &uri).await;
        assert_eq!(status, StatusCode::OK, "page {} body={page}", pages + 1);
        pages += 1;

        let data = page["data"].as_array().unwrap();
        // 250 / 50 = 5 exact pages, so every page (including the last) is full.
        assert_eq!(data.len(), 50, "page {pages} must be a full 50-row page");
        for item in data {
            seen.push(item["asset_code"].as_str().unwrap().to_string());
        }

        if page["has_more"].as_bool().unwrap() {
            let cursor = page["cursor"]
                .as_str()
                .expect("cursor present when has_more");
            uri = format!("/v1/assets?limit=50&cursor={}", enc_cursor(cursor));
        } else {
            assert!(page["cursor"].is_null(), "last page cursor must be null");
            break;
        }
    }

    // Page count: 250 rows at 50/page.
    assert_eq!(pages, 5, "expected exactly 5 pages");
    // No skips (count): every seeded row was returned.
    assert_eq!(seen.len(), 250, "collected 250 rows across the walk");
    // No duplicates: the multiset collapses to 250 distinct codes.
    let got: std::collections::BTreeSet<String> = seen.iter().cloned().collect();
    assert_eq!(
        got.len(),
        250,
        "no asset appeared twice across the cursor walk"
    );
    // No skips (set): the collected set equals the seeded set exactly.
    let expected: std::collections::BTreeSet<String> =
        (1..=250u32).map(|i| format!("A{i:04}")).collect();
    assert_eq!(got, expected, "collected set equals the seeded set");

    teardown(db).await;
}

// ---------------------------------------------------------------------------
// Task 0139 — full cursor walk over identity-derived ids.
// ---------------------------------------------------------------------------

/// One seeded identity: `(asset_code, issuer_address, contract_address)`, the
/// triple the listing publishes and the id is derived from.
type Identity = (String, String, String);

/// Seed `classic` code pairs that differ only by case under one issuer, the
/// first `shared` codes again under a second issuer, `contracts` Soroban tokens
/// with an empty code, and native XLM. Every `asset_id` is derived from the
/// identity by the database (`asset_id::fixture`), so ids are sparse hashes
/// with no relation to insertion order, as in prod after task 0139.
///
/// Ties everywhere: volumes take 7 values, prices 5, the second issuer repeats
/// codes, and every Soroban token sorts on the same empty code. Each tie group
/// is far larger than one page, so every walk crosses page boundaries inside a
/// tie and only the `(sort, asset_id)` tiebreak keeps it exact.
async fn setup_identities(
    db: &str,
    classic: usize,
    shared: usize,
    contracts: usize,
) -> Vec<Identity> {
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
        .unwrap();

    let usdt = prices_clickhouse::USDT_ISSUER;
    let mut seeded: Vec<(Identity, &str)> =
        vec![(("XLM".to_string(), String::new(), String::new()), "native")];
    for i in 0..classic {
        for code in [format!("T{i:03}"), format!("t{i:03}")] {
            seeded.push(((code, iss().to_string(), String::new()), "credit"));
        }
    }
    for i in 0..shared {
        seeded.push((
            (format!("T{i:03}"), usdt.to_string(), String::new()),
            "credit",
        ));
    }
    for i in 0..contracts {
        seeded.push((
            (String::new(), String::new(), format!("CWALK{i:03}")),
            "contract",
        ));
    }

    let rows: Vec<fixture::AssetFixture<'_>> = seeded
        .iter()
        .map(
            |((code, issuer, contract), asset_type)| fixture::AssetFixture {
                code,
                asset_type,
                issuer,
                contract,
                sac: "",
            },
        )
        .collect();
    admin
        .query(&fixture::assets_insert(db, &rows))
        .execute()
        .await
        .unwrap();

    let prices: Vec<String> = seeded
        .iter()
        .enumerate()
        .map(|(n, ((code, issuer, contract), _))| {
            format!(
                "({id}, {price}, 1.0, {vol}, '2026-02-10 12:00:00')",
                id = fixture::id(code, issuer, contract),
                price = n % 5 + 1,
                vol = (n % 7) * 100,
            )
        })
        .collect();
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at) VALUES {}",
            prices.join(", ")
        ))
        .execute()
        .await
        .unwrap();

    // Precondition: the fixture really gave every identity its own id. A
    // collision here would make the walk below fail for the wrong reason.
    let (rows_n, ids_n) = admin
        .query(&format!(
            "SELECT count(), uniqExact(asset_id) FROM {db}.assets FINAL"
        ))
        .fetch_one::<(u64, u64)>()
        .await
        .unwrap();
    assert_eq!(
        rows_n,
        seeded.len() as u64,
        "every identity is one assets row"
    );
    assert_eq!(ids_n, rows_n, "every identity has its own asset_id");

    seeded.into_iter().map(|(identity, _)| identity).collect()
}

/// Follow `next_cursor` from the first page to the last for one sort, and
/// return every published identity in walk order. Panics if the walk does not
/// end within `max_pages`.
async fn walk(db: &str, query: &str, limit: usize, max_pages: usize) -> Vec<Identity> {
    let mut seen = Vec::new();
    let mut uri = format!("/v1/assets?{query}&limit={limit}");
    for page_no in 1..=max_pages {
        let client = Client::default().with_url(ch_url()).with_database(db);
        let (status, page) = get(client, &uri).await;
        assert_eq!(status, StatusCode::OK, "{query} page {page_no} body={page}");
        let data = page["data"].as_array().unwrap();
        assert!(data.len() <= limit, "{query} page {page_no} over the limit");
        for item in data {
            let field = |k: &str| item[k].as_str().unwrap().to_string();
            seen.push((
                field("asset_code"),
                field("issuer_address"),
                field("contract_address"),
            ));
        }
        if !page["has_more"].as_bool().unwrap() {
            assert!(
                page["cursor"].is_null(),
                "{query}: last page cursor must be null"
            );
            return seen;
        }
        assert_eq!(data.len(), limit, "{query}: a page before the last is full");
        let cursor = page["cursor"]
            .as_str()
            .expect("cursor present when has_more");
        uri = format!(
            "/v1/assets?{query}&limit={limit}&cursor={}",
            enc_cursor(cursor)
        );
    }
    panic!("{query}: the walk did not end within {max_pages} pages");
}

/// Lore 0139 acceptance criterion "full cursor walk": every page of `GET
/// /assets`, followed by its cursor to the end, returns each seeded identity
/// exactly once, under every sort the cursor carries. Ids come from the
/// identity (sparse hashes, unrelated to insertion order), and the cursor
/// carries them as u64, so this runs unchanged on the UInt32 schema and on the
/// UInt64 one once the fixture switches width.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn full_cursor_walk_returns_every_identity_exactly_once() {
    let db = "it_list_full_walk_0139";
    // 1 native + 2×130 case pairs + 30 repeated under a second issuer + 25
    // Soroban tokens = 316 identities: not a multiple of the page size.
    let seeded = setup_identities(db, 130, 30, 25).await;
    assert_eq!(seeded.len(), 316);
    let expected: std::collections::BTreeSet<Identity> = seeded.iter().cloned().collect();
    assert_eq!(
        expected.len(),
        seeded.len(),
        "seeded identities are distinct"
    );

    let limit = 50;
    let max_pages = seeded.len() / limit + 2;
    for query in [
        "sort=volume_24h&order=desc",
        "sort=volume_24h&order=asc",
        "sort=price&order=asc",
        "sort=code&order=asc",
        "sort=code&order=desc",
    ] {
        let seen = walk(db, query, limit, max_pages).await;
        let got: std::collections::BTreeSet<Identity> = seen.iter().cloned().collect();
        assert_eq!(
            got.len(),
            seen.len(),
            "{query}: an identity was returned twice"
        );
        assert_eq!(
            got, expected,
            "{query}: the walk must return the seeded set"
        );
    }

    teardown(db).await;
}

// ---------------------------------------------------------------------------
// Task 0210 — Soroban token symbols composed into `asset_code` at read time.
// ---------------------------------------------------------------------------

/// Insert a `prices.asset_symbol` row for the fixture's Soroban asset
/// (`CCONTRACTTOKEN`), which `setup` seeds with an empty `asset_code`.
async fn seed_symbol(db: &str, contract: &str, symbol: &str) {
    Client::default()
        .with_url(ch_url())
        .query(&format!(
            "INSERT INTO {db}.asset_symbol (contract_address, symbol) VALUES ('{contract}', '{symbol}')"
        ))
        .execute()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn listing_composes_soroban_symbol_into_asset_code() {
    // The stored `assets` row keeps `asset_code = ''` — writing the symbol there
    // would create a SECOND row, because that column is part of the table's sort
    // key (task 0139's fan-out). The symbol lives in the single-writer
    // `asset_symbol` table and the listing composes it in.
    let db = "it_list_symbol_0210";
    let client = setup(db).await;
    seed_symbol(db, "CCONTRACTTOKEN", "SolvBTC").await;

    let (status, json) = get(client, "/v1/assets?type=soroban").await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let rows = json["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "still exactly one soroban asset, not two");
    assert_eq!(rows[0]["asset_code"], "SolvBTC");
    assert_eq!(rows[0]["contract_address"], "CCONTRACTTOKEN");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn listing_leaves_unresolved_soroban_code_empty() {
    // No `asset_symbol` row: the LEFT JOIN misses and the field stays `""`,
    // which is the pre-0210 behaviour every existing consumer sees.
    let db = "it_list_symbol_miss_0210";
    let client = setup(db).await;

    let (status, json) = get(client, "/v1/assets?type=soroban").await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["data"][0]["asset_code"], "");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn listing_reads_the_sentinel_row_as_an_empty_code() {
    // An empty `symbol` is the sentinel the resolver writes for a contract that
    // exposes no usable `symbol()`. It must read back as `""` — indistinguishable
    // from "not resolved yet" to a consumer, and never leaked as a marker.
    let db = "it_list_symbol_sentinel_0210";
    let client = setup(db).await;
    seed_symbol(db, "CCONTRACTTOKEN", "").await;

    let (status, json) = get(client, "/v1/assets?type=soroban").await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["data"][0]["asset_code"], "");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn classic_codes_are_untouched_by_the_symbol_join() {
    // Classic and native rows have `contract_address = ''` and miss the join
    // entirely; the `if(a.asset_code != '', …)` branch short-circuits for them
    // regardless. Pinned because the join sits on the listing's hot path.
    let db = "it_list_symbol_classic_0210";
    let client = setup(db).await;
    seed_symbol(db, "CCONTRACTTOKEN", "SolvBTC").await;

    let (status, json) = get(client, "/v1/assets?sort=volume_24h&order=desc").await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let codes: Vec<&str> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["asset_code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, vec!["USDC", "SolvBTC", "XLM", "FOO"]);
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn search_and_sort_still_read_the_raw_column() {
    // Deliberate scope boundary, not an oversight: `?search=` is
    // `startsWith(a.asset_code, ?)` and `sort=code` orders on `a.asset_code`,
    // both on the STORED column. A Soroban token is therefore displayed by its
    // symbol but not matched or ordered by it.
    //
    // `symbol()` is a string the contract itself controls, so making it
    // searchable is what would let a hostile token surface under a well-known
    // code. That belongs with the verification signal that makes it safe (task
    // 0252), not here. This test pins the boundary so a later change to it is a
    // decision rather than an accident.
    let db = "it_list_symbol_search_0210";
    let client = setup(db).await;
    seed_symbol(db, "CCONTRACTTOKEN", "SolvBTC").await;

    let (status, json) = get(client.clone(), "/v1/assets?search=Solv").await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert!(
        json["data"].as_array().unwrap().is_empty(),
        "search matches the stored asset_code only, so the symbol is not findable"
    );

    // sort=code orders on the stored `''`, which sorts first ascending.
    let (status, json) = get(client, "/v1/assets?sort=code&order=asc").await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(
        json["data"][0]["asset_code"], "SolvBTC",
        "the row displaying SolvBTC is ordered by its stored empty code"
    );
    teardown(db).await;
}
