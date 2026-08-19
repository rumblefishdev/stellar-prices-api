//! Issuing and revealing a key, over HTTP, against a mock control plane
//! (task 0187).
//!
//! The unit tests next to `portal/keys/naming.rs` cover the rules — the name,
//! the exact filter, the winner — on lists somebody typed by hand. This file
//! covers what none of them can: the two routes wired into the real
//! [`prices_api::app`] router, driving the **actual AWS SDK** against a mock
//! API Gateway control plane bound to loopback. The mock, and the argument for
//! why it is a service rather than a trait, live in `portal_keys/harness.rs`.
//!
//! The one assertion that is **not** here is "no key value reaches the logs":
//! it needs a process to itself and lives in `portal_keys_logs.rs`.

#[path = "portal_keys/harness.rs"]
mod harness;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::json;
use tower::ServiceExt;

use harness::*;

// ---------------------------------------------------------------------------
// The gate (AC 1)
// ---------------------------------------------------------------------------

/// **The slice the flag exists for.** Until [0189]'s eligibility gate lands,
/// `PORTAL_ENABLED=false` is the only thing between a stranger who can sign in
/// and a real production API key — so "closed" is asserted on both verbs, with a
/// valid session presented, and byte-for-byte against a path that was never
/// routed.
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

/// With the portal open and nothing provisioned the routes still exist and say
/// so — a `503`, not a `404`, so a half-configured deployment is distinguishable
/// from a closed one.
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

/// No session, no key — and, just as importantly, **no call to AWS**. A route
/// that reached the control plane before checking the cookie would let an
/// anonymous caller drive `GetApiKeys` at the portal's 10 req/s throttle.
#[tokio::test]
async fn issuing_without_a_session_is_refused_before_aws_is_touched() {
    let mock = MockGateway::start().await;
    for method in ["GET", "POST"] {
        let reply = call(app_against(&mock), method, None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method}");
        assert_eq!(reply.json()["code"], "not_signed_in", "{method}");
    }
    assert_eq!(mock.with(|s| s.list_calls), 0);
    assert_eq!(mock.with(|s| s.create_calls), 0);
}

/// The forgery that matters: edit `sub` and you are someone else — and from this
/// slice on, someone else's API key. The signature is what stops it, and nothing
/// reaches AWS on the way to finding that out.
#[tokio::test]
async fn a_forged_session_cannot_issue_a_key() {
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
// Issue and reveal (ACs 2, 4, 5)
// ---------------------------------------------------------------------------

/// The first press: one key, named for the user, enabled, tagged, on the free
/// plan, and its value on screen.
#[tokio::test]
async fn the_first_press_creates_one_key_on_the_free_plan_and_shows_it() {
    let mock = MockGateway::start().await;
    let reply = issue(&mock, USER_ID).await;

    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["name"], format!("discord-{USER_ID}-key"));
    assert_eq!(body["created"], true);

    let stored = mock.with(|s| s.keys.clone());
    assert_eq!(stored.len(), 1, "exactly one key");
    assert_eq!(body["key_id"], stored[0].id);
    assert_eq!(body["value"], stored[0].value);
    assert!(
        stored[0].enabled,
        "a key that is not enabled cannot be used"
    );
    assert_eq!(
        stored[0].tags.get("ManagedBy").map(String::as_str),
        Some("prices-portal"),
        "the tag is the only attribution `CreateApiKey` can be given"
    );

    // Attached to the plan whose id came from SSM — which is what makes the key
    // work against `/v1/` rather than authenticate and then be refused.
    assert_eq!(
        mock.with(|s| s.plan_keys.clone()),
        vec![(PLAN_ID.to_string(), stored[0].id.clone())]
    );
}

/// A second press returns the same key. Not a new one, and not a second create.
#[tokio::test]
async fn a_second_press_returns_the_same_key() {
    let mock = MockGateway::start().await;
    let first = issue(&mock, USER_ID).await.json();
    let second = issue(&mock, USER_ID).await.json();

    assert_eq!(first["key_id"], second["key_id"]);
    assert_eq!(first["value"], second["value"]);
    assert_eq!(second["created"], false);
    assert_eq!(mock.with(|s| s.create_calls), 1);
    assert_eq!(mock.with(|s| s.keys.len()), 1);
}

