//! Replacing a key — revoke now, re-issue next period — over HTTP, end to end
//! (task 0191).
//!
//! "Replace my key" is a **revocation**: `POST /key/rework` disables the
//! caller's key immediately and issues nothing. The replacement is an ordinary
//! issue round-trip (`tests/portal_issue.rs`'s flow, driven here through the
//! same mock Discord), and the issue path refuses it until the quota period in
//! which the revocation happened has rolled — the re-issue cap, decided from
//! the disabled key's `lastUpdatedDate`.
//!
//! Three properties are the spine of this file:
//!
//! - **Revoke is immediate and total.** Every key under the name is disabled
//!   in one `POST`, with no Discord call; the reveal then says "revoked" and
//!   names the date, and never hands the dead value out again.
//! - **The replacement waits for the 1st.** An issue inside the revocation's
//!   period is `?issue=capped&next_eligible_at=…` with nothing written; the
//!   same issue once the period has rolled deletes the revocation record and
//!   creates the new key.
//! - **A session cookie can revoke its own key and nothing else.** The route
//!   is `POST`-only, writes only `enabled=false` on the caller's exact name,
//!   and every other state the store can be in leaves it untouched.

#[path = "portal_keys/harness.rs"]
mod harness;
#[path = "common/mock_discord.rs"]
mod mock_discord;

use axum::Router;
use axum::http::{StatusCode, header};
use chrono::{Datelike, NaiveDate, Utc};

use harness::*;
use mock_discord::{GRANTED_SCOPE, MemberReply, MockDiscord};
use prices_api::portal::auth::discord::Endpoints;
use prices_api::portal::keys::gateway::Gateway;
use prices_api::portal::keys::{KEY_PATH, REWORK_PATH};
use prices_api::portal::usage::USAGE_PATH;

fn app_with_discord(discord: &MockDiscord, gateway: &MockGateway) -> Router {
    build_app_with(
        true,
        Some(Gateway::against(&gateway.base, PLAN_ID.to_string())),
        Endpoints {
            api_base: discord.base.clone(),
            ..Endpoints::default()
        },
        Some(eligibility(GUILD_ID, "5")),
    )
}

fn key_name() -> String {
    format!("discord-{USER_ID}-key")
}

/// Unix seconds for a UTC date at noon.
fn noon(date: NaiveDate) -> u64 {
    date.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp() as u64
}

/// The 1st of the month `months` after this one, UTC.
fn first_of_month_offset(months: i32) -> NaiveDate {
    let today = Utc::now().date_naive();
    let total = today.year() * 12 + today.month0() as i32 + months;
    NaiveDate::from_ymd_opt(total.div_euclid(12), (total.rem_euclid(12) + 1) as u32, 1).unwrap()
}

/// "Revoked on the 3rd" of the month beginning `first`.
fn the_3rd_of(first: NaiveDate) -> u64 {
    noon(first.with_day(3).unwrap())
}

fn next_eligible_expected() -> (String, String) {
    let next = first_of_month_offset(1);
    (
        format!("{}T00:00:00Z", next.format("%Y-%m-%d")),
        next.format("%Y-%m-%d").to_string(),
    )
}

/// Seed an attached, live key for the user, created at `created_at`.
fn seed_attached(gateway: &MockGateway, created_at: u64) -> String {
    gateway.with(|s| {
        let id = s.seed(&key_name(), created_at);
        s.plan_keys.push((PLAN_ID.to_string(), id.clone()));
        id
    })
}

/// Seed a key the user revoked at `revoked_at`.
fn seed_revoked(gateway: &MockGateway, revoked_at: u64) -> String {
    gateway.with(|s| {
        let id = s.seed_revoked(&key_name(), 1_000, revoked_at);
        s.plan_keys.push((PLAN_ID.to_string(), id.clone()));
        id
    })
}

async fn revoke(router: &Router, cookie: Option<&str>) -> Reply {
    call_path(router.clone(), "POST", REWORK_PATH, cookie).await
}

