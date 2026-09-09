//! The eligibility-checked issue round-trip, over HTTP, end to end
//! (task 0189).
//!
//! Every test drives the real router through the real flow: `GET
//! /auth/login?action=issue` mints the signed state pair, the callback
//! exchanges the code against a **mock Discord**, asks it for the guild
//! membership, derives the account age from the snowflake, and — only when
//! all of that passes — runs 0187's reconciler against a **mock control
//! plane**. Both mocks record what they were asked, so the assertions are
//! about what the flow *did*: which token the membership call carried, which
//! guild it named, and above all how many keys were created (usually: zero).
//!
//! The spec's three-outcome rule is the spine of this file. Only Discord's own
//! 10007/10004 on a `404` is "not a member"; a throttle, an outage, an
//! unrecognised shape or an absent `pending` field refuses **without
//! accusation** (`?issue=unknown`); and a control-plane fault after a passed
//! check is `?issue=failed` — the visitor is fine, our key service was not.
//!
//! The create/attach/delete tests that lived in `tests/portal_keys.rs` under
//! 0187 live here now, driven through the callback, because the callback is
//! the only place a key is created since the route went read-only.

#[path = "portal_keys/harness.rs"]
mod harness;
#[path = "common/mock_discord.rs"]
mod mock_discord;

use axum::Router;
use axum::http::StatusCode;

use harness::*;
use mock_discord::{GRANTED_SCOPE, MemberReply, MockDiscord};
use prices_api::portal::auth::discord::Endpoints;
use prices_api::portal::auth::{cookies, session::Session, state_token};
use prices_api::portal::keys::gateway::Gateway;
use prices_api::portal::usage::USAGE_PATH;

/// Milliseconds since the Discord epoch, shifted into snowflake position —
/// how the too-young tests mint an account "created N seconds ago".
fn snowflake_created_secs_ago(secs: u64) -> String {
    const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    ((now_ms - DISCORD_EPOCH_MS - secs * 1_000) << 22).to_string()
}

fn issue_app(discord: &MockDiscord, gateway: &MockGateway) -> Router {
    issue_app_with(discord, gateway, GUILD_ID, "5")
}

fn issue_app_with(
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

// ---------------------------------------------------------------------------
// The gate — ships closed
// ---------------------------------------------------------------------------

/// The whole issue flow is an empty `404` while the portal is closed — the
/// login that would start it and the callback that would finish it — with
/// **zero** calls to Discord and zero to the control plane. This is the slice
/// 0194 is waiting on before `PORTAL_ENABLED` may flip, so "closed" is
/// asserted with everything else fully wired.
#[tokio::test]
async fn everything_including_issue_is_an_empty_404_while_the_portal_is_closed() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let closed = build_app_with(
        false,
        Some(Gateway::against(&gateway.base, PLAN_ID.to_string())),
        Endpoints {
            api_base: discord.base.clone(),
            ..Endpoints::default()
        },
        Some(eligibility(GUILD_ID, "5")),
    );

    for path in [
        "/api/auth/login?action=issue",
        "/api/auth/callback?code=c&state=s",
    ] {
        let reply = call_path(closed.clone(), "GET", path, None).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert!(reply.body.is_empty(), "{path} carried a body");
    }
    assert_eq!(discord.exchanges(), 0);
    assert_eq!(discord.member_calls(), 0);
    assert_eq!(gateway.with(|s| s.list_calls), 0);
    assert_eq!(gateway.with(|s| s.create_calls), 0);
}

// ---------------------------------------------------------------------------
// The happy path
// ---------------------------------------------------------------------------