/// Signing out and back in is a new cookie for the same Discord id, and the key
/// is a property of the id — so it survives. This is the criterion that would
/// fail if anything about the key were kept in the session.
#[tokio::test]
async fn signing_out_and_back_in_still_shows_the_same_key() {
    let mock = MockGateway::start().await;
    let before = issue(&mock, USER_ID).await.json();
    // A fresh cookie, minted as a second sign-in would mint it.
    let after = reveal(&mock, USER_ID).await.json();

    assert_eq!(before["key_id"], after["key_id"]);
    assert_eq!(before["value"], after["value"]);
    assert_eq!(mock.with(|s| s.create_calls), 1);
}

/// Two people get two keys, and neither is handed the other's.
#[tokio::test]
async fn two_users_get_two_different_keys() {
    let mock = MockGateway::start().await;
    let mine = issue(&mock, USER_ID).await.json();
    let theirs = issue(&mock, "111111111111111111").await.json();
    assert_ne!(mine["key_id"], theirs["key_id"]);
    assert_ne!(mine["value"], theirs["value"]);
    assert_eq!(mock.with(|s| s.keys.len()), 2);
}

// ---------------------------------------------------------------------------
// The reconciler (ACs 6, 7, 8)
// ---------------------------------------------------------------------------

/// Duplicates converge on the **earliest** key and the rest are deleted. This is
/// the state a double-submit leaves behind, seeded directly so the assertion is
/// about the rule rather than about a scheduler.
#[tokio::test]
async fn duplicates_converge_on_the_earliest_and_the_losers_are_deleted() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let (early, late, later) = mock.with(|s| {
        (
            s.seed(&name, 1_000),
            s.seed(&name, 2_000),
            s.seed(&name, 3_000),
        )
    });

    let body = issue(&mock, USER_ID).await.json();

    assert_eq!(body["key_id"], early, "the earliest key must survive");
    assert_eq!(body["created"], false);
    let mut deleted = mock.with(|s| s.deleted.clone());
    deleted.sort();
    let mut expected = vec![late, later];
    expected.sort();
    assert_eq!(deleted, expected);
    assert_eq!(mock.with(|s| s.named(&name).len()), 1);
}

/// Two simultaneous first presses. However they interleave, the user ends with
/// exactly one key — and the reconciler is what gets them there.
#[tokio::test]
async fn two_simultaneous_first_presses_leave_exactly_one_key() {
    let mock = MockGateway::start().await;
    let (a, b) = tokio::join!(issue(&mock, USER_ID), issue(&mock, USER_ID));

    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(b.status, StatusCode::OK);

    let name = format!("discord-{USER_ID}-key");
    assert_eq!(
        mock.with(|s| s.named(&name).len()),
        1,
        "the reconciler must converge on one key"
    );

    // And the survivor is what a subsequent reveal hands out, so whichever of
    // the two lost, the user is not left holding a deleted value.
    let survivor = mock.with(|s| s.named(&name)[0].id.clone());
    let after = reveal(&mock, USER_ID).await.json();
    assert_eq!(after["key_id"], survivor);
}

/// The reconciler must see **every** page before it ranks. Ranking off page one
/// picks a winner from a partial list and then deletes a key it never saw.
///
/// Five duplicates at a page size of two: three pages, with the earliest key
/// deliberately on the last one, so an implementation that stops early both
/// returns the wrong key and deletes the right one.
#[tokio::test]
async fn the_reconciler_pages_get_api_keys_to_exhaustion_before_ranking() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let earliest = mock.with(|s| {
        s.page_size = 2;
        s.seed(&name, 5_000);
        s.seed(&name, 4_000);
        s.seed(&name, 3_000);
        s.seed(&name, 2_000);
        s.seed(&name, 1_000) // earliest, and last in the listing
    });

    let body = issue(&mock, USER_ID).await.json();

    assert_eq!(
        body["key_id"], earliest,
        "the winner came from the last page; a partial list was ranked"
    );
    assert_eq!(
        mock.with(|s| s.list_calls),
        3,
        "five keys at a page size of two is three pages"
    );
    assert_eq!(mock.with(|s| s.deleted.len()), 4);
}

