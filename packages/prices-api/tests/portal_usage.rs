//! Usage against quota, over HTTP, against a mock control plane (task 0188).
//!
//! The unit tests next to `portal/usage/mod.rs` cover the period arithmetic —
//! the calendar month, the December rollover, the leap year — on dates typed by
//! hand. This file covers what they cannot: the route wired into the real
//! [`prices_api::app`] router, driving the **actual AWS SDK** against the mock
//! API Gateway control plane in `portal_keys/harness.rs`, `GetUsage` included.
//!
//! What this slice promises, and what is pinned here:
//!
//! - it ships **closed** (task 0183's gate, an empty `404`);
//! - it is **read-only** — no create, no attach, no delete, on any path,
//!   because a dashboard load must never mint or reconcile a production key;
//! - repeated loads are served from the in-process cache rather than each
//!   costing a `GetUsage`;
//! - a throttled control plane gets the last good answer, not an error page;
//! - and every response is `no-store`.

#[path = "portal_keys/harness.rs"]
mod harness;

use std::time::Duration;

use axum::http::StatusCode;
use chrono::{Datelike, Utc};
use prices_api::portal::usage::USAGE_PATH;
use serde_json::Value;

use harness::*;

/// One usage request through the full router (gate included), as the page
/// makes it.
async fn usage(mock: &MockGateway, sub: &str) -> Reply {
    call_path(
        app_against(mock),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(sub)),
    )
    .await
}

/// Seed a key and a month of usage for it: `[121, 99879]` then `[0, 99879]` —
/// the exact pair of days task 0157's close read off the live plan.
fn seed_key_with_usage(mock: &MockGateway) -> String {
    mock.with(|s| {
        let id = s.seed(&format!("discord-{USER_ID}-key"), 100);
        s.usage.insert(id.clone(), vec![(121, 99_879), (0, 99_879)]);
        id
    })
}

// ---------------------------------------------------------------------------
// The gate (AC 1)
// ---------------------------------------------------------------------------

/// **Ships closed.** With `PORTAL_ENABLED=false` the route is an empty `404`,
/// byte-identical to a path that was never deployed — asserted with a valid
/// session presented, because "signed in" must buy nothing while the portal is
/// closed.
#[tokio::test]
async fn a_closed_portal_answers_usage_with_an_empty_404() {
    let mock = MockGateway::start().await;
    let closed = build_app(false, None);

    let gated = call_path(
        closed.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    let unrouted = call_path(
        closed,
        "GET",
        "/api-tokens/api/no-such-route",
        Some(&session_cookie(USER_ID)),
    )
    .await;

    assert_eq!(gated.status, StatusCode::NOT_FOUND);
    assert!(gated.body.is_empty(), "the closed 404 must carry no body");
    assert_eq!(gated.status, unrouted.status);
    assert_eq!(gated.body, unrouted.body);

    // And nothing reached the control plane on the way.
    mock.with(|s| {
        assert_eq!(s.list_calls, 0);
        assert_eq!(s.usage_calls, 0);
    });
}

// ---------------------------------------------------------------------------
// The numbers (AC 2)
// ---------------------------------------------------------------------------

/// The happy path: used, remaining and the reconstructed limit, plus the
/// period boundaries under our calendar-month rule, `no-store`, and an `as_of`
/// for the page's "last updated" line.
#[tokio::test]
async fn usage_answers_the_numbers_and_the_period() {
    let mock = MockGateway::start().await;
    let key_id = seed_key_with_usage(&mock);

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.cache_control(), "no-store");

    let body = reply.json();
    assert_eq!(body["used"], 121, "{body}");
    assert_eq!(body["remaining"], 99_879, "{body}");
    assert_eq!(body["limit"], 100_000, "{body}");

    // The period is OUR rule — the calendar month, UTC — so the expectation is
    // computed the same way rather than copied from the handler's output.
    let today = Utc::now().date_naive();
    let start = format!("{:04}-{:02}-01", today.year(), today.month());
    assert_eq!(body["period_start"], Value::String(start));
    let (reset_year, reset_month) = if today.month() == 12 {
        (today.year() + 1, 1)
    } else {
        (today.year(), today.month() + 1)
    };
    assert_eq!(
        body["resets_at"],
        Value::String(format!("{reset_year:04}-{reset_month:02}-01T00:00:00Z"))
    );
    assert!(
        body["period_end"].as_str().unwrap().starts_with(&format!(
            "{:04}-{:02}-",
            today.year(),
            today.month()
        )),
        "{body}"
    );
    assert!(body["as_of"].as_str().unwrap().ends_with('Z'), "{body}");

    // The GetUsage was for the caller's key, from the period start — but only
    // UP TO TODAY: whether the live control plane accepts a future `endDate`
    // has never been verified, and days after today carry no data anyway. The
    // rendered period_end stays the month boundary; the query does not.
    mock.with(|s| {
        assert_eq!(s.usage_queries.len(), 1);
        let (asked_key, asked_start, asked_end) = s.usage_queries[0].clone();
        assert_eq!(asked_key, key_id);
        assert_eq!(asked_start, body["period_start"].as_str().unwrap());
        assert_eq!(asked_end, today.format("%Y-%m-%d").to_string());
    });
}

