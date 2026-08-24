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
use prices_api::portal::keys::{KEY_PATH, PORTAL_REQUEST_HEADER, REWORK_PATH};
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

/// The portal page's revoke: `POST` with the same-origin marker header the
/// backend requires (`PORTAL_REQUEST_HEADER`).
async fn revoke(router: &Router, cookie: Option<&str>) -> Reply {
    call_path_with(
        router.clone(),
        "POST",
        REWORK_PATH,
        cookie,
        &[PORTAL_REQUEST_HEADER, ("sec-fetch-site", "same-origin")],
    )
    .await
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
/// pressed, and the key is disabled on the control plane in one
/// `UpdateApiKey` — no Discord call, nothing created — with the answer naming
/// the 1st of next month and the revocation instant. (The data plane follows
/// in ~25 s, 0180 item 8; that window is the page's to state, and it does.)
#[tokio::test]
async fn revoke_disables_the_key_in_one_call_and_issues_nothing() {
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

/// The ordinary answer carries no `partial` at all — the flag is omitted, not
/// sent as `false`, so a client that never learned about it reads the same
/// shape it always did.
#[tokio::test]
async fn a_clean_revocation_omits_the_partial_flag() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    let app = app_with_discord(&discord, &gateway);

    let reply = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(
        reply.json().get("partial").is_none(),
        "a clean revocation sends no partial flag: {}",
        reply.json()
    );
}

/// A disable that fails for ONE key of a pair is reported as partial — not
/// dressed up as a clean revocation, and not thrown away as a `502` either.
///
/// The visitor's own key is off, so `502` would be a lie in the other
/// direction (and would have re-armed a dialog whose work was half done). The
/// answer carries `partial`, which is what stops the page rendering a plain
/// "revoked" while a duplicate still answers on `/v1/`.
#[tokio::test]
async fn a_partial_revocation_is_reported_as_partial() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let first = seed_attached(&gateway, 1_000);
    let stubborn = gateway.with(|s| s.seed(&key_name(), 2_000));
    gateway.with(|s| s.fail_disable_of = vec![stubborn.clone()]);
    let app = app_with_discord(&discord, &gateway);

    let reply = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["revoked"], true);
    assert_eq!(reply.json()["partial"], true);
    assert!(
        !reply.json()["revoked_at"].is_null(),
        "the disables that landed are still dated"
    );
    gateway.with(|s| {
        let by_id = |id: &str| s.keys.iter().find(|k| k.id == id).unwrap().enabled;
        assert!(!by_id(&first), "the key that could be disabled is off");
        assert!(by_id(&stubborn), "the one that refused is untouched");
    });
    // And the cap does NOT bite, because a live key survived: the issue path
    // adopts it rather than refusing. That is the whole reason `partial` has to
    // reach the page — the backend cannot pretend this was a revocation, and
    // the visitor's next move is to press Replace again, not to wait for the
    // 1st.
    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    assert_eq!(
        gateway.with(|s| s.create_calls),
        0,
        "nothing new was minted"
    );
}

/// Every enabled key raced away between the listing and the patch: `no_key`,
/// not a phantom "deactivated".
///
/// Nothing was written, so there is no disabled record for the cap to read —
/// and the very next issue press finds the name empty and creates a key
/// outright. Answering `Done` here would have promised a wait the issue path
/// does not honour.
#[tokio::test]
async fn a_revocation_that_raced_away_is_no_key_not_a_phantom_record() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let gone = seed_attached(&gateway, 1_000);
    // Listed, but no longer in the store: the patch 404s, which is the
    // deletion race, not a failure.
    gateway.with(|s| {
        let key = s.keys.iter().find(|k| k.id == gone).unwrap().clone();
        s.deleted_keys.push(key);
        s.keys.retain(|k| k.id != gone);
        s.list_resurrects_deleted = true;
    });
    let app = app_with_discord(&discord, &gateway);

    let reply = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json()["code"], "no_key");
}