/// A key deleted by hand in the console is re-created, not returned as a dead
/// id. Without a registry, "deleted by hand" and "never issued" are the same
/// observation — which is why a reveal is a reconciliation.
#[tokio::test]
async fn a_key_deleted_by_hand_is_recreated_on_the_next_reveal() {
    let mock = MockGateway::start().await;
    let first = issue(&mock, USER_ID).await.json();

    // Somebody opens the console and deletes it.
    mock.with(|s| s.keys.clear());

    let second = reveal(&mock, USER_ID).await;
    assert_eq!(second.status, StatusCode::OK);
    let second = second.json();
    assert_ne!(
        second["key_id"], first["key_id"],
        "a new key, not the old id"
    );
    assert_eq!(second["created"], true);
    assert_eq!(
        second["key_id"],
        mock.with(|s| s.keys[0].id.clone()),
        "the id returned must be one that exists"
    );
}

/// The narrower race: the key is listed, and gone by the time its value is
/// read. The flow re-runs rather than reporting a failure or a dead id.
#[tokio::test]
async fn a_key_that_vanishes_between_the_list_and_the_read_is_not_a_dead_id() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| {
        s.seed(&name, 1_000);
        s.vanish_on_next_read = true;
    });

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();

    let live: Vec<String> = mock.with(|s| s.keys.iter().map(|k| k.id.clone()).collect());
    assert!(
        live.contains(&body["key_id"].as_str().unwrap().to_string()),
        "returned {:?}, which is not among the keys that exist: {live:?}",
        body["key_id"]
    );
    assert_eq!(body["created"], true, "the replacement was created");
}

/// A listing that keeps returning a key the read will not produce: every
/// attempt settles on a winner it cannot get a value for.
///
/// The answer is a `503` — "try again", because the condition is transient —
/// rather than a `502`, which would blame AWS for a control plane that is
/// answering both calls, or an unbounded retry, which would spend the rest of
/// the Lambda's budget and return a timeout instead of an answer.
///
/// **Reaching this branch takes a stale LISTING, not a busy deleter**, and that
/// is worth recording: an attempt that creates the winner already holds its
/// value and never reads it back, so a mock that merely deletes keys on read
/// makes the next attempt create one and succeed. The retry exists for a reader
/// and a lister that disagree.
#[tokio::test]
async fn a_winner_whose_value_never_reads_is_a_bounded_503() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    mock.with(|s| {
        s.seed(&name, 1_000);
        s.read_always_404 = true;
    });

    let reply = reveal(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.json()["code"], "key_unavailable");
    assert_eq!(reply.cache_control(), "no-store");
    // Bounded at two attempts, so this is an answer rather than a timeout.
    assert_eq!(
        mock.with(|s| s.list_calls),
        2,
        "one listing per attempt, and exactly two attempts"
    );
    assert_eq!(mock.with(|s| s.create_calls), 0, "nothing was created");
}

// ---------------------------------------------------------------------------
// The prefix hazard (AC 9)
// ---------------------------------------------------------------------------

/// A shorter Discord id is a prefix of a longer one, and `nameQuery` is a prefix
/// match. The `-key` suffix plus the exact filter are what stop user `123…` from
/// seeing — or **deleting** — the key of user `123…9`.
///
/// The mock matches by prefix exactly as the service does, so this test fails if
/// either guard is removed.
#[tokio::test]
async fn a_user_whose_id_prefixes_another_can_neither_see_nor_delete_their_key() {
    let mock = MockGateway::start().await;
    let victim_name = "discord-1234567890123456789-key";
    let victim = mock.with(|s| s.seed(victim_name, 1));

    let body = issue(&mock, "123456789012345678").await.json();

    assert_ne!(body["key_id"], victim);
    assert!(
        mock.with(|s| s.keys.iter().any(|k| k.id == victim)),
        "the other user's key was deleted by the reconciler"
    );
    assert!(
        mock.with(|s| s.deleted.is_empty()),
        "nothing at all should have been deleted"
    );
}