/// **Read-only, on the happy path.** The lookup adopts the reveal's flow but
/// must never take its write half: a dashboard load that created, attached or
/// deleted anything would be issuing production keys to anyone who opened the
/// page (task 0187, decision 14).
#[tokio::test]
async fn a_usage_read_never_touches_the_control_plane_write_paths() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);

    mock.with(|s| {
        assert_eq!(s.create_calls, 0, "a usage read must never create a key");
        assert_eq!(s.attach_calls, 0, "a usage read must never attach a key");
        assert!(s.deleted.is_empty(), "a usage read must never delete a key");
    });
}

/// A signed-in caller with no key is told so — a real portal `404` with the
/// JSON envelope, distinguishable from the gate's empty one on purpose — and
/// **still nothing is created**: "never issued" must not become "issued by
/// looking at the dashboard".
#[tokio::test]
async fn a_caller_with_no_key_gets_no_key_and_nothing_is_created() {
    let mock = MockGateway::start().await;

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(reply.json()["code"], "no_key");

    mock.with(|s| {
        assert_eq!(s.create_calls, 0);
        assert_eq!(s.attach_calls, 0);
        assert!(s.deleted.is_empty());
        assert_eq!(s.usage_calls, 0, "there is no key to ask usage for");
    });
}

/// No session → `401 not_signed_in`, same code and audience as the key routes.
#[tokio::test]
async fn no_session_is_401_not_signed_in() {
    let mock = MockGateway::start().await;

    let reply = call_path(app_against(&mock), "GET", USAGE_PATH, None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(reply.json()["code"], "not_signed_in");
    mock.with(|s| assert_eq!(s.list_calls, 0));
}

/// A key AWS has no rows for yet — the ordinary state minutes after issuance,
/// because `GetUsage` is not a read-after-write surface. The three counters go
/// absent **together**; the period and `as_of` are ours and stay.
#[tokio::test]
async fn a_key_with_no_rows_reports_nothing_recorded_rather_than_zeros() {
    let mock = MockGateway::start().await;
    mock.with(|s| {
        s.seed(&format!("discord-{USER_ID}-key"), 100);
    });

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["used"], Value::Null, "{body}");
    assert_eq!(body["remaining"], Value::Null, "{body}");
    assert_eq!(body["limit"], Value::Null, "{body}");
    assert!(body["period_start"].is_string());
    assert!(body["as_of"].is_string());
}