/// The revocation instant in the answer is the control plane's
/// `lastUpdatedDate`, read off the patch response — not this process's clock.
///
/// The two are seconds apart in the ordinary case and invisible; across 00:00
/// UTC on the 1st they fall in different quota periods, and the page would then
/// name a `next_eligible_at` a month before the issue round-trip would honour
/// it. Pinning the mock's stamp to a date of our choosing is the only way to
/// tell which of the two clocks the answer used.
#[tokio::test]
async fn the_revocation_instant_comes_from_the_control_plane_not_our_clock() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    // Last month's 3rd — a period that has already rolled, which our own clock
    // could never produce for a revocation happening now.
    let stamped = the_3rd_of(first_of_month_offset(-1));
    gateway.with(|s| s.disable_stamps_at = Some(stamped));
    let app = app_with_discord(&discord, &gateway);

    let reply = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json()["revoked_at"].as_str().unwrap(),
        chrono::DateTime::from_timestamp(stamped as i64, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );
    // And the cap agrees with it rather than with `now`: that period has
    // rolled, so a key is due immediately.
    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
}

/// A revoked record that cannot be deleted does not withhold the new key.
///
/// The post-roll cleanup runs on EVERY issue press once the name holds nothing
/// but revoked keys, so propagating its first failure made one undeletable
/// record — an untagged exact-name key made by hand in the console, against a
/// tag-scoped `DELETE` — a permanent `?issue=failed` with no in-product
/// recovery. Logged and stepped over instead, exactly as the loser sweep does.
#[tokio::test]
async fn an_undeletable_revoked_record_does_not_block_the_re_issue() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let dead = seed_revoked(&gateway, the_3rd_of(first_of_month_offset(-1)));
    gateway.with(|s| s.fail_delete_of = vec![dead.clone()]);
    let app = app_with_discord(&discord, &gateway);

    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    gateway.with(|s| {
        let live: Vec<_> = s.keys.iter().filter(|k| k.enabled).collect();
        assert_eq!(live.len(), 1, "the visitor got a working key");
        assert_ne!(live[0].id, dead);
        assert!(
            s.plan_keys
                .contains(&(PLAN_ID.to_string(), live[0].id.clone()))
        );
        assert!(
            s.keys.iter().any(|k| k.id == dead && !k.enabled),
            "the undeletable record is left for the next reconciliation"
        );
    });
    // The new key is what the reveal hands out, not the stale record.
    let revealed = reveal_via(&app).await;
    assert_eq!(revealed.status, StatusCode::OK);
    assert_ne!(revealed.json()["name"], serde_json::Value::Null);
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

/// An idempotent revoke of a key revoked in an EARLIER period does not tell
/// the visitor to wait another month: `next_eligible_at` comes from
/// `cap::decide`, like the reveal's and the issue path's, so a stale tab that
/// re-`POST`s cannot hide an issue link the round-trip would have honoured.
#[tokio::test]
async fn an_idempotent_revoke_after_the_period_rolled_says_a_key_is_due_now() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let revoked_at = the_3rd_of(first_of_month_offset(-1));
    seed_revoked(&gateway, revoked_at);
    let app = app_with_discord(&discord, &gateway);

    let reply = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let next = reply.json()["next_eligible_at"]
        .as_str()
        .unwrap()
        .to_owned();
    let (next_month, _) = next_eligible_expected();
    assert_ne!(
        next, next_month,
        "the period has rolled; the answer must not name the next one"
    );
    // "From now", so the page offers the issue link instead of a date.
    let announced = chrono::DateTime::parse_from_rfc3339(&next).unwrap();
    assert!(announced.timestamp() as u64 <= now_secs() + 5);
    // The recorded instant, not "now" and not the epoch.
    assert_eq!(
        reply.json()["revoked_at"],
        serde_json::json!(
            chrono::DateTime::<chrono::Utc>::from_timestamp(revoked_at as i64, 0)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        )
    );
    assert_eq!(gateway.with(|s| s.ops.len()), 0, "nothing was written");
}

