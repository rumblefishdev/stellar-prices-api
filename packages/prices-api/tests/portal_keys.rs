//! Revealing a key, over HTTP, against a mock control plane (task 0187,
//! re-shaped read-only by task 0189).
//!
//! The unit tests next to `portal/keys/naming.rs` cover the rules — the name,
//! the exact filter, the winner — on lists somebody typed by hand. This file
//! covers what none of them can: the route wired into the real
//! [`prices_api::app`] router, driving the **actual AWS SDK** against a mock
//! API Gateway control plane bound to loopback. The mock, and the argument for
//! why it is a service rather than a trait, live in `portal_keys/harness.rs`.
//!
//! **The create path is not here.** Task 0189 moved issuance behind the
//! eligibility-checked OAuth round-trip, so everything that creates, attaches
//! or deletes is exercised in `tests/portal_issue.rs`, driven through the
//! callback. What this file owns is the inverse property: a session cookie
//! alone can cause **zero control-plane writes**, on either verb, in every
//! state the store can be in.
//!
//! The one assertion that is **not** in either file is "no key value reaches
//! the logs": it needs a process to itself and lives in `portal_keys_logs.rs`.

#[path = "portal_keys/harness.rs"]
mod harness;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::json;
use tower::ServiceExt;

use harness::*;

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// "Closed" is asserted on both verbs, with a valid session presented, and
/// byte-for-byte against a path that was never routed.
#[tokio::test]
async fn both_key_routes_are_an_empty_404_while_the_portal_is_closed() {
    for method in ["GET", "POST"] {
        let closed = call(
            build_app(false, None),
            method,
            Some(&session_cookie(USER_ID)),
        )
        .await;
        assert_eq!(closed.status, StatusCode::NOT_FOUND, "{method}");
        assert!(closed.body.is_empty(), "{method}: body must be empty");
        assert!(
            closed.headers.get(header::SET_COOKIE).is_none(),
            "{method}: a closed portal must set no cookies"
        );
    }

    // The comparison that makes it a gate rather than a 404 page: an unrouted
    // path under the same prefix answers identically.
    let nowhere = build_app(false, None)
        .oneshot(
            Request::builder()
                .uri("/api-tokens/api/no-such-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(nowhere.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        axum::body::to_bytes(nowhere.into_body(), usize::MAX)
            .await
            .unwrap()
            .len(),
        0
    );
}

/// With the portal open and nothing provisioned the route still exists and
/// says so — a `503`, not a `404`, so a half-configured deployment is
/// distinguishable from a closed one.
#[tokio::test]
async fn an_unprovisioned_deployment_answers_503_rather_than_vanishing() {
    let reply = call(
        build_app(true, None),
        "POST",
        Some(&session_cookie(USER_ID)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.json()["code"], "keys_unconfigured");
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

/// No session, no answer — and, just as importantly, **no call to AWS**. A
/// route that reached the control plane before checking the cookie would let
/// an anonymous caller drive `GetApiKeys` at the portal's 10 req/s throttle.
#[tokio::test]
async fn asking_without_a_session_is_refused_before_aws_is_touched() {
    let mock = MockGateway::start().await;
    for method in ["GET", "POST"] {
        let reply = call(app_against(&mock), method, None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method}");
        assert_eq!(reply.json()["code"], "not_signed_in", "{method}");
    }
    assert_eq!(mock.with(|s| s.list_calls), 0);
    assert_eq!(mock.with(|s| s.create_calls), 0);
}

/// The forgery that matters: edit `sub` and you are someone else — and from
/// this slice on, someone else's API key. The signature is what stops it, and
/// nothing reaches AWS on the way to finding that out.
#[tokio::test]
async fn a_forged_session_cannot_reveal_a_key() {
    use base64::Engine as _;

    let mock = MockGateway::start().await;
    let genuine = session_cookie(USER_ID);
    let (name, value) = genuine.split_once('=').unwrap();
    let (payload, mac) = value.split_once('.').unwrap();
    let forged_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({ "sub": "1", "name": "mallory", "exp": now_secs() + 3600 }).to_string());
    assert_ne!(forged_payload, payload);

    let reply = call(
        app_against(&mock),
        "POST",
        Some(&format!("{name}={forged_payload}.{mac}")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(mock.with(|s| s.create_calls), 0);
    assert_eq!(mock.with(|s| s.list_calls), 0);
}

// ---------------------------------------------------------------------------
// The reveal — and the read-only invariant (task 0189)
// ---------------------------------------------------------------------------

/// The reveal shows an existing key: id, name, value.
#[tokio::test]
async fn an_existing_key_is_revealed_with_its_value() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let id = mock.with(|s| s.seed(&name, 1_000));

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["key_id"], id);
    assert_eq!(body["name"], name);
    assert_eq!(body["value"], mock.with(|s| s.keys[0].value.clone()));
}

/// **The acceptance criterion "issue is unreachable with a session cookie
/// alone", verified by calling it directly with nothing else.** Both verbs,
/// empty store — the state 0187's handler would have created in — and the
/// answer is `no_key`, with zero writes of any kind on the control plane.
#[tokio::test]
async fn a_session_cookie_alone_cannot_create_a_key_on_either_verb() {
    let mock = MockGateway::start().await;
    for method in ["GET", "POST"] {
        let reply = call(app_against(&mock), method, Some(&session_cookie(USER_ID))).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{method}");
        assert_eq!(reply.json()["code"], "no_key", "{method}");
        assert_eq!(reply.cache_control(), "no-store", "{method}");
    }
    assert_eq!(mock.with(|s| s.create_calls), 0, "nothing was created");
    assert_eq!(mock.with(|s| s.attach_calls), 0, "nothing was attached");
    assert!(mock.with(|s| s.deleted.is_empty()), "nothing was deleted");
}

/// Around duplicates too: the reveal answers with the same deterministic
/// winner the issue flow would pick, and sweeps **nothing** — converging
/// duplicates stays the issue flow's job, because a delete reachable from a
/// session-only route would be a control-plane write a forged top-level
/// navigation could trigger.
#[tokio::test]
async fn a_reveal_never_creates_attaches_or_deletes_even_around_duplicates() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let earliest = mock.with(|s| {
        let earliest = s.seed(&name, 1_000);
        s.seed(&name, 2_000);
        s.seed(&name, 3_000);
        earliest
    });

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json()["key_id"],
        earliest,
        "the reveal ranks like the issue flow"
    );

    assert_eq!(
        mock.with(|s| s.keys.len()),
        3,
        "the duplicates survive a reveal"
    );
    assert!(mock.with(|s| s.deleted.is_empty()));
    assert_eq!(mock.with(|s| s.create_calls), 0);
    assert_eq!(mock.with(|s| s.attach_calls), 0);
}