/// `GetUsage` paginates like every other listing; the summing walks every page
/// before it reports. Five days at two per page — stopping at page one would
/// report `used = 3` and the wrong `remaining`.
#[tokio::test]
async fn usage_pages_are_summed_to_exhaustion() {
    let mock = MockGateway::start().await;
    mock.with(|s| {
        let id = s.seed(&format!("discord-{USER_ID}-key"), 100);
        s.usage.insert(
            id,
            vec![
                (1, 99_999),
                (2, 99_997),
                (3, 99_994),
                (4, 99_990),
                (5, 99_985),
            ],
        );
        s.usage_page_size = 2;
    });

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["used"], 15, "{body}");
    assert_eq!(body["remaining"], 99_985, "{body}");
    assert_eq!(body["limit"], 100_000, "{body}");
    mock.with(|s| {
        assert!(
            s.usage_calls >= 3,
            "five days at two per page is three pages"
        )
    });
}

// ---------------------------------------------------------------------------
// The lookup shares the reveal's discipline
// ---------------------------------------------------------------------------

/// The mock matches `nameQuery` by **prefix**, like the service — so a name
/// that extends the caller's exact name comes back from the listing, and only
/// the client-side exact filter keeps its usage out of the answer.
#[tokio::test]
async fn a_prefix_neighbour_is_not_the_callers_key() {
    let mock = MockGateway::start().await;
    let (own, _imposter) = mock.with(|s| {
        let own = s.seed(&format!("discord-{USER_ID}-key"), 200);
        // An exact-name EXTENSION — what a console-created copy looks like.
        let imposter = s.seed(&format!("discord-{USER_ID}-key-old"), 100);
        s.usage.insert(own.clone(), vec![(7, 99_993)]);
        s.usage.insert(imposter.clone(), vec![(555, 0)]);
        (own, imposter)
    });

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["used"], 7);
    mock.with(|s| {
        assert_eq!(s.usage_queries.len(), 1);
        assert_eq!(
            s.usage_queries[0].0, own,
            "usage must be read for the exact-name key only"
        );
    });
}

/// Duplicates rank by the same deterministic rule the reveal uses — earliest
/// `createdDate` — so the usage shown belongs to the key the reveal hands out.
/// And the loser is **left alone**: sweeping duplicates is the issue flow's
/// job, not a read's.
#[tokio::test]
async fn duplicates_resolve_to_the_reveals_winner_without_deleting_the_loser() {
    let mock = MockGateway::start().await;
    let (earliest, later) = mock.with(|s| {
        let later = s.seed(&format!("discord-{USER_ID}-key"), 500);
        let earliest = s.seed(&format!("discord-{USER_ID}-key"), 100);
        s.usage.insert(earliest.clone(), vec![(11, 99_989)]);
        s.usage.insert(later.clone(), vec![(999, 0)]);
        (earliest, later)
    });

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["used"], 11);
    mock.with(|s| {
        assert_eq!(s.usage_queries[0].0, earliest);
        assert!(s.deleted.is_empty(), "a read must not sweep duplicates");
        assert!(
            s.keys.iter().any(|k| k.id == later),
            "the loser survives a read"
        );
    });
}

// ---------------------------------------------------------------------------
// The cache (AC 5)
// ---------------------------------------------------------------------------

/// **Repeated dashboard loads do not produce one `GetUsage` call each.** Two
/// loads through one router (one warm process): the second is answered from
/// the cache — no `GetApiKeys` either — and carries the same `as_of`, which is
/// what the "last updated" line is for.
#[tokio::test]
async fn a_second_load_is_served_from_the_cache() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    let router = app_against(&mock);

    let first = call_path(
        router.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    let second = call_path(router, "GET", USAGE_PATH, Some(&session_cookie(USER_ID))).await;

    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(first.json(), second.json(), "same answer, same as_of");
    mock.with(|s| {
        assert_eq!(s.usage_calls, 1, "the second load must not reach GetUsage");
        assert_eq!(s.list_calls, 1, "or GetApiKeys");
    });
}

