//! Replacing a key — the rework round-trip and its pre-check, over HTTP,
//! end to end (task 0191).
//!
//! Every test drives the real router through the real flow, the way
//! `tests/portal_issue.rs` does for issuance: `GET /auth/login?action=rework`
//! mints the signed state pair, the callback exchanges the code against a
//! **mock Discord**, asks it for the guild membership — and **not** the
//! account age — decides the once-per-period cap from the current key's
//! `createdDate`, and only then swaps the key against a **mock control
//! plane**. Both mocks record what they were asked, so the assertions are
//! about what the flow *did*: which calls happened, in which order, and how
//! many keys were created and deleted.
//!
//! Three properties are the spine of this file:
//!
//! - **Never keyless.** The new key is created and attached before the old
//!   one is deleted, asserted on the mock's write *sequence* rather than on
//!   the end state; and every failure leaves the visitor holding exactly the
//!   key they had.
//! - **One rework per quota period.** A key created inside the period —
//!   issued or reworked, it makes no difference — is refused with `409` and
//!   a `next_eligible_at` that is the 1st of the next month, on the pre-check
//!   and on the round-trip alike.
//! - **Unreachable with a session cookie alone.** The only thing a session
//!   reaches is the read-only pre-check; the swap needs the round-trip.

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
use prices_api::portal::auth::{cookies, state_token};
use prices_api::portal::keys::gateway::Gateway;
use prices_api::portal::keys::{KEY_PATH, REWORK_PATH};
use prices_api::portal::usage::USAGE_PATH;

fn rework_app(discord: &MockDiscord, gateway: &MockGateway) -> Router {
    rework_app_with(discord, gateway, GUILD_ID, "5")
}

fn rework_app_with(
    discord: &MockDiscord,
    gateway: &MockGateway,
    guild_id: &str,
    min_age_minutes: &str,
) -> Router {
    build_app_with(
        true,
        Some(Gateway::against(&gateway.base, PLAN_ID.to_string())),
        Endpoints {
            api_base: discord.base.clone(),
            ..Endpoints::default()
        },
        Some(eligibility(guild_id, min_age_minutes)),
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

/// The meeting's worked example, relative to the real calendar: "reworked on
/// the 3rd" of the current month, and the 3rd of the previous month.
fn the_3rd_of(first: NaiveDate) -> u64 {
    noon(first.with_day(3).unwrap())
}

fn next_eligible_at_expected() -> (String, String) {
    let next = first_of_month_offset(1);
    (
        format!("{}T00:00:00Z", next.format("%Y-%m-%d")),
        next.format("%Y-%m-%d").to_string(),
    )
}

/// Seed an attached key for the user, created at `created_at`.
fn seed_attached(gateway: &MockGateway, created_at: u64) -> String {
    gateway.with(|s| {
        let id = s.seed(&key_name(), created_at);
        s.plan_keys.push((PLAN_ID.to_string(), id.clone()));
        id
    })
}

async fn precheck(router: &Router, cookie: Option<&str>) -> Reply {
    call_path(router.clone(), "POST", REWORK_PATH, cookie).await
}

// ---------------------------------------------------------------------------
// The gate — ships closed
// ---------------------------------------------------------------------------

/// The whole rework surface is an empty `404` while the portal is closed —
/// the login that starts it, the callback that finishes it, and the
/// pre-check — with zero calls to Discord and zero to the control plane.
#[tokio::test]
async fn everything_including_rework_is_an_empty_404_while_the_portal_is_closed() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    let closed = build_app_with(
        false,
        Some(Gateway::against(&gateway.base, PLAN_ID.to_string())),
        Endpoints {
            api_base: discord.base.clone(),
            ..Endpoints::default()
        },
        Some(eligibility(GUILD_ID, "5")),
    );

    for (method, path) in [
        ("GET", "/api-tokens/api/auth/login?action=rework"),
        ("GET", "/api-tokens/api/auth/callback?code=c&state=s"),
        ("POST", REWORK_PATH),
    ] {
        let reply = call_path(closed.clone(), method, path, Some(&session_cookie(USER_ID))).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{method} {path}");
        assert!(reply.body.is_empty(), "{method} {path} carried a body");
    }
    assert_eq!(discord.exchanges(), 0);
    assert_eq!(discord.member_calls(), 0);
    assert_eq!(gateway.with(|s| s.list_calls), 0);
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
}

// ---------------------------------------------------------------------------
// The happy path — a swap, never keyless
// ---------------------------------------------------------------------------

/// A member with a key from last period gets a new one: the old is deleted,
/// the new is enabled and on the plan, the landing is `?rework=ok` with a
/// fresh session — and the new value rides nowhere in the redirect.
#[tokio::test]
async fn a_member_replaces_their_key_and_the_old_one_is_gone() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    let app = rework_app(&discord, &gateway);

    let reply = rework_round_trip(&app).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api-tokens/?rework=ok");
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_some());

    let stored = gateway.with(|s| s.keys.clone());
    assert_eq!(stored.len(), 1, "exactly one key survives");
    assert_ne!(stored[0].id, old, "and it is not the old one");
    assert_eq!(stored[0].name, key_name());
    assert!(stored[0].enabled);
    assert!(
        gateway.with(|s| s
            .plan_keys
            .contains(&(PLAN_ID.to_string(), stored[0].id.clone()))),
        "the new key is on the plan"
    );
    assert_eq!(gateway.with(|s| s.deleted.clone()), vec![old]);

    let headers = format!("{:?}", reply.headers);
    assert!(!headers.contains(&stored[0].value), "{headers}");
    assert!(!headers.contains("CANARY"), "{headers}");

    // The page's next step: the reveal hands out the NEW key.
    let revealed = reveal(&gateway, USER_ID).await;
    assert_eq!(revealed.status, StatusCode::OK);
    assert_eq!(revealed.json()["key_id"], stored[0].id);
}