/// A key that exists but is on no plan is revealed as it is — NOT silently
/// attached, because attach is a write. It answers `403` on `/v1/` until the
/// holder presses "get my key", whose round-trip adopts and attaches it
/// (asserted in `portal_issue.rs`).
#[tokio::test]
async fn an_unattached_key_is_revealed_but_not_repaired_in_place() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let orphan = mock.with(|s| s.seed(&name, 1_000));

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["key_id"], orphan);
    assert_eq!(mock.with(|s| s.attach_calls), 0);
    assert!(mock.with(|s| s.plan_keys.is_empty()));
}

/// A key deleted by hand in the console answers `no_key` — it is **not**
/// resurrected by a reveal, which is the deliberate reversal of 0187's
/// behaviour: recreating a key is issuance, and issuance now lives behind the
/// eligibility round-trip. The heal is one press away, and behind the gate.
#[tokio::test]
async fn a_key_deleted_by_hand_answers_no_key_rather_than_recreating() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| s.seed(&name, 1_000));
    assert_eq!(reveal(&mock, USER_ID).await.status, StatusCode::OK);

    // Somebody opens the console and deletes it.
    mock.with(|s| s.keys.clear());

    let after = reveal(&mock, USER_ID).await;
    assert_eq!(after.status, StatusCode::NOT_FOUND);
    assert_eq!(after.json()["code"], "no_key");
    assert_eq!(
        mock.with(|s| s.create_calls),
        0,
        "the reveal must not recreate"
    );
}