/// A member in good standing, on an old account: the round-trip creates one
/// enabled, tagged key on the free plan, lands on `?issue=ok` with a fresh
/// session — and the key value appears **nowhere** in the redirect. The page
/// then reveals it over the read-only route.
#[tokio::test]
async fn a_member_in_good_standing_gets_a_key_and_lands_on_issue_ok() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = issue_app(&discord, &gateway);

    // Note the driver presents NO session cookie: the signed state pair plus
    // the fresh code are the authentication (ADR 0010 §8).
    let reply = issue_round_trip(&app).await;

    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api/?issue=ok");
    assert!(
        reply.cookie(cookies::SESSION_COOKIE).is_some(),
        "the round-trip proves identity, so it signs the visitor in"
    );

    let stored = gateway.with(|s| s.keys.clone());
    assert_eq!(stored.len(), 1, "exactly one key");
    assert_eq!(stored[0].name, key_name());
    assert!(
        stored[0].enabled,
        "a key that is not enabled cannot be used"
    );
    assert_eq!(
        stored[0].tags.get("ManagedBy").map(String::as_str),
        Some("prices-portal")
    );
    assert_eq!(
        gateway.with(|s| s.plan_keys.clone()),
        vec![(PLAN_ID.to_string(), stored[0].id.clone())],
        "the key is on the plan before anyone is told it exists"
    );

    // The credential never rides in the Location — or anywhere on the redirect.
    let headers = format!("{:?}", reply.headers);
    assert!(!headers.contains(&stored[0].value), "{headers}");
    assert!(!headers.contains("CANARY"), "{headers}");

    // The page's next step: reveal it, session-only, over the read-only route.
    let revealed = reveal(&gateway, USER_ID).await;
    assert_eq!(revealed.status, StatusCode::OK);
    assert_eq!(revealed.json()["key_id"], stored[0].id);
    assert_eq!(revealed.json()["value"], stored[0].value);
}

/// A second round-trip converges on the same key — issuance is idempotent
/// through the callback exactly as it was through 0187's button.
#[tokio::test]
async fn a_second_round_trip_returns_the_same_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = issue_app(&discord, &gateway);

    assert_eq!(issue_round_trip(&app).await.location(), "/api/?issue=ok");
    assert_eq!(issue_round_trip(&app).await.location(), "/api/?issue=ok");

    assert_eq!(
        gateway.with(|s| s.create_calls),
        1,
        "one create, however many trips"
    );
    assert_eq!(gateway.with(|s| s.keys.len()), 1);
}

/// Two people get two keys, and neither is handed the other's.
#[tokio::test]
async fn two_users_get_two_different_keys() {
    let gateway = MockGateway::start().await;
    let mine = MockDiscord::start(GRANTED_SCOPE, None).await;
    let theirs = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Member {
            pending: Some(false),
        },
        "111111111111111111",
    )
    .await;

    assert_eq!(
        issue_round_trip(&issue_app(&mine, &gateway))
            .await
            .location(),
        "/api/?issue=ok"
    );
    assert_eq!(
        issue_round_trip(&issue_app(&theirs, &gateway))
            .await
            .location(),
        "/api/?issue=ok"
    );

    let names: Vec<String> = gateway.with(|s| s.keys.iter().map(|k| k.name.clone()).collect());
    assert_eq!(names.len(), 2);
    assert!(names.contains(&key_name()));
    assert!(names.contains(&"discord-111111111111111111-key".to_string()));
}

/// However two simultaneous round-trips interleave, the user ends with exactly
/// one key — the reconciler is what gets them there.
#[tokio::test]
async fn two_simultaneous_round_trips_leave_exactly_one_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = issue_app(&discord, &gateway);

    let (a, b) = tokio::join!(issue_round_trip(&app), issue_round_trip(&app));
    assert_eq!(a.location(), "/api/?issue=ok");
    assert_eq!(b.location(), "/api/?issue=ok");

    assert_eq!(
        gateway.with(|s| s.named(&key_name()).len()),
        1,
        "the reconciler must converge on one key"
    );

    // And the survivor is what a subsequent reveal hands out, so whichever of
    // the two lost, the user is not left holding a deleted value.
    let survivor = gateway.with(|s| s.named(&key_name())[0].id.clone());
    assert_eq!(reveal(&gateway, USER_ID).await.json()["key_id"], survivor);
}

// ---------------------------------------------------------------------------
// The membership verdicts — three outcomes, not two
// ---------------------------------------------------------------------------

/// A confirmed non-member (Discord's own 10007 on a 404) is refused, no key is
/// created — and identity was still proven, so the refusal carries a session:
/// a non-member legitimately holds reveal and usage for any key they already
/// have.
#[tokio::test]
async fn a_non_member_is_refused_and_no_key_is_created() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::NotFound { code: 10_007 },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api/?issue=not_member");
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_some());

    assert_eq!(
        gateway.with(|s| s.create_calls),
        0,
        "no key for a non-member"
    );
    assert_eq!(
        gateway.with(|s| s.list_calls),
        0,
        "the control plane is not even asked"
    );
}