/// The cache answers per **caller**: a second user's load is not served the
/// first user's numbers.
#[tokio::test]
async fn the_cache_is_per_caller() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    let other = "999999999999999999";
    mock.with(|s| {
        let id = s.seed(&format!("discord-{other}-key"), 100);
        s.usage.insert(id, vec![(3, 99_997)]);
    });
    let router = app_against(&mock);

    let first = call_path(
        router.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    let second = call_path(router, "GET", USAGE_PATH, Some(&session_cookie(other))).await;

    assert_eq!(first.json()["used"], 121);
    assert_eq!(second.json()["used"], 3);
    mock.with(|s| assert_eq!(s.usage_calls, 2));
}

/// An expired entry is refreshed rather than served: TTL zero, so the second
/// load asks AWS again.
#[tokio::test]
async fn an_expired_entry_is_refreshed() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    let router = usage_router_with(&mock, Duration::ZERO, Duration::from_secs(10));

    let first = call_path(
        router.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    let second = call_path(router, "GET", USAGE_PATH, Some(&session_cookie(USER_ID))).await;

    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(second.status, StatusCode::OK);
    mock.with(|s| assert_eq!(s.usage_calls, 2, "TTL zero means every load refetches"));
}

// ---------------------------------------------------------------------------
// Throttling (AC 6)
// ---------------------------------------------------------------------------

/// **Throttling backs off rather than erroring the page.** The SDK's own
/// retries come first; when the throttle outlasts them, the last good answer
/// is served — with its ORIGINAL `as_of`, so the page's "last updated" line
/// says exactly how stale it is.
#[tokio::test]
async fn a_throttled_control_plane_gets_the_last_good_answer() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    // TTL zero: the cached entry is expired immediately, so serving it is the
    // throttle fallback and nothing else.
    let router = usage_router_with(&mock, Duration::ZERO, Duration::from_secs(10));

    let first = call_path(
        router.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);

    mock.with(|s| s.throttle_usage = true);
    let second = call_path(router, "GET", USAGE_PATH, Some(&session_cookie(USER_ID))).await;

    assert_eq!(second.status, StatusCode::OK, "stale beats an error page");
    assert_eq!(first.json(), second.json(), "same answer, same as_of");
}

/// The throttle can land on the key LOOKUP just as well as on the usage read —
/// the account-wide budget does not care which operation is asked — and both
/// must reach the same stale-serve branch. This is the assertion behind
/// `list_named` raising `GatewayError::Throttled`: revert that arm to a plain
/// error mapping and this test answers `502` instead of the stale `200`.
#[tokio::test]
async fn a_throttled_key_lookup_also_gets_the_last_good_answer() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    let router = usage_router_with(&mock, Duration::ZERO, Duration::from_secs(10));

    let first = call_path(
        router.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);

    mock.with(|s| s.throttle_list = true);
    let second = call_path(router, "GET", USAGE_PATH, Some(&session_cookie(USER_ID))).await;

    assert_eq!(second.status, StatusCode::OK, "stale beats an error page");
    assert_eq!(first.json(), second.json());
}

/// Serving a stale answer during a throttle **re-stamps** it, so the next
/// load inside the TTL is a cache hit that never reaches AWS. Without the
/// re-stamp, every viewer refresh during a throttle event fired both
/// control-plane calls just to be told to slow down again — the opposite of
/// backing off.
#[tokio::test]
async fn a_served_stale_answer_backs_off_the_next_load() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    let router = usage_router_with(&mock, Duration::from_millis(50), Duration::from_secs(10));

    let first = call_path(
        router.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);

    // Age the entry past the TTL, then throttle: the next load attempts AWS,
    // gets refused, and serves (and re-stamps) the stale answer.
    tokio::time::sleep(Duration::from_millis(60)).await;
    mock.with(|s| s.throttle_usage = true);
    let calls_before_stale = mock.with(|s| s.usage_calls);
    let stale = call_path(
        router.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(stale.status, StatusCode::OK);
    let calls_after_stale = mock.with(|s| s.usage_calls);
    assert!(
        calls_after_stale > calls_before_stale,
        "the stale serve tried AWS first"
    );

    // Immediately again: the re-stamped entry is fresh, so nothing reaches
    // the (still throttling) control plane at all.
    let backed_off = call_path(router, "GET", USAGE_PATH, Some(&session_cookie(USER_ID))).await;
    assert_eq!(backed_off.status, StatusCode::OK);
    assert_eq!(backed_off.json(), first.json(), "same answer, same as_of");
    assert_eq!(
        mock.with(|s| s.usage_calls),
        calls_after_stale,
        "a re-stamped stale answer is a cache hit; AWS is left alone"
    );
}