/// **The ordering is the acceptance criterion.** The new key is created and
/// attached before the old one is deleted, so there is no instant at which
/// the visitor holds no working key. Asserted on the mock's write sequence:
/// the end state cannot tell a swap from a delete-then-create.
#[tokio::test]
async fn the_new_key_is_created_and_attached_before_the_old_is_deleted() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);

    assert_eq!(
        rework_round_trip(&rework_app(&discord, &gateway))
            .await
            .location(),
        "/api-tokens/?rework=ok"
    );

    let ops = gateway.with(|s| s.ops.clone());
    let new = gateway.with(|s| s.keys[0].id.clone());
    assert_eq!(
        ops,
        vec![
            format!("create:{new}"),
            format!("attach:{new}"),
            format!("delete:{old}")
        ],
        "create → attach → delete, and nothing else"
    );
}

/// What the data plane will do is a consequence of what the control plane
/// holds: the old key id no longer exists (→ `403 Forbidden` on `/v1/`, the
/// same answer as no key at all — measured under 0180 item 8), and the new
/// key is enabled and attached (→ `200`). The live `curl` of both is the
/// deploy-time check `packages/prices-api/README.md` names; this pins the
/// two facts it depends on.
#[tokio::test]
async fn the_old_key_is_deleted_and_the_new_one_is_enabled_and_on_the_plan() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);

    rework_round_trip(&rework_app(&discord, &gateway)).await;

    gateway.with(|s| {
        assert!(!s.keys.iter().any(|k| k.id == old), "the old key is gone");
        let new = &s.keys[0];
        assert!(new.enabled);
        assert!(s.plan_keys.contains(&(PLAN_ID.to_string(), new.id.clone())));
        // The old attachment is irrelevant once the key is gone, but the
        // survivor must be the ONLY attached live key.
        let live_attached: Vec<_> = s
            .plan_keys
            .iter()
            .filter(|(_, id)| s.keys.iter().any(|k| &k.id == id))
            .collect();
        assert_eq!(live_attached.len(), 1);
    });
}