/// 10004 ("Unknown Guild") also reads as not-a-member per the spec — the
/// warn naming the guild id (a mis-seeded parameter is the likelier cause) is
/// asserted at the unit level; here the wire outcome is what is pinned.
#[tokio::test]
async fn an_unknown_guild_code_is_also_refused_as_not_a_member() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::NotFound { code: 10_004 },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=not_member");
    assert_eq!(gateway.with(|s| s.create_calls), 0);
}

/// A 404 whose body does not carry a recognised code proves nothing — an
/// unmeasured shape (0180 item 1 is still open) must not become an accusation.
#[tokio::test]
async fn a_404_with_an_unrecognised_code_is_unknown_not_an_accusation() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::NotFound { code: 10_008 },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=unknown");
    assert_eq!(gateway.with(|s| s.create_calls), 0);
}

/// The acceptance criterion verbatim: a `429` or `5xx` from Discord refuses
/// **without** claiming non-membership. `401`/`403` land the same way — an
/// auth fault on our side is not the visitor's standing either.
#[tokio::test]
async fn a_429_or_5xx_from_discord_refuses_without_claiming_non_membership() {
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

        let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
        assert_eq!(
            reply.location(),
            "/api/?issue=unknown",
            "a {status} must be 'could not verify', never 'not a member'"
        );
        assert_eq!(gateway.with(|s| s.create_calls), 0, "{status}");
    }
}

/// `pending: true` — joined, but not through Membership Screening — is
/// refused without a key, and lands on ITS OWN state (task 0254; this test
/// asserted `not_member` before). The remedy is "accept the rules", not
/// "join", and the page needs the literal to say which.
#[tokio::test]
async fn a_pending_member_is_refused_as_pending_rules() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Member {
            pending: Some(true),
        },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=pending_rules");
    assert_eq!(gateway.with(|s| s.create_calls), 0);
}

/// The acceptance criterion: `pending === undefined` is handled explicitly
/// and does not silently pass. It is `unknown` — refused without accusation —
/// because 0180 item 2 (whether the REST route carries the field at all) is
/// unmeasured, and reading absence as "cleared" would void the gate.
#[tokio::test]
async fn an_absent_pending_field_does_not_silently_pass() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Member { pending: None },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=unknown");
    assert_eq!(gateway.with(|s| s.create_calls), 0);
}

/// A 200 whose body is not a member object proves nothing either.
#[tokio::test]
async fn a_malformed_member_body_is_unknown() {
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Malformed,
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=unknown");
    assert_eq!(gateway.with(|s| s.create_calls), 0);
}

/// The membership call carries the FRESH token from this round-trip's own
/// exchange — that is what "eligibility travels by re-authentication" means on
/// the wire — and asks about exactly the configured guild.
#[tokio::test]
async fn the_member_call_carries_the_fresh_token_and_the_configured_guild() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;

    issue_round_trip(&issue_app(&discord, &gateway)).await;

    assert_eq!(discord.member_calls(), 1);
    assert_eq!(
        discord.member_bearer().as_deref(),
        Some("Bearer an-access-token"),
        "the member call must authenticate with the just-exchanged token"
    );
    assert_eq!(discord.member_guild().as_deref(), Some(GUILD_ID));
}

/// A grant missing the member scope is refused at the exchange, before any
/// membership call — a registration that drifted narrower must not turn every
/// eligibility check into a refusal with a misleading shape.
///
/// **And it lands, rather than answering `502`.** This is the likeliest fault
/// on this path — the Developer Portal registration still carrying 0186's
/// `identify` alone — and the visitor pressed "get my API key" from a page
/// they were looking at. `?issue=denied` is where the callback already puts
/// Discord's own `invalid_scope`, which is the same fault noticed one step
/// earlier: one registration drift, one landing, whichever half sees it.
/// "Try again shortly" (`unknown`) would be a lie; no wait fixes a
/// registration.
#[tokio::test]
async fn a_grant_missing_the_member_scope_lands_on_denied_before_any_member_call() {
    for drifted in ["identify", "identify guilds.members.read guilds"] {
        let discord = MockDiscord::start(drifted, None).await;
        let gateway = MockGateway::start().await;

        let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "scope={drifted}");
        assert_eq!(reply.location(), "/api/?issue=denied", "{drifted}");
        // Not the sign-in arm's error page: nothing renderable, no way back.
        assert!(reply.body.is_empty(), "{drifted}");
        assert_eq!(discord.member_calls(), 0, "{drifted}");
        assert_eq!(gateway.with(|s| s.create_calls), 0, "{drifted}");
    }
}