/// The narrower race: the key is listed, and gone by the time its value is
/// read. `no_key`, not an error and not a retry-into-create — the observation
/// is the same as "deleted by hand", and so is the honest answer.
#[tokio::test]
async fn a_key_that_vanishes_between_the_list_and_the_read_answers_no_key() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| {
        s.seed(&name, 1_000);
        s.vanish_on_next_read = true;
    });

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json()["code"], "no_key");
    assert_eq!(mock.with(|s| s.create_calls), 0);
}

/// A listing that keeps returning a key the read will not produce — a stale
/// lister and a reader that disagree. One pass, `no_key`, done: 0187's
/// bounded retry existed to give the CREATE path a second chance, and with no
/// create there is nothing a second identical read would learn.
#[tokio::test]
async fn a_stale_listing_whose_keys_never_read_answers_no_key_in_one_pass() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| {
        s.seed(&name, 1_000);
        s.read_always_404 = true;
    });

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json()["code"], "no_key");
    assert_eq!(
        mock.with(|s| s.list_calls),
        1,
        "a lookup is one pass, not a retry loop"
    );
    assert_eq!(mock.with(|s| s.create_calls), 0);
}

/// Signing out and back in is a new cookie for the same Discord id, and the
/// key is a property of the id — so it survives. This is the criterion that
/// would fail if anything about the key were kept in the session.
#[tokio::test]
async fn signing_out_and_back_in_still_shows_the_same_key() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| s.seed(&name, 1_000));

    // Two independently minted cookies, as two sign-ins would mint them.
    let before = reveal(&mock, USER_ID).await.json();
    let after = reveal(&mock, USER_ID).await.json();
    assert_eq!(before["key_id"], after["key_id"]);
    assert_eq!(before["value"], after["value"]);
}

// ---------------------------------------------------------------------------
// The prefix hazard
// ---------------------------------------------------------------------------

/// A shorter Discord id is a prefix of a longer one, and `nameQuery` is a
/// prefix match. The `-key` suffix plus the exact filter are what stop user
/// `123…` from seeing the key of user `123…9` — and a read-only route still
/// must not *reveal* across the boundary, even though it can no longer delete.
#[tokio::test]
async fn a_user_whose_id_prefixes_another_cannot_see_their_key() {
    let mock = MockGateway::start().await;
    let victim_name = "discord-1234567890123456789-key";
    let victim = mock.with(|s| s.seed(victim_name, 1));

    let reply = call(
        app_against(&mock),
        "GET",
        Some(&session_cookie("123456789012345678")),
    )
    .await;

    assert_eq!(
        reply.status,
        StatusCode::NOT_FOUND,
        "the neighbour's key is not theirs"
    );
    assert_eq!(reply.json()["code"], "no_key");
    assert!(
        mock.with(|s| s.keys.iter().any(|k| k.id == victim)),
        "the other user's key must survive untouched"
    );
    assert!(mock.with(|s| s.deleted.is_empty()));
}

/// Names a human typed in the console prefix ours too. They are neither
/// returned nor (self-evidently now) deleted.
#[tokio::test]
async fn console_created_lookalikes_are_not_revealed() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| {
        s.seed(&format!("{name}-old"), 1);
        s.seed(&format!("{name}s"), 2);
        s.seed(&format!("{name}-BACKUP"), 3);
    });

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "none of those is ours");
    assert_eq!(reply.json()["code"], "no_key");
    assert_eq!(mock.with(|s| s.keys.len()), 3);
    assert!(mock.with(|s| s.deleted.is_empty()));
}

