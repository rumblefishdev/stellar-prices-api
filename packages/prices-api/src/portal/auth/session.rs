//! The portal session (task 0186).
//!
//! A signed cookie carrying a Discord user ID and an expiry, and **nothing that
//! would have to be revoked**. There is no server-side session store: the cookie
//! is self-contained and self-authenticating, which is what makes it survivable
//! across a Lambda that has no database of its own and may be a different
//! execution environment on every request.
//!
//! # What is deliberately not in it
//!
//! - **No Discord access or refresh token.** Once the callback has read the user
//!   ID the token has no further use here, and keeping it would create a
//!   credential to protect, rotate and leak — for a capability (acting as the
//!   user on Discord) this service never wants. [`super::discord`] drops it at
//!   the end of the callback and nothing writes it anywhere.
//! - **No eligibility claim.** ADR 0010 §8: a signed "this person is a member"
//!   would date the verdict to sign-in time and would still be sitting in the
//!   cookie weeks later when [0191]'s rework asks the same question. Eligibility
//!   travels by re-authentication, per action, and that is [0189]'s to build.
//!
//! # What is in it, and one thing that is a judgement call
//!
//! `sub` (the Discord user ID) is the account key, per ADR 0010 §1. `exp` is the
//! expiry. `name` — the Discord username — is **display only**, and putting it
//! here is a decision worth naming rather than burying:
//!
//! The acceptance criterion is that the page shows the visitor's username and
//! ID. Without a registry ([0190], and not yet built) and without a stored token
//! (see above), the only place a username can survive the redirect back to the
//! bundle is the cookie itself. It is inside the signed payload, so it cannot be
//! forged; it is not a credential and nothing is authorized by it; and it is
//! refreshed on every sign-in, so a rename shows up on the visitor's next
//! sign-in rather than never. What it must not become is an input to a decision
//! — the ID is the identity, always.

use serde::{Deserialize, Serialize};

use super::crypto::{self, CTX_SESSION};

/// How long a session lasts.
///
/// Twenty-four hours. The portal is a place a visitor goes to do one thing —
/// sign in, get a key, occasionally check usage — not an application they live
/// in, so a long-lived cookie buys little. Against that, from [0187] onwards
/// this cookie authorizes revealing an API key, and it is `SameSite=Lax` on a
/// host shared with another application. Re-signing in costs a redirect and no
/// consent
/// screen (Discord does not re-prompt for scopes already granted), which is
/// what makes a short session cheap here and not merely virtuous.
pub const SESSION_TTL_SECS: u64 = 24 * 60 * 60;

/// A signed-in visitor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// The Discord user ID (a snowflake, as a string). The account key.
    pub sub: String,
    /// The Discord username. Display only — see the module docs.
    pub name: String,
    /// Unix seconds at which this cookie stops being accepted.
    pub exp: u64,
}

impl Session {
    /// Mint a session for `sub`/`name`, expiring [`SESSION_TTL_SECS`] from `now`.
    pub fn issue(sub: impl Into<String>, name: impl Into<String>, now: u64) -> Self {
        Self {
            sub: sub.into(),
            name: name.into(),
            exp: now + SESSION_TTL_SECS,
        }
    }

    /// The cookie value: `<payload>.<mac>`, base64url both.
    pub fn encode(&self, key: &[u8]) -> String {
        let json = serde_json::to_vec(self).expect("a session is plain data and always serializes");
        crypto::sign(key, CTX_SESSION, &json)
    }

    /// Read a session back, or `None` if the cookie is absent, altered, signed
    /// with another key, minted as a different kind of token, or expired.
    ///
    /// Expiry is enforced HERE and not left to the browser's `Max-Age`. The two
    /// normally agree, but only one of them is under our control: a cookie
    /// replayed by something that is not a browser carries no `Max-Age` at all,
    /// so a check that trusted it would mean the session never actually expired.
    pub fn decode(key: &[u8], cookie: &str, now: u64) -> Option<Self> {
        let payload = crypto::verify(key, CTX_SESSION, cookie)?;
        let session: Self = serde_json::from_slice(&payload).ok()?;
        (session.exp > now).then_some(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"session-test-key-of-a-realistic-length-here";
    const NOW: u64 = 1_800_000_000;

    #[test]
    fn a_session_round_trips_through_its_cookie() {
        let issued = Session::issue("123456789012345678", "adam", NOW);
        let cookie = issued.encode(KEY);
        assert_eq!(Session::decode(KEY, &cookie, NOW).as_ref(), Some(&issued));
    }

    #[test]
    fn a_session_expires_and_the_check_does_not_depend_on_the_browser() {
        let cookie = Session::issue("1", "adam", NOW).encode(KEY);
        assert!(Session::decode(KEY, &cookie, NOW + SESSION_TTL_SECS - 1).is_some());
        assert!(Session::decode(KEY, &cookie, NOW + SESSION_TTL_SECS).is_none());
    }

    /// The forgery that matters: change `sub` and you are someone else. From
    /// [0187] on, that is someone else's API key.
    #[test]
    fn an_edited_user_id_invalidates_the_cookie() {
        let cookie = Session::issue("111", "adam", NOW).encode(KEY);
        let (_, mac) = cookie.split_once('.').unwrap();
        let forged = format!(
            "{}.{mac}",
            crypto::b64_encode(
                &serde_json::to_vec(&Session {
                    sub: "222".into(),
                    name: "adam".into(),
                    exp: NOW + SESSION_TTL_SECS,
                })
                .unwrap()
            )
        );
        assert!(Session::decode(KEY, &forged, NOW).is_none());
    }

    #[test]
    fn a_cookie_signed_with_another_key_is_not_a_session() {
        let cookie = Session::issue("1", "adam", NOW).encode(b"some-other-signing-key");
        assert!(Session::decode(KEY, &cookie, NOW).is_none());
    }

    /// The state parameter travels through the visitor's own address bar. If it
    /// verified here, `/auth/login` would be a session vending machine.
    #[test]
    fn a_state_parameter_is_not_accepted_as_a_session() {
        let started =
            super::super::state_token::start(KEY, super::super::state_token::Action::SignIn, NOW);
        assert!(Session::decode(KEY, &started.state_param, NOW).is_none());
        assert!(Session::decode(KEY, &started.pending_cookie, NOW).is_none());
    }

    #[test]
    fn garbage_is_rejected_rather_than_panicking() {
        for bad in ["", ".", "x.y", "not-base64!.also-not"] {
            assert!(Session::decode(KEY, bad, NOW).is_none(), "accepted {bad:?}");
        }
    }

    /// No Discord token is representable in this type, let alone stored. Asserted
    /// on the serialized form so adding a field is what fails, rather than a
    /// reviewer having to notice.
    #[test]
    fn the_serialized_session_carries_exactly_three_fields_and_no_token() {
        let json: serde_json::Value =
            serde_json::to_value(Session::issue("1", "adam", NOW)).unwrap();
        let object = json.as_object().unwrap();
        assert_eq!(object.len(), 3, "unexpected session fields: {object:?}");
        for field in ["sub", "name", "exp"] {
            assert!(object.contains_key(field));
        }
        let text = json.to_string().to_lowercase();
        for forbidden in ["access_token", "refresh_token", "bearer"] {
            assert!(!text.contains(forbidden), "session carries {forbidden}");
        }
    }
}