async fn reveal_via(router: &Router) -> Reply {
    call_path(
        router.clone(),
        "GET",
        KEY_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await
}

// ---------------------------------------------------------------------------
// The gate — ships closed
// ---------------------------------------------------------------------------

/// The revoke is an empty `404` while the portal is closed, with a valid
/// session presented and zero control-plane calls.
#[tokio::test]
async fn revoke_is_an_empty_404_while_the_portal_is_closed() {
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    let closed = build_app(
        false,
        Some(Gateway::against(&gateway.base, PLAN_ID.to_string())),
    );

    let reply = revoke(&closed, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    assert_eq!(gateway.with(|s| s.list_calls), 0);
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
    assert!(gateway.with(|s| s.keys[0].enabled), "nothing was touched");
}

// ---------------------------------------------------------------------------
// Revoke — immediate, total, no Discord
// ---------------------------------------------------------------------------

/// The user's scenario: a key issued TODAY leaks, "Replace my key" is
/// pressed, and the key is off at once — one `UpdateApiKey`, no Discord call,
/// nothing created — with the answer naming the 1st of next month.
#[tokio::test]
async fn revoke_deactivates_the_key_immediately_and_issues_nothing() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let key = seed_attached(&gateway, now_secs());
    let app = app_with_discord(&discord, &gateway);
    let (expected_at, _) = next_eligible_expected();

    let reply = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let body = reply.json();
    assert_eq!(body["revoked"], true);
    assert_eq!(body["next_eligible_at"], expected_at);
    assert!(body["revoked_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(reply.cache_control(), "no-store");

    gateway.with(|s| {
        assert!(!s.keys[0].enabled, "the key is off");
        assert_eq!(s.keys.len(), 1, "and nothing was created");
        assert_eq!(s.ops, vec![format!("disable:{key}")]);
    });
    assert_eq!(discord.exchanges(), 0, "no Discord round-trip for a revoke");
    assert_eq!(discord.member_calls(), 0);
}

/// After the revoke the reveal never hands the dead value out again: it
/// answers `404 key_revoked` with the date, and the page renders that instead
/// of the "get my API key" link.
#[tokio::test]
async fn a_revoked_key_is_never_revealed_again_and_the_reveal_names_the_date() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    let app = app_with_discord(&discord, &gateway);
    let (expected_at, _) = next_eligible_expected();

    assert_eq!(
        revoke(&app, Some(&session_cookie(USER_ID))).await.status,
        StatusCode::OK
    );

    let revealed = reveal_via(&app).await;
    assert_eq!(revealed.status, StatusCode::NOT_FOUND);
    let body = revealed.json();
    assert_eq!(body["code"], "key_revoked");
    assert_eq!(body["details"]["next_eligible_at"], expected_at);
    assert!(body["details"]["revoked_at"].is_string());
    assert!(
        !String::from_utf8_lossy(&revealed.body).contains("CANARY"),
        "the revoked value must not appear anywhere"
    );
    assert_eq!(revealed.cache_control(), "no-store");
}

/// Idempotent: a second press answers `200` with the same dates and writes
/// nothing more — a retry after a dropped response is safe.
#[tokio::test]
async fn revoking_twice_is_one_write() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let key = seed_attached(&gateway, 1_000);
    let app = app_with_discord(&discord, &gateway);

    let first = revoke(&app, Some(&session_cookie(USER_ID))).await;
    let second = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        second.json()["next_eligible_at"],
        first.json()["next_eligible_at"]
    );
    assert_eq!(
        gateway.with(|s| s.ops.clone()),
        vec![format!("disable:{key}")]
    );
}

/// Every key under the name dies, not only the current one: a duplicate left
/// by an earlier double-submit is a working credential the visitor was just
/// told had stopped.
#[tokio::test]
async fn every_key_under_the_name_is_revoked_not_only_the_current_one() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    gateway.with(|s| s.seed(&key_name(), 2_000));
    let app = app_with_discord(&discord, &gateway);

    assert_eq!(
        revoke(&app, Some(&session_cookie(USER_ID))).await.status,
        StatusCode::OK
    );
    gateway.with(|s| {
        assert!(s.keys.iter().all(|k| !k.enabled));
        assert_eq!(s.ops.len(), 2);
    });
}