// ---------------------------------------------------------------------------
// Caching, secrecy and failure
// ---------------------------------------------------------------------------

/// The handler's own half of "reveal is not cached". The other two halves are
/// the gateway's `cachingEnabled: false` and CloudFront's `CachingDisabled`,
/// both asserted against the synthesized templates by
/// `tools/scripts/verify-openapi-routes.mjs`.
#[tokio::test]
async fn every_answer_carries_no_store() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| s.seed(&name, 1_000));

    assert_eq!(reveal(&mock, USER_ID).await.cache_control(), "no-store");
    assert_eq!(post_key(&mock, USER_ID).await.cache_control(), "no-store");

    // The `no_key` refusal — a cached one would tell a fresh key-holder they
    // have nothing.
    mock.with(|s| s.keys.clear());
    assert_eq!(reveal(&mock, USER_ID).await.cache_control(), "no-store");

    // And the 401 — a cached one is a signed-in visitor being told they are
    // signed out.
    assert_eq!(
        call(app_against(&mock), "GET", None).await.cache_control(),
        "no-store"
    );
}

/// The listing must not ask for values. A lookup that requested them would
/// pull every matched key's credential into this process for a field nothing
/// reads until the single `GetApiKey` at the end.
#[tokio::test]
async fn the_listing_never_requests_key_values() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| s.seed(&name, 1_000));
    reveal(&mock, USER_ID).await;
    let seen = mock.with(|s| s.include_values_seen.clone());
    assert!(!seen.is_empty(), "the listing must have run");
    for value in seen {
        assert_eq!(value, "false", "GetApiKeys asked for key values");
    }
}

/// A control plane that will not answer is a `502`, not a `500`: the failure
/// is upstream, and saying so is what stops an operator hunting for a bug here
/// during an AWS incident. It is also not a `404`, which would read as "you
/// have no key".
#[tokio::test]
async fn a_control_plane_failure_is_a_502() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.fail_list = true);
    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.json()["code"], "key_unavailable");
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(mock.with(|s| s.create_calls), 0);
}

/// A `GetApiKey` that fails for a reason that is **not** "gone" is a `502`,
/// not `no_key`: an AccessDenied or a throttle must not read as "you have no
/// key" and send the visitor off to re-issue.
#[tokio::test]
async fn a_read_that_fails_for_any_other_reason_is_a_502_and_creates_nothing() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| {
        s.seed(&name, 1_000);
        s.fail_reads = true;
    });

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.json()["code"], "key_unavailable");
    assert_eq!(
        mock.with(|s| s.create_calls),
        0,
        "a failed read must not be mistaken for a missing key"
    );
    assert_eq!(mock.with(|s| s.keys.len()), 1, "and nothing was deleted");
}

/// **A slow control plane gets an answer, not a dead invocation.** The
/// per-call timeouts do not compose into a request-level bound — a listing can
/// walk 50 pages each with its own budget. Against a 15s Lambda that is a
/// killed invocation; the wall-clock deadline turns it into a `503`.
#[tokio::test]
async fn a_control_plane_slower_than_the_deadline_answers_503() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.list_delay_ms = 400);

    let started = std::time::Instant::now();
    let reply = call(
        keys_router_with_deadline(&mock, std::time::Duration::from_millis(50)),
        "POST",
        Some(&session_cookie(USER_ID)),
    )
    .await;
    let elapsed = started.elapsed();

    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.json()["code"], "key_unavailable");
    assert_eq!(reply.cache_control(), "no-store");
    assert!(
        elapsed < std::time::Duration::from_millis(350),
        "the deadline did not cut the work off: {elapsed:?}"
    );
    assert_eq!(
        mock.with(|s| s.create_calls),
        0,
        "nothing was created on the way out"
    );
}