/// Duplicates left by an earlier double-submit are all "the old key": every
/// one of them dies, or the visitor keeps a working credential they were
/// told had stopped.
#[tokio::test]
async fn every_old_key_under_the_name_is_replaced_not_only_the_winner() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let first = seed_attached(&gateway, 1_000);
    let second = gateway.with(|s| s.seed(&key_name(), 2_000));

    assert_eq!(
        rework_round_trip(&rework_app(&discord, &gateway))
            .await
            .location(),
        "/api-tokens/?rework=ok"
    );

    let mut deleted = gateway.with(|s| s.deleted.clone());
    deleted.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(deleted, expected);
    assert_eq!(gateway.with(|s| s.keys.len()), 1);
}

/// However two simultaneous reworks interleave (two tabs — one tab's confirm
/// is disabled on submit), the visitor ends with a working key: never none,
/// every survivor on the plan, the old one gone. Two survivors are possible
/// and accepted — both are capped until the 1st, and the next issue
/// round-trip's reconciler converges them.
#[tokio::test]
async fn two_simultaneous_reworks_never_leave_the_user_keyless() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    let app = rework_app(&discord, &gateway);

    let (a, b) = tokio::join!(rework_round_trip(&app), rework_round_trip(&app));
    for reply in [&a, &b] {
        assert!(
            reply.location() == "/api-tokens/?rework=ok"
                || reply.location() == "/api-tokens/?rework=failed",
            "{}",
            reply.location()
        );
    }

    gateway.with(|s| {
        assert!(!s.keys.iter().any(|k| k.id == old), "the old key is gone");
        assert!(!s.keys.is_empty(), "never keyless");
        assert!(s.keys.len() <= 2);
        for key in &s.keys {
            assert!(
                s.plan_keys.contains(&(PLAN_ID.to_string(), key.id.clone())),
                "every survivor is on the plan"
            );
        }
    });
    assert_eq!(reveal(&gateway, USER_ID).await.status, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// The cap — once per quota period
// ---------------------------------------------------------------------------

/// A second rework in the same period is refused — `409` with
/// `next_eligible_at` on the pre-check, `?rework=capped&next_eligible_at=…`
/// on the round-trip — and nothing moves: the key the first rework made is
/// the key that stays.
#[tokio::test]
async fn a_second_rework_in_the_same_period_is_refused_with_409_and_next_eligible_at() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    let app = rework_app(&discord, &gateway);

    assert_eq!(
        rework_round_trip(&app).await.location(),
        "/api-tokens/?rework=ok"
    );
    let after_first = gateway.with(|s| {
        (
            s.keys.iter().map(|k| k.id.clone()).collect::<Vec<_>>(),
            s.ops.clone(),
        )
    });
    let (expected_at, expected_date) = next_eligible_at_expected();

    // The pre-check: the modal's question, answered before any round-trip.
    let check = precheck(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(
        check.status,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&check.body)
    );
    let body = check.json();
    assert_eq!(body["code"], "rework_capped");
    assert_eq!(body["details"]["next_eligible_at"], expected_at);
    assert_eq!(check.cache_control(), "no-store");

    // The round-trip, should a client skip the pre-check.
    let reply = rework_round_trip(&app).await;
    assert_eq!(
        reply.location(),
        format!("/api-tokens/?rework=capped&next_eligible_at={expected_date}")
    );
    assert_eq!(
        gateway.with(|s| {
            (
                s.keys.iter().map(|k| k.id.clone()).collect::<Vec<_>>(),
                s.ops.clone(),
            )
        }),
        after_first,
        "a capped rework writes nothing"
    );
}

