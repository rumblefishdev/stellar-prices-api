//! The `state` parameter and the cookie that binds it to one browser
//! (task 0186).
//!
//! `/auth/callback` is a public, keyless `GET` that a stranger can call with any
//! query string they like. Everything that stops that from being a login-CSRF —
//! signing a victim into an attacker's Discord account, or the reverse — is in
//! this file.
//!
//! # The shape
//!
//! `/auth/login` mints **two** values from one nonce:
//!
//! | | goes to | carries | signed under |
//! | --- | --- | --- | --- |
//! | [`StateParam`] | Discord, in the redirect URL | action, nonce, expiry | [`CTX_STATE`] |
//! | [`PendingLogin`] | the browser, as a cookie | action, nonce, PKCE verifier, expiry | [`CTX_PENDING`] |
//!
//! The callback receives the first back from Discord and the second from the
//! browser, and [`PendingLogin::accept`] requires them to agree. That is the
//! binding the task asks for: a `state` is only good in the browser that
//! requested it, because the other half never left that browser.
//!
//! Four independent rejections fall out of it, and each is a distinct test:
//!
//! - **No cookie** — a callback nobody asked for. Also what a replay hits, see
//!   below.
//! - **Bad signature on either half** — a forged or edited value. The nonce is
//!   inside the signed payload rather than compared against a separately-stored
//!   copy, so there is no server-side store to grow or to expire.
//! - **Nonce mismatch** — both halves are well-formed and signed, but they came
//!   from different `/auth/login` calls. This is the case a single signed value
//!   would not catch: an attacker who calls `/auth/login` themselves has a
//!   genuine, correctly-signed `state`, and pasting it into a victim's browser
//!   is the whole CSRF. Their nonce does not match the victim's cookie.
//! - **Action mismatch** — see below.
//! - **Expired** — a ten-minute window, checked on both halves.
//!
//! # Replay
//!
//! The callback **deletes the cookie before it does anything else** with the
//! result, so the second presentation of the same `code`+`state` finds no cookie
//! and is rejected at the first check. That is deliberately not the only
//! defence — Discord's authorization codes are single-use, and the expiry bounds
//! the window — but it is the one this service controls, and it is the reason
//! nothing here needs a used-nonce table.
//!
//! # The action slot
//!
//! [`Action`] has one variant today and the task requires it anyway, because
//! [0189] binds "issue a key" and "rework a key" to a round-trip and adding the
//! field then would mean re-deriving the signing format while a deployed portal
//! holds live cookies in the old one. It is verified now, on both halves, so
//! that when a second action exists the check is already there and already
//! tested rather than being added alongside the thing it is meant to constrain.
//!
//! ADR 0010 §8 is what the slot is for: eligibility travels by
//! re-authentication, not in the session, and "the callback completes that
//! action and nothing else". A callback for a sign-in must not be usable to
//! complete an issuance.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::crypto::{self, CTX_PENDING, CTX_STATE};

/// How long a started sign-in stays completable.
///
/// Ten minutes is a consent screen plus a Discord login plus a two-factor
/// prompt, with room to spare; it is short enough that an intercepted `state`
/// has nowhere to be used. Both halves carry it, so shortening it later cannot
/// be defeated by an old cookie.
pub const PENDING_TTL_SECS: u64 = 600;

/// Bytes of OS entropy behind the nonce and the PKCE verifier.
///
/// 32 for both. The verifier's floor is RFC 7636's 32 octets; the nonce has no
/// standard, and matching it keeps one number in the file.
const TOKEN_BYTES: usize = 32;

/// What the round-trip is being performed *for*.
///
/// Serialized as a short string rather than an integer so a signed value is
/// legible in a log or a browser address bar, and so [0189] adding a variant
/// cannot renumber an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Establish who the visitor is. The only action in this slice.
    #[serde(rename = "signin")]
    SignIn,
    /// A second action that exists **only under `cfg(test)`**.
    ///
    /// Not [0189]'s `issue` arriving early — it is never parsed from a query
    /// string, never minted by [`start`] outside a test, and is compiled out of
    /// every shipped binary. It exists because with a single variant the
    /// [`StateError::ActionMismatch`] arm in [`accept`] is **unreachable**, and
    /// an unreachable branch is an untested one: deleting the comparison
    /// entirely left the whole suite green, which is exactly the regression the
    /// action slot is supposed to be protected against when [0189] adds a real
    /// second action.
    ///
    /// The alternative — asserting the mismatch through a hand-signed payload
    /// carrying an unknown action string — does not test this branch at all. It
    /// fails earlier, at deserialization, and reports [`StateError::BadSignature`].
    #[cfg(test)]
    #[serde(rename = "test-only-other")]
    TestOther,
}