/// Revoking with no key is `404 no_key` and writes nothing.
#[tokio::test]
async fn revoking_with_no_key_is_no_key_and_writes_nothing() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = app_with_discord(&discord, &gateway);

    let reply = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json()["code"], "no_key");
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
}

/// Unauthenticated is refused before AWS is touched; a `GET` on the path is
/// not a route. The `POST`-only shape plus `SameSite=Lax` is the CSRF guard.
#[tokio::test]
async fn revoke_needs_a_session_and_the_post_verb() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    let app = app_with_discord(&discord, &gateway);

    let anonymous = revoke(&app, None).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.json()["code"], "not_signed_in");
    assert_eq!(anonymous.cache_control(), "no-store");

    let get = call_path(app, "GET", REWORK_PATH, Some(&session_cookie(USER_ID))).await;
    assert_eq!(get.status, StatusCode::METHOD_NOT_ALLOWED);

    assert_eq!(gateway.with(|s| s.list_calls), 0);
    assert!(gateway.with(|s| s.keys[0].enabled));
}

/// A forged session cannot revoke anybody's key, and one user's session
/// cannot reach another user's key — the name is derived from the signed
/// `sub`, and only exact matches are touched.
#[tokio::test]
async fn a_session_can_only_revoke_its_own_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let mine = seed_attached(&gateway, 1_000);
    let theirs = gateway.with(|s| s.seed("discord-999999999999999999-key", 1_000));
    // A prefix neighbour — `discord-<my id>-key-old` — which `nameQuery`
    // returns and the exact filter must drop.
    let lookalike = gateway.with(|s| s.seed(&format!("{}-old", key_name()), 1_000));
    let app = app_with_discord(&discord, &gateway);

    assert_eq!(
        revoke(&app, Some(&session_cookie(USER_ID))).await.status,
        StatusCode::OK
    );
    gateway.with(|s| {
        let by_id = |id: &str| s.keys.iter().find(|k| k.id == id).unwrap().enabled;
        assert!(!by_id(&mine));
        assert!(by_id(&theirs), "somebody else's key is untouched");
        assert!(by_id(&lookalike), "a console lookalike is untouched");
    });

    let forged = format!(
        "{}=not-a-real-session",
        prices_api::portal::auth::cookies::SESSION_COOKIE
    );
    let reply = revoke(&app, Some(&forged)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

/// A control plane that refuses the disable is a `502`, and "revoked" is
/// never said of a key that still works.
#[tokio::test]
async fn a_failed_disable_is_a_502_not_a_false_revoked() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    gateway.with(|s| s.fail_disables = true);
    let app = app_with_discord(&discord, &gateway);

    let reply = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.json()["code"], "key_unavailable");
    assert!(gateway.with(|s| s.keys[0].enabled));
    // And the reveal still hands the (un-revoked) key out.
    assert_eq!(reveal_via(&app).await.status, StatusCode::OK);
}