/// The meeting's worked example against the real calendar: a key reworked on
/// the 3rd of THIS month is refused until the 1st of next month; the same
/// key dated the 3rd of LAST month — i.e. the calendar has rolled past the
/// 1st — is allowed. (The literal 3 August → 1 September dates are pinned
/// in `keys::cap`'s unit tests; this is the same rule over HTTP.)
#[tokio::test]
async fn reworked_on_the_3rd_refuses_until_the_1st_and_succeeds_once_it_has_passed() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let (expected_at, _) = next_eligible_at_expected();

    // This month's 3rd: capped, naming next month's 1st.
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, the_3rd_of(first_of_month_offset(0)));
    let app = rework_app(&discord, &gateway);
    let refused = precheck(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json()["details"]["next_eligible_at"], expected_at);
    assert_eq!(gateway.with(|s| s.ops.len()), 0);

    // Last month's 3rd: the 1st has passed, so the rework goes through.
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, the_3rd_of(first_of_month_offset(-1)));
    let app = rework_app(&discord, &gateway);
    let allowed = precheck(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(allowed.status, StatusCode::OK);
    assert_eq!(allowed.json()["eligible"], true);
    assert_eq!(
        rework_round_trip(&app).await.location(),
        "/api-tokens/?rework=ok"
    );
}

/// The cap is about creation, not about reworks: a key **issued** this
/// period (the issue round-trip, seconds ago) cannot be reworked this period
/// either — which is the loophole the fallback closes.
#[tokio::test]
async fn a_key_issued_this_period_cannot_be_reworked_into_a_clean_counter() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = rework_app(&discord, &gateway);

    assert_eq!(
        issue_round_trip(&app).await.location(),
        "/api-tokens/?issue=ok"
    );
    let issued = gateway.with(|s| s.keys[0].id.clone());

    let reply = rework_round_trip(&app).await;
    assert!(
        reply
            .location()
            .starts_with("/api-tokens/?rework=capped&next_eligible_at="),
        "{}",
        reply.location()
    );
    assert_eq!(gateway.with(|s| s.keys[0].id.clone()), issued);
    assert_eq!(gateway.with(|s| s.deleted.len()), 0);
}

// ---------------------------------------------------------------------------
// Unreachable with a session cookie alone
// ---------------------------------------------------------------------------

/// The one route a session cookie reaches is the pre-check, and the
/// pre-check writes nothing — on the allowed answer, on the capped answer,
/// and on the no-key answer alike. A callback presented with a session but
/// no signed state is a `400` before any Discord call.
#[tokio::test]
async fn rework_is_unreachable_with_a_session_cookie_alone() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);
    let app = rework_app(&discord, &gateway);
    let session = session_cookie(USER_ID);

    let allowed = precheck(&app, Some(&session)).await;
    assert_eq!(allowed.status, StatusCode::OK);
    assert_eq!(allowed.cache_control(), "no-store");
    assert_eq!(
        gateway.with(|s| s.ops.len()),
        0,
        "an allowed pre-check writes nothing"
    );
    assert_eq!(gateway.with(|s| s.keys.len()), 1);

    // The callback with a session and no state: refused at the door.
    let callback = call_path(
        app.clone(),
        "GET",
        "/api-tokens/api/auth/callback?code=c",
        Some(&session),
    )
    .await;
    assert_eq!(callback.status, StatusCode::BAD_REQUEST);
    assert_eq!(discord.exchanges(), 0);
    assert_eq!(discord.member_calls(), 0);

    // `GET` on the pre-check path is not a route at all.
    let get = call_path(app, "GET", REWORK_PATH, Some(&session)).await;
    assert_eq!(get.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
}

#[tokio::test]
async fn the_pre_check_refuses_an_unauthenticated_caller_before_aws_is_touched() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = rework_app(&discord, &gateway);

    let reply = precheck(&app, None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.json()["code"], "not_signed_in");
    assert_eq!(gateway.with(|s| s.list_calls), 0);
}