/// Names a human typed in the console prefix ours too, and the reconciler holds
/// `DeleteApiKey`. They are neither returned nor deleted.
#[tokio::test]
async fn console_created_lookalikes_are_neither_returned_nor_deleted() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let lookalikes = mock.with(|s| {
        [
            s.seed(&format!("{name}-old"), 1),
            s.seed(&format!("{name}s"), 2),
            s.seed(&format!("{name}-BACKUP"), 3),
        ]
    });

    let body = issue(&mock, USER_ID).await.json();

    assert_eq!(body["created"], true, "none of those was ours");
    for id in lookalikes {
        assert!(
            mock.with(|s| s.keys.iter().any(|k| k.id == id)),
            "a console-created key named like ours was deleted"
        );
        assert_ne!(body["key_id"], id);
    }
    assert!(mock.with(|s| s.deleted.is_empty()));
}

// ---------------------------------------------------------------------------
// Caching, secrecy and failure (AC 10)
// ---------------------------------------------------------------------------

/// The handler's own half of "reveal is not cached". The other two halves are
/// the gateway's `cachingEnabled: false` and CloudFront's `CachingDisabled`,
/// both asserted against the synthesized templates by
/// `tools/scripts/verify-openapi-routes.mjs`.
#[tokio::test]
async fn every_answer_carries_no_store() {
    let mock = MockGateway::start().await;
    assert_eq!(issue(&mock, USER_ID).await.cache_control(), "no-store");
    assert_eq!(reveal(&mock, USER_ID).await.cache_control(), "no-store");
    // Including the refusals — a cached `401` is a signed-in visitor being told
    // they are signed out.
    assert_eq!(
        call(app_against(&mock), "GET", None).await.cache_control(),
        "no-store"
    );
}

/// The listing must not ask for values. A reconciler sweep that requested them
/// would pull every matched key's credential into this process for a field
/// nothing reads.
#[tokio::test]
async fn the_listing_never_requests_key_values() {
    let mock = MockGateway::start().await;
    issue(&mock, USER_ID).await;
    let seen = mock.with(|s| s.include_values_seen.clone());
    assert!(!seen.is_empty(), "the listing must have run");
    for value in seen {
        assert_eq!(value, "false", "GetApiKeys asked for key values");
    }
}

/// A control plane that will not answer is a `502`, not a `500`: the failure is
/// upstream, and saying so is what stops an operator hunting for a bug here
/// during an AWS incident. It is also not a `404`, which would read as "you have
/// no key" and send the visitor to press the button again.
#[tokio::test]
async fn a_control_plane_failure_is_a_502() {
    let mock = MockGateway::start().await;
    mock.with(|s| s.fail_list = true);
    let reply = issue(&mock, USER_ID).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.json()["code"], "key_unavailable");
    assert_eq!(reply.cache_control(), "no-store");
    assert_eq!(mock.with(|s| s.create_calls), 0);
}

/// A `GetApiKey` that fails for a reason that is **not** "gone" is a `502`, not
/// a silent re-issue.
///
/// The distinction lives in one line of `Gateway::value_of`: a `404` becomes
/// `Ok(None)` and re-enters the flow, everything else is an error. Get that
/// wrong in the permissive direction — treat any failure as "gone" — and an
/// AccessDenied or a throttle would make the handler create a **second** key for
/// a user who already has one, on every request, which is the one thing the
/// reconciler exists to prevent.
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

/// **A key that exists but is on no usage plan is put on it, not handed out as
/// it is.**
///
/// Three things produce such a key and only the first is ours to prevent: a
/// `CreateApiKey` that succeeded followed by a `CreateUsagePlanKey` that failed
/// (these are control-plane calls, throttled hard); a `CreateApiKey` that timed
/// out *after* the service made the key, so no id ever reached us; and somebody
/// creating one with this exact name in the console.
///
/// While the attach lived on the create path, all three were **permanent**: the
/// next request adopted the orphan, answered `200`, and the holder had a key
/// that returns `403` from `/v1/` with no retry that could fix it — every retry
/// took the same branch. This is the test that fails if the attach is moved back
/// there.
#[tokio::test]
async fn an_adopted_key_is_put_on_the_free_plan() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let orphan = mock.with(|s| s.seed(&name, 1_000));

    let body = issue(&mock, USER_ID).await.json();

    assert_eq!(body["key_id"], orphan, "the existing key is adopted");
    assert_eq!(body["created"], false, "and it was not re-created");
    assert_eq!(
        mock.with(|s| s.plan_keys.clone()),
        vec![(PLAN_ID.to_string(), orphan)],
        "an adopted key is only useful once it is on the plan"
    );
    assert_eq!(
        mock.with(|s| s.create_calls),
        0,
        "adopting must not mint a second key"
    );
}