/// A successful revoke evicts the usage route's cached answer, so the next
/// dashboard load re-reads rather than serving a pre-revoke snapshot.
#[tokio::test]
async fn a_revoke_evicts_the_cached_usage() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let key = seed_attached(&gateway, 1_000);
    gateway.with(|s| {
        s.usage.insert(key.clone(), vec![vec![42, 99_958]]);
    });
    let app = app_with_discord(&discord, &gateway);
    let session = session_cookie(USER_ID);

    let before = call_path(app.clone(), "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(before.json()["used"], 42);
    assert_eq!(gateway.with(|s| s.usage_calls), 1);

    assert_eq!(revoke(&app, Some(&session)).await.status, StatusCode::OK);

    // Inside the 60s TTL, so only the eviction explains a second read. The
    // counter is preserved across a disable (0180 item 8), so the numbers
    // are the same — and honest.
    let after = call_path(app, "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(after.json()["used"], 42);
    assert_eq!(gateway.with(|s| s.usage_calls), 2);
}

// ---------------------------------------------------------------------------
// The re-issue cap — the replacement waits for the 1st
// ---------------------------------------------------------------------------

/// Revoke, then "Get my API key" in the same period: eligibility passes and
/// the issue is still refused — `?issue=capped&next_eligible_at=…` — with
/// nothing created and the revoked key left as the record.
#[tokio::test]
async fn an_issue_after_a_revoke_in_the_same_period_is_capped_with_the_date() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let key = seed_attached(&gateway, 1_000);
    let app = app_with_discord(&discord, &gateway);
    let (_, expected_date) = next_eligible_expected();

    assert_eq!(
        revoke(&app, Some(&session_cookie(USER_ID))).await.status,
        StatusCode::OK
    );

    let reply = issue_round_trip(&app).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(
        reply.location(),
        format!("/api-tokens/?issue=capped&next_eligible_at={expected_date}")
    );
    assert_eq!(
        discord.member_calls(),
        1,
        "eligibility ran and passed first"
    );
    gateway.with(|s| {
        assert_eq!(s.create_calls, 0);
        assert_eq!(s.keys.len(), 1);
        assert_eq!(s.keys[0].id, key);
        assert!(!s.keys[0].enabled, "the revocation record stays");
        assert_eq!(s.deleted.len(), 0);
    });
    // And the reveal keeps saying revoked, not "no key".
    assert_eq!(reveal_via(&app).await.json()["code"], "key_revoked");
}

/// The worked example against the real calendar: revoked on the 3rd of THIS
/// month → capped until the 1st of next; revoked on the 3rd of LAST month →
/// the period has rolled, the revocation record is deleted, a new key is
/// created and attached, and the reveal hands the NEW one out.
#[tokio::test]
async fn revoked_on_the_3rd_refuses_until_the_1st_and_issues_once_it_has_passed() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let (expected_at, expected_date) = next_eligible_expected();

    // This month's 3rd: capped.
    let gateway = MockGateway::start().await;
    seed_revoked(&gateway, the_3rd_of(first_of_month_offset(0)));
    let app = app_with_discord(&discord, &gateway);
    assert_eq!(
        issue_round_trip(&app).await.location(),
        format!("/api-tokens/?issue=capped&next_eligible_at={expected_date}")
    );
    assert_eq!(gateway.with(|s| s.create_calls), 0);
    let revealed = reveal_via(&app).await;
    assert_eq!(revealed.json()["code"], "key_revoked");
    assert_eq!(revealed.json()["details"]["next_eligible_at"], expected_at);

    // Last month's 3rd: the 1st has passed.
    let gateway = MockGateway::start().await;
    let dead = seed_revoked(&gateway, the_3rd_of(first_of_month_offset(-1)));
    let app = app_with_discord(&discord, &gateway);
    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    gateway.with(|s| {
        assert_eq!(
            s.deleted,
            vec![dead.clone()],
            "the revocation record is gone"
        );
        assert_eq!(s.keys.len(), 1);
        assert_ne!(s.keys[0].id, dead);
        assert!(s.keys[0].enabled);
        assert!(
            s.plan_keys
                .contains(&(PLAN_ID.to_string(), s.keys[0].id.clone()))
        );
        let new = s.keys[0].id.clone();
        assert_eq!(
            s.ops,
            vec![
                format!("delete:{dead}"),
                format!("create:{new}"),
                format!("attach:{new}")
            ]
        );
    });
    let revealed = reveal_via(&app).await;
    assert_eq!(revealed.status, StatusCode::OK);
    assert_eq!(
        revealed.json()["key_id"],
        gateway.with(|s| s.keys[0].id.clone())
    );
}

/// A revoked key from last month — whose replacement is therefore due — is
/// "no key" on the reveal until the owner presses the issue link: the dead
/// value is never revealed, and the page offers the round-trip.
#[tokio::test]
async fn a_revocation_whose_period_has_rolled_reveals_as_no_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_revoked(&gateway, the_3rd_of(first_of_month_offset(-1)));
    let app = app_with_discord(&discord, &gateway);

    let revealed = reveal_via(&app).await;
    assert_eq!(revealed.status, StatusCode::NOT_FOUND);
    assert_eq!(revealed.json()["code"], "no_key");
    assert_eq!(
        gateway.with(|s| s.ops.len()),
        0,
        "the reveal still writes nothing"
    );
}