#[tokio::test]
async fn the_pre_check_answers_no_key_when_there_is_nothing_to_replace() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = rework_app(&discord, &gateway);

    let reply = precheck(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json()["code"], "no_key");
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
}

/// A round-trip for a visitor with no key lands on `no_key` and creates
/// nothing — a rework is not an issue with weaker checks.
#[tokio::test]
async fn a_rework_with_no_key_lands_on_no_key_and_creates_nothing() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;

    let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=no_key");
    assert_eq!(gateway.with(|s| s.create_calls), 0);
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_some());
}

// ---------------------------------------------------------------------------
// Membership — re-proved; age — never
// ---------------------------------------------------------------------------

/// A user who has left the guild is refused on rework with the not-a-member
/// landing (the page names the server), nothing moves — and reveal still
/// works for them afterwards: the epic's non-goal holds.
#[tokio::test]
async fn a_user_who_has_left_the_guild_is_refused_on_rework_and_keeps_their_key() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::NotFound { code: 10_007 },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    let app = rework_app(&discord, &gateway);

    let reply = rework_round_trip(&app).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=not_member");
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_some());
    assert_eq!(gateway.with(|s| s.ops.len()), 0, "nothing moves");
    assert_eq!(discord.member_calls(), 1);

    let revealed = call_path(app, "GET", KEY_PATH, Some(&session_cookie(USER_ID))).await;
    assert_eq!(revealed.status, StatusCode::OK);
    assert_eq!(revealed.json()["key_id"], old);
}

/// A `429`/`5xx`/`401`/`403` from Discord refuses **without** claiming
/// non-membership — `unknown`, and nothing moves.
#[tokio::test]
async fn a_429_or_5xx_from_discord_refuses_the_rework_without_claiming_non_membership() {
    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
    ] {
        let discord = MockDiscord::start_with(
            GRANTED_SCOPE,
            None,
            MemberReply::Status(status),
            mock_discord::USER_ID,
        )
        .await;
        let gateway = MockGateway::start().await;
        seed_attached(&gateway, 1_000);

        let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
        assert_eq!(reply.location(), "/api-tokens/?rework=unknown", "{status}");
        assert_eq!(gateway.with(|s| s.ops.len()), 0, "{status}");
    }
}

/// `pending` absent is `unknown` on a rework exactly as on an issue — the
/// two paths share one membership table.
#[tokio::test]
async fn an_absent_pending_field_is_unknown_on_rework_too() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Member { pending: None },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);

    let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=unknown");
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
}

/// **Age is never re-checked on a rework.** An account created ten seconds
/// ago — which the issue round-trip would refuse as too young under a
/// five-minute threshold — reworks its key: an account old enough once is
/// old enough forever.
#[tokio::test]
async fn account_age_is_not_re_checked_on_a_rework() {
    const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let brand_new = ((now_ms - DISCORD_EPOCH_MS - 10_000) << 22).to_string();

    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Member {
            pending: Some(false),
        },
        &brand_new,
    )
    .await;
    let gateway = MockGateway::start().await;
    gateway.with(|s| {
        let id = s.seed(&format!("discord-{brand_new}-key"), 1_000);
        s.plan_keys.push((PLAN_ID.to_string(), id));
    });
    let app = rework_app_with(&discord, &gateway, GUILD_ID, "5");

    // The issue path, for contrast, refuses this account.
    assert!(
        issue_round_trip(&app)
            .await
            .location()
            .starts_with("/api-tokens/?issue=too_young")
    );
    // The rework path does not look.
    assert_eq!(
        rework_round_trip(&app).await.location(),
        "/api-tokens/?rework=ok"
    );
}

/// The membership call carries the FRESH token from this round-trip's own
/// exchange and names the configured guild — eligibility travels by
/// re-authentication on a rework exactly as on an issue.
#[tokio::test]
async fn the_member_call_carries_the_fresh_token_on_a_rework() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);

    rework_round_trip(&rework_app(&discord, &gateway)).await;

    assert_eq!(discord.exchanges(), 1);
    assert_eq!(discord.member_calls(), 1);
    assert_eq!(
        discord.member_bearer().as_deref(),
        Some("Bearer an-access-token")
    );
    assert_eq!(discord.member_guild().as_deref(), Some(GUILD_ID));
}