/// An undated revocation record beside a dated one must not erase the date:
/// a `None` instant caps against a `next_eligible_at` recomputed from the
/// current period on every read, which rolls forward forever.
#[tokio::test]
async fn an_undated_duplicate_does_not_lock_the_owner_out_forever() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_revoked(&gateway, the_3rd_of(first_of_month_offset(-1)));
    gateway.with(|s| {
        let id = s.seed_revoked(&key_name(), 900, 0);
        s.undate(&id);
        s.plan_keys.push((PLAN_ID.to_string(), id));
    });
    let app = app_with_discord(&discord, &gateway);

    // Last month's revocation is the one that governs, so the replacement is
    // due: the reveal says "no key" and the round-trip issues.
    let revealed = reveal_via(&app).await;
    assert_eq!(revealed.json()["code"], "no_key");
    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    assert_eq!(gateway.with(|s| s.create_calls), 1);
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
        s.keys[0].last_updated_at = Some(the_3rd_of(first_of_month_offset(-1)));
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

// ---------------------------------------------------------------------------
// Review findings (2026-08-21 audit)
// ---------------------------------------------------------------------------

/// A revoke without the portal's own request marker — a cross-site form
/// `POST` that `SameSite=Lax` would let through once the portal and another
/// page share a registrable domain — is refused before the session is read,
/// and so is one that carries the marker but says `Sec-Fetch-Site:
/// cross-site`. Nothing is written either way.
#[tokio::test]
async fn a_revoke_without_the_same_origin_markers_is_refused_before_anything_is_read() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    let app = app_with_discord(&discord, &gateway);
    let session = session_cookie(USER_ID);

    for (label, headers) in [
        ("no marker", vec![]),
        ("wrong marker", vec![("x-requested-with", "XMLHttpRequest")]),
        (
            "marker but cross-site",
            vec![PORTAL_REQUEST_HEADER, ("sec-fetch-site", "cross-site")],
        ),
        (
            "marker but same-site sibling host",
            vec![PORTAL_REQUEST_HEADER, ("sec-fetch-site", "same-site")],
        ),
    ] {
        let reply =
            call_path_with(app.clone(), "POST", REWORK_PATH, Some(&session), &headers).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{label}");
        assert_eq!(reply.json()["code"], "cross_site_request", "{label}");
        assert_eq!(reply.cache_control(), "no-store", "{label}");
    }
    assert_eq!(gateway.with(|s| s.list_calls), 0, "refused before AWS");
    assert!(gateway.with(|s| s.keys[0].enabled));

    // A typed-URL / bookmark style request (`none`) and a browser that sends
    // no fetch metadata at all are both fine WITH the marker.
    for headers in [
        vec![PORTAL_REQUEST_HEADER, ("sec-fetch-site", "none")],
        vec![PORTAL_REQUEST_HEADER],
    ] {
        let reply =
            call_path_with(app.clone(), "POST", REWORK_PATH, Some(&session), &headers).await;
        assert_eq!(reply.status, StatusCode::OK);
    }
}

/// The reveal and the issue path decide the cap from the SAME instant — the
/// latest revocation — so two revocation records from different months can
/// never make the page offer an issue the round-trip refuses.
#[tokio::test]
async fn the_reveal_and_the_issue_agree_on_the_cap_with_mixed_period_revocations() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    // Older record revoked last month, newer one revoked this month.
    seed_revoked(&gateway, the_3rd_of(first_of_month_offset(-1)));
    seed_revoked(&gateway, the_3rd_of(first_of_month_offset(0)));
    let app = app_with_discord(&discord, &gateway);
    let (expected_at, expected_date) = next_eligible_expected();

    let revealed = reveal_via(&app).await;
    assert_eq!(revealed.json()["code"], "key_revoked", "not no_key");
    assert_eq!(revealed.json()["details"]["next_eligible_at"], expected_at);

    assert_eq!(
        issue_round_trip(&app).await.location(),
        format!("/api-tokens/?issue=capped&next_eligible_at={expected_date}")
    );

    // And the revoke's idempotent answer names the same instant.
    let again = revoke(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(again.json()["next_eligible_at"], expected_at);
}