/// Discord failing the **token exchange** on an issue round-trip is
/// "we could not verify", not the sign-in arm's `502 discord_unavailable`.
///
/// The visitor is mid-navigation from their own dashboard: a JSON envelope
/// worded "could not complete sign-in" is a dead end with no link back, about
/// an action they did not take. It also must not leak Discord's body.
#[tokio::test]
async fn a_failed_token_exchange_on_an_issue_lands_on_unknown() {
    let discord = MockDiscord::start(GRANTED_SCOPE, Some(StatusCode::SERVICE_UNAVAILABLE)).await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api/?issue=unknown");
    assert!(!String::from_utf8_lossy(&reply.body).contains("upstream said no"));
    assert_eq!(discord.member_calls(), 0);
    assert_eq!(gateway.with(|s| s.create_calls), 0);
    // Refused before identity was proven, so no session is minted — and the
    // one the visitor arrived with is left exactly as it was.
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_none());
}

/// The same rule for the **identity read**, the last Discord call on the
/// path: it happens after the membership check, so this is a round-trip that
/// got all the way to "who is this?" and lost Discord there.
#[tokio::test]
async fn a_failed_identity_read_on_an_issue_lands_on_unknown() {
    let discord = MockDiscord::start_full(
        GRANTED_SCOPE,
        None,
        MemberReply::Member {
            pending: Some(false),
        },
        USER_ID,
        Some(StatusCode::TOO_MANY_REQUESTS),
    )
    .await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/api/?issue=unknown");
    assert_eq!(discord.member_calls(), 1, "the membership call did happen");
    // No identity means no key: the reconciler needs a `sub` to name one.
    assert_eq!(gateway.with(|s| s.create_calls), 0);
}

/// `prompt=none` is on the issue round-trip and **only** there.
///
/// The re-authorisation is the one that has to be a redirect rather than a
/// login — eligibility is proved per action, so the visitor crosses the
/// authorize endpoint again for every issue, every retry after a refusal and
/// (0191) every rework. Sign-in is the opposite case: it is where the first
/// authorisation happens, and what Discord does with `prompt=none` for an app
/// that has not been authorised (or was authorised under 0186's narrower
/// scope) is undocumented.
#[tokio::test]
async fn only_the_issue_round_trip_suppresses_the_consent_screen() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = issue_app(&discord, &gateway);

    let prompt_of = |location: &str| -> Option<String> {
        let query = location.split_once('?').expect("a query").1.to_string();
        form_urlencoded::parse(query.as_bytes())
            .find(|(k, _)| k == "prompt")
            .map(|(_, v)| v.to_string())
    };

    let issue = call_path(app.clone(), "GET", "/api/auth/login?action=issue", None).await;
    assert_eq!(prompt_of(&issue.location()).as_deref(), Some("none"));

    let signin = call_path(app, "GET", "/api/auth/login", None).await;
    assert_eq!(prompt_of(&signin.location()), None);
}

// ---------------------------------------------------------------------------
// Account age
// ---------------------------------------------------------------------------

/// The acceptance criterion: an account below the threshold is refused **with
/// the time remaining** — carried as `wait_secs`, digits the page renders, so
/// the copy follows the operator's threshold instead of hard-coding one.
#[tokio::test]
async fn an_account_below_the_threshold_is_refused_with_the_time_remaining() {
    // Created two minutes ago, against a five-minute threshold.
    let young = snowflake_created_secs_ago(120);
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Member {
            pending: Some(false),
        },
        &young,
    )
    .await;
    let gateway = MockGateway::start().await;

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    let location = reply.location();
    let (base, wait) = location
        .split_once("&wait_secs=")
        .unwrap_or_else(|| panic!("no wait_secs in {location}"));
    assert_eq!(base, "/api/?issue=too_young");
    let wait: u64 = wait.parse().expect("wait_secs must be digits");
    // ~180s remain; generous bounds absorb test-runner latency.
    assert!((150..=181).contains(&wait), "wait_secs={wait}");
    assert_eq!(gateway.with(|s| s.create_calls), 0);
    // Still signed in — a wait is not a rejection.
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_some());
}