/// The re-authorisation is a redirect, not a login: `prompt=none` rides on
/// the rework's authorize URL as it does on the issue's.
#[tokio::test]
async fn the_rework_round_trip_suppresses_the_consent_screen() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = rework_app(&discord, &gateway);

    let login = call_path(app, "GET", "/api-tokens/api/auth/login?action=rework", None).await;
    let query = login.location().split_once('?').unwrap().1.to_string();
    let prompt = form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "prompt")
        .map(|(_, v)| v.to_string());
    assert_eq!(prompt.as_deref(), Some("none"));
}

// ---------------------------------------------------------------------------
// Discord ending the round-trip early
// ---------------------------------------------------------------------------

/// A narrow or wider grant lands on `?rework=denied` before any member call
/// — the registration drift the issue flow has a designed state for, and
/// the rework the same.
#[tokio::test]
async fn a_drifted_grant_lands_on_rework_denied_before_any_member_call() {
    for drifted in ["identify", "identify guilds.members.read guilds"] {
        let discord = MockDiscord::start(drifted, None).await;
        let gateway = MockGateway::start().await;
        seed_attached(&gateway, 1_000);

        let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{drifted}");
        assert_eq!(reply.location(), "/api-tokens/?rework=denied", "{drifted}");
        assert_eq!(discord.member_calls(), 0, "{drifted}");
        assert_eq!(gateway.with(|s| s.ops.len()), 0, "{drifted}");
    }
}

/// Cancelled at Discord's screen → `?rework=cancelled`; refused by Discord
/// for any other reason → `?rework=denied`. Neither is a verdict, and
/// neither lands on the issue's or sign-in's banners.
#[tokio::test]
async fn a_rework_ended_at_discord_lands_on_its_own_cancelled_or_denied_state() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = rework_app(&discord, &gateway);

    for (error, expected) in [
        ("access_denied", "/api-tokens/?rework=cancelled"),
        ("invalid_scope", "/api-tokens/?rework=denied"),
    ] {
        let login = call_path(
            app.clone(),
            "GET",
            "/api-tokens/api/auth/login?action=rework",
            None,
        )
        .await;
        let pending = login.cookie(cookies::PENDING_COOKIE).unwrap();
        let query = login.location().split_once('?').unwrap().1.to_string();
        let state = form_urlencoded::parse(query.as_bytes())
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .to_string();

        let reply = call_path(
            app.clone(),
            "GET",
            &format!("/api-tokens/api/auth/callback?error={error}&state={state}"),
            Some(&format!("{}={pending}", cookies::PENDING_COOKIE)),
        )
        .await;
        assert_eq!(reply.location(), expected, "{error}");
    }
    assert_eq!(discord.exchanges(), 0);
}

/// Discord failing the exchange or the identity read on a rework is
/// `unknown` — a landing, never the sign-in arm's `502` page.
#[tokio::test]
async fn a_discord_failure_mid_round_trip_lands_on_rework_unknown() {
    let gateway = MockGateway::start().await;
    seed_attached(&gateway, 1_000);

    let exchange_down =
        MockDiscord::start(GRANTED_SCOPE, Some(StatusCode::SERVICE_UNAVAILABLE)).await;
    let reply = rework_round_trip(&rework_app(&exchange_down, &gateway)).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api-tokens/?rework=unknown");

    let identity_down = MockDiscord::start_full(
        GRANTED_SCOPE,
        None,
        MemberReply::Member {
            pending: Some(false),
        },
        USER_ID,
        Some(StatusCode::TOO_MANY_REQUESTS),
    )
    .await;
    let reply = rework_round_trip(&rework_app(&identity_down, &gateway)).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=unknown");
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
}