impl Action {
    /// Parse the `action` query parameter of `/auth/login`.
    ///
    /// An unknown value is rejected rather than defaulted. Defaulting would mean
    /// that when [0189] adds `issue`, a client asking for an action this build
    /// does not know would silently get a sign-in — and the callback would then
    /// complete a *different* action than the one confirmed, which is the exact
    /// thing the slot exists to prevent.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "signin" => Some(Self::SignIn),
            // `TestOther` is deliberately absent: it must not be reachable from
            // a query string even in a test build.
            _ => None,
        }
    }
}

/// The `state` query parameter: what Discord echoes back.
#[derive(Debug, Serialize, Deserialize)]
struct StateClaims {
    action: Action,
    nonce: String,
    exp: u64,
}

/// The cookie: what never leaves the browser that started the flow.
#[derive(Debug, Serialize, Deserialize)]
struct PendingClaims {
    action: Action,
    nonce: String,
    /// The PKCE code verifier. This is why the cookie is `HttpOnly` and why the
    /// challenge — not the verifier — is what goes to Discord.
    verifier: String,
    exp: u64,
}

/// A started sign-in: the pair of signed values `/auth/login` hands out.
pub struct StartedLogin {
    /// Goes in the redirect URL as `state`.
    pub state_param: String,
    /// Goes in the [`PENDING_COOKIE`](super::cookies::PENDING_COOKIE).
    pub pending_cookie: String,
    /// Goes in the redirect URL as `code_challenge`.
    pub code_challenge: String,
}

/// Mint a fresh `(state, cookie, challenge)` triple for `action`.
pub fn start(key: &[u8], action: Action, now: u64) -> StartedLogin {
    let nonce = crypto::random_token(TOKEN_BYTES);
    let verifier = crypto::random_token(TOKEN_BYTES);
    let exp = now + PENDING_TTL_SECS;

    let state_param = sign_claims(
        key,
        CTX_STATE,
        &StateClaims {
            action,
            nonce: nonce.clone(),
            exp,
        },
    );
    let pending_cookie = sign_claims(
        key,
        CTX_PENDING,
        &PendingClaims {
            action,
            nonce,
            verifier: verifier.clone(),
            exp,
        },
    );

    StartedLogin {
        state_param,
        pending_cookie,
        code_challenge: crypto::pkce_challenge(&verifier),
    }
}

/// What survives a successful [`accept`]: the verifier the token exchange needs,
/// and the action the callback is now allowed to complete.
#[derive(Debug)]
pub struct AcceptedLogin {
    pub action: Action,
    pub verifier: String,
}

/// Why a callback was refused.
///
/// Distinguished in the *type* but deliberately not on the wire — the handler
/// answers every variant with the same `400` and the same message, because the
/// caller who would benefit from telling them apart is the one probing. They
/// exist so the tests can assert which check fired, which is the difference
/// between "state verification rejects this" and "state verification rejects
/// this for the reason we think".
#[derive(Debug, PartialEq, Eq)]
pub enum StateError {
    /// No pending-login cookie. An unsolicited callback, or a replay of one
    /// that already consumed its cookie.
    NoPendingLogin,
    /// A signature did not verify, or a payload was not the expected shape.
    BadSignature,
    /// Both halves are genuine but came from different `/auth/login` calls.
    NonceMismatch,
    /// The two halves disagree about what is being authorized.
    ActionMismatch,
    /// The ten-minute window closed.
    Expired,
}

/// Verify a callback's `state` against the browser's pending-login cookie.
///
/// `cookie` is `None` when the browser sent none — kept as an argument rather
/// than an early return in the handler so that "there was no cookie" is one of
/// the outcomes this function is tested for, alongside the other four.
pub fn accept(
    key: &[u8],
    state_param: &str,
    cookie: Option<&str>,
    now: u64,
) -> Result<AcceptedLogin, StateError> {
    let cookie = cookie.ok_or(StateError::NoPendingLogin)?;

    let pending: PendingClaims =
        verify_claims(key, CTX_PENDING, cookie).ok_or(StateError::BadSignature)?;
    let state: StateClaims =
        verify_claims(key, CTX_STATE, state_param).ok_or(StateError::BadSignature)?;

    // Order matters only for which error a test sees; every check runs against
    // values that are already authenticated, so none of them is reachable with
    // attacker-chosen content.
    if pending.exp <= now || state.exp <= now {
        return Err(StateError::Expired);
    }
    // Constant time. The nonce is a secret held by one browser, and comparing it
    // with `==` leaks a prefix — the same reasoning as the MAC comparison in
    // `crypto::verify`, applied to the value that comparison protects.
    if !crate::auth::ct_eq(pending.nonce.as_bytes(), state.nonce.as_bytes()) {
        return Err(StateError::NonceMismatch);
    }
    if pending.action != state.action {
        return Err(StateError::ActionMismatch);
    }

    Ok(AcceptedLogin {
        action: state.action,
        verifier: pending.verifier,
    })
}