/// The usage section reads the same key the reveal does: a live key beside a
/// revoked record shows its OWN counter, and a revocation whose period has
/// rolled answers `no_key` like the reveal — never a record the next issue
/// will delete.
#[tokio::test]
async fn usage_follows_the_same_key_as_the_reveal() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let session = session_cookie(USER_ID);

    // Live beside revoked (the revoked one is OLDER, so a blind "earliest
    // wins" would pick it).
    let gateway = MockGateway::start().await;
    let dead = seed_revoked(&gateway, now_secs());
    let live = seed_attached(&gateway, 2_000);
    gateway.with(|s| {
        s.usage.insert(dead.clone(), vec![vec![999, 99_001]]);
        s.usage.insert(live.clone(), vec![vec![7, 99_993]]);
    });
    let app = app_with_discord(&discord, &gateway);
    let usage = call_path(app.clone(), "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(
        usage.json()["used"],
        7,
        "{}",
        String::from_utf8_lossy(&usage.body)
    );
    assert_eq!(
        gateway.with(|s| s.usage_queries.last().unwrap().0.clone()),
        live
    );

    // Revoked this period: the counter is preserved and still shown.
    let gateway = MockGateway::start().await;
    let dead = seed_revoked(&gateway, now_secs());
    gateway.with(|s| {
        s.usage.insert(dead.clone(), vec![vec![42, 99_958]]);
    });
    let app = app_with_discord(&discord, &gateway);
    let usage = call_path(app.clone(), "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(usage.status, StatusCode::OK);
    assert_eq!(usage.json()["used"], 42);

    // Revoked LAST period: the reveal says no_key; so does usage.
    let gateway = MockGateway::start().await;
    let dead = seed_revoked(&gateway, the_3rd_of(first_of_month_offset(-1)));
    gateway.with(|s| {
        s.usage.insert(dead.clone(), vec![vec![42, 99_958]]);
    });
    let app = app_with_discord(&discord, &gateway);
    assert_eq!(reveal_via(&app).await.json()["code"], "no_key");
    let usage = call_path(app, "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(usage.status, StatusCode::NOT_FOUND);
    assert_eq!(usage.json()["code"], "no_key");
}

/// Eventual consistency on the post-roll re-issue: the listing after the
/// create still shows the record just deleted. It must not be ranked — it
/// would win (earliest), its attach would 404, and the single retry would be
/// spent on a phantom, leaving the NEW key created and unattached.
#[tokio::test]
async fn a_stale_listing_after_the_roll_does_not_rank_the_deleted_record() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let dead = seed_revoked(&gateway, the_3rd_of(first_of_month_offset(-1)));
    // Every listing from now on includes whatever has been deleted — so the
    // re-listing after the record is deleted and the new key created still
    // shows the dead record, exactly as a lagging `GetApiKeys` would.
    gateway.with(|s| s.list_resurrects_deleted = true);
    let app = app_with_discord(&discord, &gateway);

    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    gateway.with(|s| {
        assert_eq!(s.keys.len(), 1);
        assert_ne!(s.keys[0].id, dead);
        assert!(s.keys[0].enabled);
        assert!(
            s.plan_keys
                .contains(&(PLAN_ID.to_string(), s.keys[0].id.clone())),
            "the new key is attached — not orphaned by a phantom winner"
        );
        assert_eq!(s.create_calls, 1);
    });
}

/// `CreateApiKey` is never retried by the SDK: a create whose request landed
/// but whose response was lost makes ONE key, not two or three, and the
/// round-trip lands `failed` honestly rather than minting duplicates.
#[tokio::test]
async fn a_lost_create_response_is_not_retried_into_duplicates() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    gateway.with(|s| s.fail_next_create_after_creating = true);
    let app = app_with_discord(&discord, &gateway);

    let reply = issue_round_trip(&app).await;
    assert_eq!(reply.location(), "/api-tokens/?issue=failed");
    assert_eq!(
        gateway.with(|s| s.create_calls),
        1,
        "one create request, however the SDK would like to retry it"
    );
    // The next press adopts the key that landed, as before.
    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    assert_eq!(gateway.with(|s| s.create_calls), 1);
}