/// The threshold is configuration, not code: the same two-minute-old account
/// is refused under a five-minute rule and issued under a zero-minute rule.
/// (Live, the value is re-read from SSM per action — changing it needs no
/// redeploy; here the two sources stand in for the two values.)
#[tokio::test]
async fn the_threshold_value_decides_the_verdict_at_action_time() {
    let young = snowflake_created_secs_ago(120);

    let strict_discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Member {
            pending: Some(false),
        },
        &young,
    )
    .await;
    let gateway = MockGateway::start().await;
    let strict = issue_round_trip(&issue_app_with(&strict_discord, &gateway, GUILD_ID, "5")).await;
    assert!(
        strict.location().starts_with("/api/?issue=too_young"),
        "{}",
        strict.location()
    );

    let lax_discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::Member {
            pending: Some(false),
        },
        &young,
    )
    .await;
    let lax = issue_round_trip(&issue_app_with(&lax_discord, &gateway, GUILD_ID, "0")).await;
    assert_eq!(lax.location(), "/api/?issue=ok");
}

// ---------------------------------------------------------------------------
// Sessions and identities
// ---------------------------------------------------------------------------

/// An existing session for a DIFFERENT account does not survive the
/// round-trip: the fresh re-auth identity wins, the key is named for it, and
/// the cookie now says so — key and session can never disagree.
#[tokio::test]
async fn a_session_for_someone_else_is_replaced_by_the_re_auth_identity() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let app = issue_app(&discord, &gateway);

    // Start the round-trip, then complete the callback while presenting a
    // session for somebody else alongside the pending cookie.
    let login = call_path(app.clone(), "GET", "/api/auth/login?action=issue", None).await;
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
        &format!("/api/auth/callback?code=c&state={state}"),
        Some(&format!("{}={pending}; {other}", cookies::PENDING_COOKIE)),
    )
    .await;

    assert_eq!(reply.location(), "/api/?issue=ok");
    let cookie = reply.cookie(cookies::SESSION_COOKIE).unwrap();
    let session = Session::decode(SIGNING_KEY.as_bytes(), &cookie, state_token::now_secs())
        .expect("the fresh session must verify");
    assert_eq!(session.sub, USER_ID, "the re-auth identity wins");

    let names: Vec<String> = gateway.with(|s| s.keys.iter().map(|k| k.name.clone()).collect());
    assert_eq!(
        names,
        vec![key_name()],
        "the key belongs to the fresh identity too"
    );
}

/// A successful issue evicts the usage route's cached "no key" — the half of
/// 0188's R2/C2 fix that moved here when issuance moved to the callback.
///
/// The dashboard asks for usage on mount, so a visitor with no key has a
/// `NoKey` cached for the next 60 seconds *before* they press "get my API
/// key". Without the eviction, the page they land on after `?issue=ok` — and
/// every reload for the rest of that TTL — tells a key-holder they have no
/// key, which is exactly the falsehood 0188's R2 was filed for.
///
/// Asserted here because it was asserted **nowhere** otherwise: 0188's
/// `issuing_a_key_evicts_a_cached_no_key` became a reveal-path test when this
/// slice made `/key` read-only, and deleting `invalidate_no_key` from
/// `complete_issue` left the whole workspace green.
#[tokio::test]
async fn a_successful_issue_evicts_the_cached_no_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    // One app, so the issue callback and the usage route share the cache the
    // way `portal::apply` wires them in production.
    let app = issue_app(&discord, &gateway);
    let session = session_cookie(USER_ID);

    // The dashboard's mount-time read, before the key exists: caches `NoKey`.
    let before = call_path(app.clone(), "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(before.status, StatusCode::NOT_FOUND);
    assert_eq!(before.json()["code"], "no_key");

    let issued = issue_round_trip(&app).await;
    assert_eq!(issued.location(), "/api/?issue=ok");

    // Well inside the 60s TTL, so only the eviction can explain this.
    let after = call_path(app, "GET", USAGE_PATH, Some(&session)).await;
    assert_eq!(
        after.status,
        StatusCode::OK,
        "a cached `no_key` survived the issue that falsified it: {}",
        String::from_utf8_lossy(&after.body)
    );
}