/// Seconds since the Unix epoch.
///
/// Saturating rather than `expect`: a clock before 1970 is not a reason to panic
/// a request handler, and `0` makes every token look expired, which fails the
/// flow closed.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sign_claims<T: Serialize>(key: &[u8], context: &str, claims: &T) -> String {
    let json = serde_json::to_vec(claims).expect("claims are plain data and always serialize");
    crypto::sign(key, context, &json)
}

fn verify_claims<T: for<'de> Deserialize<'de>>(
    key: &[u8],
    context: &str,
    token: &str,
) -> Option<T> {
    let payload = crypto::verify(key, context, token)?;
    serde_json::from_slice(&payload).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"state-token-test-key-long-enough-to-be-real";
    const NOW: u64 = 1_800_000_000;

    #[test]
    fn a_freshly_started_login_is_accepted_and_yields_its_verifier() {
        let started = start(KEY, Action::SignIn, NOW);
        let accepted = accept(
            KEY,
            &started.state_param,
            Some(&started.pending_cookie),
            NOW + 1,
        )
        .expect("the pair this function just minted must verify");
        assert_eq!(accepted.action, Action::SignIn);
        assert!(!accepted.verifier.is_empty());
    }

    /// The challenge is what goes to Discord; the verifier is what stays in the
    /// cookie. Sending the verifier instead would make PKCE decorative.
    #[test]
    fn the_challenge_is_the_hash_of_the_verifier_and_not_the_verifier() {
        let started = start(KEY, Action::SignIn, NOW);
        let accepted = accept(
            KEY,
            &started.state_param,
            Some(&started.pending_cookie),
            NOW,
        )
        .unwrap();
        assert_eq!(
            started.code_challenge,
            crypto::pkce_challenge(&accepted.verifier)
        );
        assert_ne!(started.code_challenge, accepted.verifier);
    }

    #[test]
    fn a_callback_with_no_cookie_is_refused() {
        let started = start(KEY, Action::SignIn, NOW);
        assert_eq!(
            accept(KEY, &started.state_param, None, NOW).unwrap_err(),
            StateError::NoPendingLogin
        );
    }

    /// The login-CSRF case, and the one a single signed `state` would pass.
    /// Both values here are genuine and correctly signed — they simply belong to
    /// two different sign-ins, which is what pasting your own `state` into
    /// somebody else's browser produces.
    #[test]
    fn a_state_from_a_different_login_does_not_match_this_browsers_cookie() {
        let victim = start(KEY, Action::SignIn, NOW);
        let attacker = start(KEY, Action::SignIn, NOW);
        assert_eq!(
            accept(
                KEY,
                &attacker.state_param,
                Some(&victim.pending_cookie),
                NOW
            )
            .unwrap_err(),
            StateError::NonceMismatch
        );
    }

    #[test]
    fn a_tampered_state_is_refused() {
        let started = start(KEY, Action::SignIn, NOW);
        let (payload, sig) = started.state_param.split_once('.').unwrap();
        // Keep the signature, swap the payload for a well-formed one of our own.
        let forged = format!(
            "{}.{sig}",
            crypto::b64_encode(
                &serde_json::to_vec(&StateClaims {
                    action: Action::SignIn,
                    nonce: "attacker-chosen".into(),
                    exp: NOW + 9_999,
                })
                .unwrap()
            )
        );
        assert_ne!(forged.split_once('.').unwrap().0, payload);
        assert_eq!(
            accept(KEY, &forged, Some(&started.pending_cookie), NOW).unwrap_err(),
            StateError::BadSignature
        );
    }

    #[test]
    fn a_state_signed_with_another_key_is_refused() {
        let started = start(KEY, Action::SignIn, NOW);
        let other = start(b"a-completely-different-signing-key", Action::SignIn, NOW);
        assert_eq!(
            accept(KEY, &other.state_param, Some(&started.pending_cookie), NOW).unwrap_err(),
            StateError::BadSignature
        );
    }

    #[test]
    fn an_expired_pair_is_refused_the_moment_the_window_closes() {
        let started = start(KEY, Action::SignIn, NOW);
        assert!(
            accept(
                KEY,
                &started.state_param,
                Some(&started.pending_cookie),
                NOW + PENDING_TTL_SECS - 1
            )
            .is_ok()
        );
        assert_eq!(
            accept(
                KEY,
                &started.state_param,
                Some(&started.pending_cookie),
                NOW + PENDING_TTL_SECS
            )
            .unwrap_err(),
            StateError::Expired
        );
    }

    /// Replay, at this layer: the handler drops the cookie on the way out, so
    /// the second attempt presents the same `state` with nothing to match it
    /// against. `tests/portal_auth.rs` asserts the same thing over HTTP.
    #[test]
    fn replaying_a_state_without_its_cookie_is_refused() {
        let started = start(KEY, Action::SignIn, NOW);
        assert!(
            accept(
                KEY,
                &started.state_param,
                Some(&started.pending_cookie),
                NOW
            )
            .is_ok()
        );
        assert_eq!(
            accept(KEY, &started.state_param, None, NOW).unwrap_err(),
            StateError::NoPendingLogin
        );
    }

    /// The slot is verified even though only one action exists — otherwise the
    /// check would arrive with [0189] at the same time as the thing it
    /// constrains, which is when it is least likely to be right.
    #[test]
    fn the_action_slot_is_signed_into_both_halves_and_compared() {
        let started = start(KEY, Action::SignIn, NOW);
        // Both halves carry it, so neither can be swapped for a bare nonce.
        let state: StateClaims = verify_claims(KEY, CTX_STATE, &started.state_param).unwrap();
        let pending: PendingClaims =
            verify_claims(KEY, CTX_PENDING, &started.pending_cookie).unwrap();
        assert_eq!(state.action, Action::SignIn);
        assert_eq!(pending.action, Action::SignIn);

        // And a disagreement between them is refused. Constructed by signing a
        // mismatched pair directly, which is the only way to reach this arm
        // while one action exists — and the point is that it stays reachable.
        let crossed = sign_claims(
            KEY,
            CTX_STATE,
            &serde_json::json!({ "action": "issue", "nonce": pending.nonce, "exp": state.exp }),
        );
        assert_eq!(
            accept(KEY, &crossed, Some(&started.pending_cookie), NOW).unwrap_err(),
            // An action this build does not know fails to deserialize, which is
            // the same outcome as a bad signature and is the correct one: an
            // older deployment must not complete an action it cannot check.
            StateError::BadSignature
        );
    }

    /// The branch the crossed-payload test above cannot reach.
    ///
    /// Both halves are genuine, correctly signed, share a nonce and are in
    /// date — they disagree about the ACTION and nothing else. Removing the
    /// comparison in `accept` makes this the only failing test in the suite,
    /// which is the property the slot is carried for: when [0189] adds a real
    /// second action, the check is already there and already exercised.
    #[test]
    fn two_genuine_halves_that_disagree_about_the_action_are_refused() {
        let signin = start(KEY, Action::SignIn, NOW);
        // A pending cookie for a DIFFERENT action, sharing the sign-in's nonce
        // so that the nonce check passes and the action check is what decides.
        let state: StateClaims = verify_claims(KEY, CTX_STATE, &signin.state_param).unwrap();
        let crossed_pending = sign_claims(
            KEY,
            CTX_PENDING,
            &PendingClaims {
                action: Action::TestOther,
                nonce: state.nonce.clone(),
                verifier: "a-verifier".into(),
                exp: state.exp,
            },
        );

        assert_eq!(
            accept(KEY, &signin.state_param, Some(&crossed_pending), NOW).unwrap_err(),
            StateError::ActionMismatch
        );

        // …and the same pair, agreeing, is accepted — so the test above is
        // failing on the action and not on something incidental.
        let agreeing = start(KEY, Action::TestOther, NOW);
        assert_eq!(
            accept(
                KEY,
                &agreeing.state_param,
                Some(&agreeing.pending_cookie),
                NOW
            )
            .unwrap()
            .action,
            Action::TestOther
        );
    }

    #[test]
    fn an_unknown_action_query_value_is_rejected_rather_than_defaulted() {
        assert_eq!(Action::parse("signin"), Some(Action::SignIn));
        for unknown in ["issue", "rework", "", "SIGNIN", "signin "] {
            assert_eq!(Action::parse(unknown), None, "accepted {unknown:?}");
        }
    }
}