/// The attach runs on **every** request, so the ordinary case is a key that is
/// already on the plan — which API Gateway answers with `409 ConflictException`.
///
/// That is the desired state, not a failure, and this is what makes attaching
/// unconditionally affordable: the conflict costs one call and changes nothing.
/// The mock answers `409` like the service, so a handler that treated the
/// conflict as an error would fail here rather than in production on the second
/// press.
#[tokio::test]
async fn a_key_already_on_the_plan_stays_on_it_and_is_not_an_error() {
    let mock = MockGateway::start().await;

    let first = issue(&mock, USER_ID).await;
    let second = issue(&mock, USER_ID).await;
    let third = reveal(&mock, USER_ID).await;

    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(third.status, StatusCode::OK);
    assert_eq!(first.json()["key_id"], third.json()["key_id"]);

    let key_id = first.json()["key_id"].as_str().unwrap().to_string();
    assert_eq!(
        mock.with(|s| s.plan_keys.clone()),
        vec![(PLAN_ID.to_string(), key_id)],
        "one membership, however many times it was asserted"
    );
    assert_eq!(
        mock.with(|s| s.attach_calls),
        3,
        "one attach per request — the conflict is the answer, not a retry"
    );
}

/// **A key that vanishes before the ATTACH is not a dead end either.**
///
/// The sibling of `a_key_that_vanishes_between_the_list_and_the_read_is_not_a_dead_id`,
/// and it exists because attaching the winner moved that race earlier: the
/// attach now runs before the read, so a key deleted in the console after the
/// listing is observed by `CreateUsagePlanKey` first. Reporting that `404` as a
/// control-plane failure would have restored the dead end this slice removed —
/// on a narrower path, which is the kind that stays broken because nothing
/// exercises it.
#[tokio::test]
async fn a_key_that_vanishes_before_the_attach_is_not_a_dead_id() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let doomed = mock.with(|s| {
        let id = s.seed(&name, 1_000);
        s.vanish_on_next_attach = true;
        id
    });

    let reply = reveal(&mock, USER_ID).await;

    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_ne!(body["key_id"], doomed, "the vanished key is not handed out");
    assert_eq!(body["created"], true, "a replacement was made");
    let survivors = mock.with(|s| {
        s.named(&name)
            .iter()
            .map(|k| k.id.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        survivors,
        vec![body["key_id"].as_str().unwrap().to_string()],
        "and the id returned is the one that exists"
    );
    assert_eq!(
        mock.with(|s| s.plan_keys.len()),
        1,
        "the replacement is on the plan"
    );
}

/// **A duplicate that will not delete does not withhold the key.**
///
/// By the time the deletions run, the winner is created, attached and ready.
/// Propagating a failed `DeleteApiKey` would answer `502` and hand the caller
/// nothing — housekeeping denying the thing the request was for — and it would
/// not even be transient: task 0194 may put an `aws:ResourceTag/ManagedBy`
/// condition on `DELETE`, and an exact-name duplicate created by hand in the
/// console carries no tag, so it would fail on every request forever.
#[tokio::test]
async fn a_duplicate_that_will_not_delete_does_not_withhold_the_key() {
    let mock = MockGateway::start().await;
    let name = format!("discord-{USER_ID}-key");
    let earliest = mock.with(|s| {
        let earliest = s.seed(&name, 1_000);
        s.seed(&name, 2_000);
        s.fail_deletes = true;
        earliest
    });

    let reply = issue(&mock, USER_ID).await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json()["key_id"],
        earliest,
        "the winner is still the earliest, and it is still handed out"
    );
    assert_eq!(
        mock.with(|s| s.keys.len()),
        2,
        "the duplicate survives, for the next reconciliation to try again"
    );
    assert_eq!(
        mock.with(|s| s.create_calls),
        0,
        "and nothing was re-created to work around it"
    );
}