/// The epic's non-goal, as a test: a user who has left the guild keeps their
/// key — reveal and usage still work with the session alone, and **Discord is
/// never consulted** on either route.
#[tokio::test]
async fn reveal_and_usage_still_work_for_a_user_who_has_left_the_guild() {
    // Discord now says "not a member" — they left after issuance.
    let discord = MockDiscord::start_with(
        GRANTED_SCOPE,
        None,
        MemberReply::NotFound { code: 10_007 },
        mock_discord::USER_ID,
    )
    .await;
    let gateway = MockGateway::start().await;
    gateway.with(|s| {
        let id = s.seed(&key_name(), 1_000);
        s.plan_keys.push((PLAN_ID.to_string(), id));
    });
    let app = issue_app(&discord, &gateway);

    let revealed = call_path(
        app.clone(),
        "GET",
        prices_api::portal::keys::KEY_PATH,
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(revealed.status, StatusCode::OK);
    assert_eq!(revealed.json()["name"], key_name());

    let usage = call_path(app, "GET", USAGE_PATH, Some(&session_cookie(USER_ID))).await;
    assert_eq!(
        usage.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&usage.body)
    );

    assert_eq!(
        discord.member_calls(),
        0,
        "membership is proved at issuance and never re-checked by reveal or usage"
    );
    assert_eq!(discord.exchanges(), 0);
}

// ---------------------------------------------------------------------------
// Configuration faults
// ---------------------------------------------------------------------------

/// An eligibility parameter that cannot be resolved — empty guild id, or a
/// threshold that is not a number — refuses as `unknown`: the fault is ours,
/// the log names it, and the visitor is told to try again rather than
/// anything about their membership. The member call never happens.
#[tokio::test]
async fn an_unreadable_eligibility_parameter_is_unknown() {
    for (guild, age) in [("", "5"), (GUILD_ID, "five"), ("not-a-snowflake", "5")] {
        let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
        let gateway = MockGateway::start().await;
        let app = issue_app_with(&discord, &gateway, guild, age);

        let reply = issue_round_trip(&app).await;
        assert_eq!(
            reply.location(),
            "/api/?issue=unknown",
            "guild={guild:?} age={age:?}"
        );
        assert_eq!(gateway.with(|s| s.create_calls), 0);
        // An empty or unparseable parameter never becomes a member URL; the
        // non-snowflake one is refused by the URL builder itself.
        if guild.is_empty() || age == "five" {
            assert_eq!(discord.member_calls(), 0, "guild={guild:?} age={age:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// The control plane, after eligibility passed
// ---------------------------------------------------------------------------

/// A control-plane failure after a PASSED check is `?issue=failed`, not
/// `unknown` — the visitor's membership was verified, and rendering an AWS
/// incident as a doubt about it would be both false and unfixable by them.
#[tokio::test]
async fn a_control_plane_failure_after_eligibility_lands_on_issue_failed_not_unknown() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    gateway.with(|s| s.fail_list = true);

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=failed");
    assert_eq!(discord.member_calls(), 1, "eligibility ran and passed");
    // Still signed in: identity and membership both proved.
    assert!(reply.cookie(cookies::SESSION_COOKIE).is_some());
}

/// A usage plan that does not exist is `failed` too — and the flow must not
/// keep minting keys while it is broken.
#[tokio::test]
async fn a_missing_usage_plan_is_issue_failed_without_minting_more_keys() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    gateway.with(|s| s.attach_always_404 = true);

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=failed");
    assert_eq!(gateway.with(|s| s.create_calls), 1);
}

/// A stale listing whose winner never reads bounds the attempts and lands on
/// `failed` — an answer, not a timeout.
#[tokio::test]
async fn a_stale_listing_bounds_the_attempts_and_lands_on_failed() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    gateway.with(|s| {
        s.seed(&key_name(), 1_000);
        s.read_always_404 = true;
    });

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=failed");
    assert_eq!(
        gateway.with(|s| s.list_calls),
        2,
        "one listing per attempt, and exactly two attempts"
    );
    assert_eq!(gateway.with(|s| s.create_calls), 0);
}

// ---------------------------------------------------------------------------
// The reconciler, driven through the callback (moved from portal_keys.rs)
// ---------------------------------------------------------------------------

/// A key that exists but is on no usage plan is adopted and put on it — the
/// heal for the orphan the read-only reveal deliberately leaves alone.
#[tokio::test]
async fn an_adopted_key_is_put_on_the_free_plan() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let orphan = gateway.with(|s| s.seed(&key_name(), 1_000));

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=ok");
    assert_eq!(
        gateway.with(|s| s.plan_keys.clone()),
        vec![(PLAN_ID.to_string(), orphan.clone())],
        "an adopted key is only useful once it is on the plan"
    );
    assert_eq!(
        gateway.with(|s| s.create_calls),
        0,
        "adopting must not mint a second key"
    );
    assert_eq!(reveal(&gateway, USER_ID).await.json()["key_id"], orphan);
}