/// Issuing a key evicts a cached "no key": without this, the page's own
/// refetch after the press — and any reload for the next minute — would be
/// served the stale `NoKey` and tell a key-holder they have no key.
#[tokio::test]
async fn issuing_a_key_evicts_a_cached_no_key() {
    let mock = MockGateway::start().await;
    let router = app_against(&mock);

    let before = call_path(
        router.clone(),
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(before.status, StatusCode::NOT_FOUND);
    assert_eq!(before.json()["code"], "no_key");

    let issued = call(router.clone(), "POST", Some(&session_cookie(USER_ID))).await;
    assert_eq!(issued.status, StatusCode::OK);

    // Inside the 60s TTL — only the eviction can explain a non-stale answer.
    let after = call_path(router, "GET", USAGE_PATH, Some(&session_cookie(USER_ID))).await;
    assert_eq!(
        after.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&after.body)
    );
    // A fresh key has no usage rows yet, and that is the honest shape.
    assert_eq!(after.json()["used"], serde_json::Value::Null);
}

/// Throttled with nothing cached: there is no honest number to show, so the
/// answer is a `503` that names the condition — not a `502`, because the
/// control plane is not down, it is telling us to slow down.
#[tokio::test]
async fn a_throttle_with_nothing_cached_is_a_503() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    mock.with(|s| s.throttle_usage = true);

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(reply.json()["code"], "usage_unavailable");
}

// ---------------------------------------------------------------------------
// Failure shapes
// ---------------------------------------------------------------------------

/// A control plane that stays down is a `502` — the sticky knob, because the
/// SDK retries a one-shot 500 into invisibility.
#[tokio::test]
async fn a_control_plane_failure_is_a_502() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    mock.with(|s| s.fail_usage = true);

    let reply = usage(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(reply.json()["code"], "usage_unavailable");
}

/// The wall-clock deadline turns a stalled control plane into a `503` answer
/// rather than a killed invocation with no response at all.
#[tokio::test]
async fn a_stalled_lookup_elapses_into_a_503() {
    let mock = MockGateway::start().await;
    seed_key_with_usage(&mock);
    mock.with(|s| s.list_delay_ms = 400);
    let router = usage_router_with(&mock, Duration::from_secs(60), Duration::from_millis(50));

    let reply = call_path(router, "GET", USAGE_PATH, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.json()["code"], "usage_unavailable");
}

/// The portal open with no control-plane client wired answers `503` and says
/// so, exactly as the key routes do — a portal with a dashboard that cannot
/// work must not look like a dashboard with no data.
#[tokio::test]
async fn an_unconfigured_deployment_says_so() {
    let open_but_unwired = build_app(true, None);
    let reply = call_path(
        open_but_unwired,
        "GET",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.json()["code"], "usage_unconfigured");
}

/// The route is `GET`-only. A `POST` is a `405`, not a second way in — reading
/// a counter must not share a verb with anything that writes.
#[tokio::test]
async fn usage_accepts_only_get() {
    let mock = MockGateway::start().await;
    let reply = call_path(
        app_against(&mock),
        "POST",
        USAGE_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}