/// The LATEST revocation governs: a duplicate revoked last month beside a key
/// revoked this month must not open the door this month's revocation closed.
#[tokio::test]
async fn the_latest_revocation_governs_the_cap() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_revoked(&gateway, the_3rd_of(first_of_month_offset(-1)));
    seed_revoked(&gateway, the_3rd_of(first_of_month_offset(0)));
    let app = app_with_discord(&discord, &gateway);

    assert!(
        issue_round_trip(&app)
            .await
            .location()
            .starts_with("/api-tokens/?issue=capped")
    );
    assert_eq!(gateway.with(|s| s.create_calls), 0);
    assert_eq!(gateway.with(|s| s.deleted.len()), 0);
}

/// The cap refuses only after eligibility: a non-member who revoked is told
/// to rejoin, not to wait — and nothing is written either way.
#[tokio::test]
async fn membership_is_still_checked_before_the_cap() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::NotFound { code: 10_007 },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;
    seed_revoked(&gateway, now_secs());
    let app = app_with_discord(&discord, &gateway);

    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=not_member"
    );
    assert_eq!(gateway.with(|s| s.list_calls), 0);
}

/// A live key beside a revoked one (a console re-enable, a duplicate): the
/// live key is the current key — revealed, adopted — and the revoked record
/// is swept by the next issue like any duplicate.
#[tokio::test]
async fn a_live_key_beside_a_revoked_one_is_the_current_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let dead = seed_revoked(&gateway, now_secs());
    let live = seed_attached(&gateway, 2_000);
    let app = app_with_discord(&discord, &gateway);

    assert_eq!(reveal_via(&app).await.json()["key_id"], live);

    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    gateway.with(|s| {
        assert_eq!(s.deleted, vec![dead.clone()]);
        assert_eq!(s.keys.len(), 1);
        assert_eq!(s.keys[0].id, live);
        assert_eq!(s.create_calls, 0);
    });
}

/// Regression: a live key from last month is still simply adopted by the
/// issue — the cap exists only for revoked keys.
#[tokio::test]
async fn a_live_key_is_adopted_as_before_the_cap_existed() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let key = seed_attached(&gateway, 1_000);
    let app = app_with_discord(&discord, &gateway);

    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    assert_eq!(gateway.with(|s| s.create_calls), 0);
    assert_eq!(reveal_via(&app).await.json()["key_id"], key);
}

/// The full cycle on one router: issue → revoke → issue refused → (period
/// rolls, simulated by back-dating the revocation) → issue creates the new
/// key and the old value is gone for good.
#[tokio::test]
async fn the_full_cycle_issue_revoke_wait_reissue() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = app_with_discord(&discord, &gateway);
    let session = session_cookie(USER_ID);

    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    let first = gateway.with(|s| s.keys[0].clone());

    assert_eq!(revoke(&app, Some(&session)).await.status, StatusCode::OK);
    assert!(
        issue_round_trip(&app)
            .await
            .location()
            .starts_with("/api-tokens/?issue=capped")
    );

    // The 1st arrives.
    gateway.with(|s| {
        s.keys[0].last_updated_at = the_3rd_of(first_of_month_offset(-1));
    });
    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );

    gateway.with(|s| {
        assert_eq!(s.keys.len(), 1);
        assert_ne!(s.keys[0].id, first.id);
        assert_ne!(s.keys[0].value, first.value);
        assert!(s.deleted.contains(&first.id));
    });
    let revealed = reveal_via(&app).await;
    assert_eq!(revealed.status, StatusCode::OK);
    assert_ne!(revealed.json()["value"], first.value);
}

/// Every revoke answer is uncacheable and sets no cookie.
#[tokio::test]
async fn every_revoke_answer_carries_no_store() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = app_with_discord(&discord, &gateway);

    for reply in [
        revoke(&app, None).await,
        revoke(&app, Some(&session_cookie(USER_ID))).await,
    ] {
        assert_eq!(reply.cache_control(), "no-store");
        assert!(reply.headers.get(header::SET_COOKIE).is_none());
    }
}