/// An unwired deployment refuses to START a rework, with a landing rather
/// than a JSON page, and mints no pending cookie.
#[tokio::test]
async fn an_unwired_deployment_refuses_to_start_a_rework() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let unwired = build_app_with(
        true,
        None,
        Endpoints {
            api_base: discord.base.clone(),
            ..Endpoints::default()
        },
        None,
    );

    let reply = call_path(
        unwired,
        "GET",
        "/api-tokens/api/auth/login?action=rework",
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api-tokens/?rework=failed");
    assert!(reply.cookie(cookies::PENDING_COOKIE).is_none());
    assert_eq!(discord.exchanges(), 0);
}

// ---------------------------------------------------------------------------
// The control plane, after membership passed — every failure leaves the key
// ---------------------------------------------------------------------------

/// A control plane that is down after a passed check is `failed` — not
/// `unknown` — and the old key is untouched.
#[tokio::test]
async fn a_control_plane_failure_lands_on_failed_with_the_old_key_intact() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    gateway.with(|s| s.fail_list = true);

    let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=failed");
    assert_eq!(discord.member_calls(), 1, "membership ran and passed");
    assert_eq!(gateway.with(|s| s.keys[0].id.clone()), old);
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
}

/// A delete of the old key that fails rolls the replacement back: the
/// visitor is told it failed, and what they hold is exactly the key they
/// had — not two keys of which the page reveals the older.
#[tokio::test]
async fn a_failed_delete_of_the_old_key_rolls_the_replacement_back() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    gateway.with(|s| s.fail_delete_of = vec![old.clone()]);

    let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=failed");

    let stored = gateway.with(|s| s.keys.clone());
    assert_eq!(stored.len(), 1, "exactly the key they had");
    assert_eq!(stored[0].id, old);
    // The replacement was created, attached, and then deleted again.
    let ops = gateway.with(|s| s.ops.clone());
    let new = ops[0].trim_start_matches("create:").to_string();
    assert_eq!(
        ops,
        vec![
            format!("create:{new}"),
            format!("attach:{new}"),
            format!("delete:{new}")
        ]
    );
    assert_eq!(reveal(&gateway, USER_ID).await.json()["key_id"], old);
}

/// A replacement that vanishes before it can be attached is `failed` with
/// the old key untouched — the visitor never lost anything.
#[tokio::test]
async fn a_replacement_that_vanishes_before_the_attach_leaves_the_old_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    gateway.with(|s| s.vanish_on_next_attach = true);

    let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=failed");
    gateway.with(|s| {
        assert_eq!(s.keys.len(), 1);
        assert_eq!(s.keys[0].id, old);
        assert!(s.plan_keys.contains(&(PLAN_ID.to_string(), old.clone())));
    });
}

/// A usage plan that does not exist is `failed`, and the old key stays.
#[tokio::test]
async fn a_missing_usage_plan_is_failed_without_deleting_the_old_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    gateway.with(|s| s.attach_always_404 = true);

    let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=failed");
    assert!(gateway.with(|s| s.keys.iter().any(|k| k.id == old)));
    assert_eq!(gateway.with(|s| s.deleted.len()), 0);
}

/// A control plane slower than the deadline is `failed` — an answer, not a
/// Lambda-killed invocation — and the old key stays.
#[tokio::test]
async fn a_control_plane_slower_than_the_deadline_lands_on_failed() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    // The listing is held past the deadline the issue deps carry; the budget
    // arithmetic caps the swap at what is left of `ISSUE_BUDGET` (12s), so
    // hold for longer than that.
    gateway.with(|s| s.list_delay_ms = 13_000);

    let reply = rework_round_trip(&rework_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api-tokens/?rework=failed");
    assert_eq!(gateway.with(|s| s.keys[0].id.clone()), old);
    assert_eq!(gateway.with(|s| s.ops.len()), 0);
}