/// Duplicates converge on the **earliest** key and the rest are deleted —
/// through the round-trip, which is now the only path that may delete.
#[tokio::test]
async fn duplicates_converge_on_the_earliest_and_the_losers_are_deleted() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let (early, late, later) = gateway.with(|s| {
        (
            s.seed(&key_name(), 1_000),
            s.seed(&key_name(), 2_000),
            s.seed(&key_name(), 3_000),
        )
    });

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=ok");

    let mut deleted = gateway.with(|s| s.deleted.clone());
    deleted.sort();
    let mut expected = vec![late, later];
    expected.sort();
    assert_eq!(deleted, expected);
    assert_eq!(gateway.with(|s| s.named(&key_name()).len()), 1);
    assert_eq!(
        reveal(&gateway, USER_ID).await.json()["key_id"],
        early,
        "the earliest key must survive"
    );
}

/// The reconciler must see **every** page before it ranks — five duplicates
/// at a page size of two, with the earliest deliberately on the last page.
#[tokio::test]
async fn the_reconciler_pages_get_api_keys_to_exhaustion_before_ranking() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let earliest = gateway.with(|s| {
        s.page_size = 2;
        s.seed(&key_name(), 5_000);
        s.seed(&key_name(), 4_000);
        s.seed(&key_name(), 3_000);
        s.seed(&key_name(), 2_000);
        s.seed(&key_name(), 1_000) // earliest, and last in the listing
    });

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=ok");
    assert_eq!(
        gateway.with(|s| s.list_calls),
        3,
        "five keys at a page size of two is three pages"
    );
    assert_eq!(gateway.with(|s| s.deleted.len()), 4);
    assert_eq!(
        gateway.with(|s| s.named(&key_name())[0].id.clone()),
        earliest,
        "the winner came from the last page"
    );
}

/// A key that vanishes before the attach re-enters the flow rather than
/// becoming a dead end.
#[tokio::test]
async fn a_key_that_vanishes_before_the_attach_is_not_a_dead_end() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let doomed = gateway.with(|s| {
        let id = s.seed(&key_name(), 1_000);
        s.vanish_on_next_attach = true;
        id
    });

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=ok");

    let survivors: Vec<String> =
        gateway.with(|s| s.named(&key_name()).iter().map(|k| k.id.clone()).collect());
    assert_eq!(survivors.len(), 1);
    assert_ne!(
        survivors[0], doomed,
        "the vanished key is not what survived"
    );
    assert_eq!(
        gateway.with(|s| s.plan_keys.len()),
        1,
        "the replacement is on the plan"
    );
}

/// An eventually-consistent listing that has not caught up must not orphan
/// the key just created.
#[tokio::test]
async fn a_key_the_listing_has_not_caught_up_with_is_not_orphaned() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let older = gateway.with(|s| {
        let older = s.seed(&key_name(), 1_000);
        s.next_list_is_empty = true;
        s.next_list_omits_newest = true;
        older
    });

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=ok");
    assert_eq!(
        gateway.with(|s| s.keys.len()),
        1,
        "the key created during this request was reconciled away, not orphaned"
    );
    assert_eq!(
        gateway.with(|s| s.plan_keys.clone()),
        vec![(PLAN_ID.to_string(), older)],
        "and the survivor is on the plan"
    );
}

/// A duplicate that will not delete does not withhold the outcome.
#[tokio::test]
async fn a_duplicate_that_will_not_delete_does_not_withhold_the_key() {
    let discord = MockDiscord::start(GRANTED_SCOPE, None).await;
    let gateway = MockGateway::start().await;
    let earliest = gateway.with(|s| {
        let earliest = s.seed(&key_name(), 1_000);
        s.seed(&key_name(), 2_000);
        s.fail_deletes = true;
        earliest
    });

    let reply = issue_round_trip(&issue_app(&discord, &gateway)).await;
    assert_eq!(reply.location(), "/api/?issue=ok");
    assert_eq!(
        gateway.with(|s| s.keys.len()),
        2,
        "the duplicate survives, for the next round-trip to try again"
    );
    assert_eq!(gateway.with(|s| s.create_calls), 0);
    assert_eq!(
        reveal(&gateway, USER_ID).await.json()["key_id"],
        earliest,
        "the winner is still the earliest, and it is what the reveal hands out"
    );
}