// ---------------------------------------------------------------------------
// The usage cache
// ---------------------------------------------------------------------------

/// A successful rework evicts the usage route's cached answer: the numbers
/// it held describe a key that no longer exists, and the new key starts from
/// a clean counter. Driven through the real router so the callback and the
/// usage route share one cache.
#[tokio::test]
async fn a_successful_rework_evicts_the_cached_usage_of_the_old_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let old = seed_attached(&gateway, 1_000);
    gateway.with(|s| {
        s.usage.insert(old.clone(), vec![vec![42, 99_958]]);
    });
    let app = rework_app(&discord, &gateway);
    let session = session_cookie(USER_ID);

    let before = call_path(app.clone(), "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(before.status, StatusCode::OK);
    assert_eq!(before.json()["used"], 42);
    assert_eq!(gateway.with(|s| s.usage_calls), 1);

    assert_eq!(
        rework_round_trip(&app).await.location(),
        "/api-tokens/?rework=ok"
    );

    // Well inside the 60s TTL, so only the eviction can explain a second
    // control-plane read — and its answer is the NEW key's: nothing recorded.
    let after = call_path(app, "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(after.status, StatusCode::OK);
    assert!(
        after.json()["used"].is_null(),
        "{}",
        String::from_utf8_lossy(&after.body)
    );
    assert_eq!(gateway.with(|s| s.usage_calls), 2);
    let new = gateway.with(|s| s.keys[0].id.clone());
    assert_eq!(
        gateway.with(|s| s.usage_queries.last().unwrap().0.clone()),
        new
    );
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// A session for somebody else does not survive the round-trip: the fresh
/// identity names the key that is replaced and the cookie now says so.
#[tokio::test]
async fn the_re_auth_identity_decides_whose_key_is_replaced() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let mine = seed_attached(&gateway, 1_000);
    let theirs = gateway.with(|s| s.seed("discord-999999999999999999-key", 1_000));
    let app = rework_app(&discord, &gateway);

    let login = call_path(
        app.clone(),
        "GET",
        "/api-tokens/api/auth/login?action=rework",
        None,
    )
    .await;
    let pending = login.cookie(cookies::PENDING_COOKIE).unwrap();
    let query = login.location().split_once('?').unwrap().1.to_string();
    let state = form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .to_string();
    let other = session_cookie("999999999999999999");
    let reply = call_path(
        app,
        "GET",
        &format!("/api-tokens/api/auth/callback?code=c&state={state}"),
        Some(&format!("{}={pending}; {other}", cookies::PENDING_COOKIE)),
    )
    .await;

    assert_eq!(reply.location(), "/api-tokens/?rework=ok");
    let cookie = reply.cookie(cookies::SESSION_COOKIE).unwrap();
    let session = prices_api::portal::auth::session::Session::decode(
        SIGNING_KEY.as_bytes(),
        &cookie,
        state_token::now_secs(),
    )
    .unwrap();
    assert_eq!(session.sub, USER_ID);
    assert_eq!(gateway.with(|s| s.deleted.clone()), vec![mine]);
    assert!(
        gateway.with(|s| s.keys.iter().any(|k| k.id == theirs)),
        "theirs is untouched"
    );
}

/// Every pre-check answer is uncacheable.
#[tokio::test]
async fn every_pre_check_answer_carries_no_store() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = rework_app(&discord, &gateway);

    let no_key = precheck(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(no_key.cache_control(), "no-store");
    let unauthenticated = precheck(&app, None).await;
    assert_eq!(unauthenticated.cache_control(), "no-store");

    seed_attached(&gateway, now_secs());
    let capped = precheck(&app, Some(&session_cookie(USER_ID))).await;
    assert_eq!(capped.status, StatusCode::CONFLICT);
    assert_eq!(capped.cache_control(), "no-store");
    assert!(capped.headers.get(header::SET_COOKIE).is_none());
}
