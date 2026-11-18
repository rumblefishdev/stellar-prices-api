//! The hub's token: `v1.<payload>.<mac>` (task 0348; the shared sign-in
//! spec's decision 15).
//!
//! # The format
//!
//! `payload` is base64url without padding of the claims' JSON, and `mac` is
//! base64url without padding of HMAC-SHA256(secret, [`CONTEXT`] ‖ 0x00 ‖
//! ASCII(payload)). That is exactly `auth::crypto::sign` under [`CONTEXT`] —
//! the construction the portal's own cookies use, domain-separated by its
//! context — so this module verifies with `auth::crypto::verify` and adds only
//! the version, the second key and the claims. The spec's test vector is
//! pinned below; the hub pins the same one (`bff/test/token.test.ts`), so a
//! byte changed on either side is a token the other refuses.
//!
//! # What is accepted
//!
//! A token is accepted when all of these hold, and refused otherwise:
//!
//! - it starts `v1.`, and the rest is `<payload>.<mac>` and nothing more;
//! - the MAC verifies, in constant time, under `current` or `previous`;
//! - the payload is the claims object: `iss`, `aud`, `sub`, `name`, `iat`,
//!   `exp`, `jti` and `act` all present and of their type (a claim the spec
//!   does not name is ignored, as a JWT's would be);
//! - `iss` is [`ISSUER`] and `aud` is [`AUDIENCE`], compiled in;
//! - `exp > now - 5 s` and `iat <= now + 5 s` ([`LEEWAY_SECS`]), and
//!   `iat <= exp <= iat + 60 s` ([`MAX_LIFETIME_SECS`]);
//! - `sub` is `discord:` and a snowflake, written canonically (no leading
//!   `0`, and not `0`);
//! - `act` is one of `read`, `reveal`, `revoke`, `issue`. Whether it is the
//!   one the route needs is the route's question (task 0348's `auth`), not
//!   this module's.
//!
//! The MAC is checked before anything in the payload is decoded, so nothing
//! the caller wrote is parsed until it is known to be the hub's.
//!
//! # One answer for every refusal
//!
//! [`verify`] returns `None` for all of them, with no reason, as
//! `auth::crypto::verify` does: the caller answers each with the same `401`,
//! and telling them apart would be a probing oracle. Pure — no I/O, no clock
//! of its own — so every rule is a unit test.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use super::secret::HubSecret;
use crate::portal::auth::crypto;
use crate::portal::auth::discord::is_snowflake;

/// The context the MAC is taken under (decision 15).
pub const CONTEXT: &str = "sorobanscan-hub/prices-token/v1";
/// The only `iss` accepted.
pub const ISSUER: &str = "https://sorobanscan.com/api/hub";
/// The only `aud` accepted.
pub const AUDIENCE: &str = "prices";
/// The only version a token has: everything before the first `.`.
const VERSION_PREFIX: &str = "v1.";
/// The prefix of `sub`; a snowflake follows it.
const SUBJECT_PREFIX: &str = "discord:";
/// How far the hub's clock may be from ours, either way, in seconds.
pub const LEEWAY_SECS: u64 = 5;
/// The longest a token may live, `exp - iat`, in seconds: the hub gives
/// every token 60.
pub const MAX_LIFETIME_SECS: u64 = 60;

/// What a token lets its call do: one per call to the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Act {
    Read,
    Reveal,
    Revoke,
    Issue,
}

impl Act {
    /// The claim's spelling, for a log line.
    pub fn as_str(self) -> &'static str {
        match self {
            Act::Read => "read",
            Act::Reveal => "reveal",
            Act::Revoke => "revoke",
            Act::Issue => "issue",
        }
    }
}

/// Who called, and to do what: the verified token's subject, `act` and `jti`.
///
/// `discord_id` is the bare snowflake, without `discord:`. Its `Debug` leaves
/// it out: the contract's logs name a call by `jti` and `act`, never by the
/// account.
#[derive(Clone, PartialEq, Eq)]
pub struct HubCaller {
    pub discord_id: String,
    pub act: Act,
    pub jti: String,
}

impl std::fmt::Debug for HubCaller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubCaller")
            .field("discord_id", &"<redacted>")
            .field("act", &self.act)
            .field("jti", &self.jti)
            .finish()
    }
}

/// The claims, as the hub writes them. All required.
#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    /// The Discord username. Required, as every claim is, and otherwise
    /// unused: nothing is decided by it, and it is never logged.
    #[serde(rename = "name")]
    _name: String,
    iat: u64,
    exp: u64,
    jti: String,
    act: Act,
}

/// Verify `token` under `secret` at `now`: the caller it names, or `None`
/// for every refusal alike (see the module doc).
pub fn verify(token: &str, secret: &HubSecret, now: SystemTime) -> Option<HubCaller> {
    let signed = token.strip_prefix(VERSION_PREFIX)?;
    let payload = secret
        .keys()
        .find_map(|key| crypto::verify(key, CONTEXT, signed))?;
    let claims: Claims = serde_json::from_slice(&payload).ok()?;
    // Whole seconds, floored: every bound below is an integer, so comparing
    // the floor decides exactly as comparing the instant would.
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    accept(claims, now)
}

/// The claims' rules, after the MAC.
fn accept(claims: Claims, now: u64) -> Option<HubCaller> {
    if claims.iss != ISSUER || claims.aud != AUDIENCE {
        return None;
    }
    // `exp > now - 5 s`, written so nothing underflows.
    if claims.exp.saturating_add(LEEWAY_SECS) <= now {
        return None;
    }
    // `iat <= now + 5 s`.
    if claims.iat > now.saturating_add(LEEWAY_SECS) {
        return None;
    }
    // `exp - iat <= 60 s`, and an `exp` before its `iat` is no lifetime.
    if claims.exp.checked_sub(claims.iat)? > MAX_LIFETIME_SECS {
        return None;
    }
    let discord_id = claims.sub.strip_prefix(SUBJECT_PREFIX)?;
    // Canonical only: no snowflake is `0` or starts with one, and a leading
    // zero would name another key (`discord-0<id>-key`) for the same account.
    if !is_snowflake(discord_id) || discord_id.starts_with('0') {
        return None;
    }
    Some(HubCaller {
        discord_id: discord_id.to_string(),
        act: claims.act,
        jti: claims.jti,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
    use serde_json::{Value, json};

    use super::*;
    use crate::portal::auth::crypto;

    /// The spec's test vector (decision 15), exactly as both sides pin it.
    const VECTOR_SECRET: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";
    const VECTOR_CLAIMS: &str = r#"{"iss":"https://sorobanscan.com/api/hub","aud":"prices","sub":"discord:80351110224678912","name":"nelly","iat":1795000000,"exp":1795000060,"jti":"AAECAwQFBgcICQoLDA0ODw","act":"reveal"}"#;
    const VECTOR_TOKEN: &str = "v1.eyJpc3MiOiJodHRwczovL3Nvcm9iYW5zY2FuLmNvbS9hcGkvaHViIiwiYXVkIjoicHJpY2VzIiwic3ViIjoiZGlzY29yZDo4MDM1MTExMDIyNDY3ODkxMiIsIm5hbWUiOiJuZWxseSIsImlhdCI6MTc5NTAwMDAwMCwiZXhwIjoxNzk1MDAwMDYwLCJqdGkiOiJBQUVDQXdRRkJnY0lDUW9MREEwT0R3IiwiYWN0IjoicmV2ZWFsIn0.aeS9_nACshTGuf2Afv4wbQhvLR-f64XCX0OVmmngofE";
    const VECTOR_IAT: u64 = 1_795_000_000;
    const VECTOR_EXP: u64 = 1_795_000_060;

    /// Another key: 32 bytes of 0xAA.
    const OTHER_SECRET: &str = "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo=";

    fn secret(current: &str, previous: Option<&str>) -> HubSecret {
        let json = match previous {
            Some(previous) => json!({"current": current, "previous": previous}),
            None => json!({"current": current}),
        };
        HubSecret::parse(&json.to_string()).unwrap()
    }

    fn vector_secret() -> HubSecret {
        secret(VECTOR_SECRET, None)
    }

    fn key(b64: &str) -> Vec<u8> {
        STANDARD.decode(b64).unwrap()
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    /// The vector's caller.
    fn nelly() -> HubCaller {
        HubCaller {
            discord_id: "80351110224678912".into(),
            act: Act::Reveal,
            jti: "AAECAwQFBgcICQoLDA0ODw".into(),
        }
    }

    /// The vector's claims as a JSON object, to change one at a time.
    fn claims() -> serde_json::Map<String, Value> {
        serde_json::from_str(VECTOR_CLAIMS).unwrap()
    }

    /// A token for `claims` exactly as decision 15 builds one, under the
    /// vector's key.
    fn mint(claims: &serde_json::Map<String, Value>) -> String {
        mint_raw(&serde_json::to_vec(claims).unwrap(), &key(VECTOR_SECRET))
    }

    fn mint_raw(payload: &[u8], key: &[u8]) -> String {
        format!("v1.{}", crypto::sign(key, CONTEXT, payload))
    }

    /// `token` with its payload replaced by `payload` and MACed again over the
    /// new text, as a signer that encodes differently would.
    fn resigned(payload: &str) -> String {
        let mac = mac_over(payload);
        format!("v1.{payload}.{mac}")
    }

    /// The MAC decision 15 puts on `payload`'s text, base64url.
    fn mac_over(payload: &str) -> String {
        use hmac::{Hmac, Mac};
        let mut hmac = Hmac::<sha2::Sha256>::new_from_slice(&key(VECTOR_SECRET)).unwrap();
        hmac.update(CONTEXT.as_bytes());
        hmac.update(&[0]);
        hmac.update(payload.as_bytes());
        URL_SAFE_NO_PAD.encode(hmac.finalize().into_bytes())
    }

    fn parts(token: &str) -> (&str, &str) {
        let rest = token.strip_prefix("v1.").unwrap();
        rest.split_once('.').unwrap()
    }

    fn verifies_at(token: &str, now: u64) -> bool {
        verify(token, &vector_secret(), at(now)).is_some()
    }

    // --- The vector -------------------------------------------------------

    #[test]
    fn the_spec_test_vector_verifies() {
        assert_eq!(
            verify(VECTOR_TOKEN, &vector_secret(), at(1_795_000_030)),
            Some(nelly())
        );
    }

    /// The vector is these claims' bytes under this construction — so the
    /// hub's `mintToken` and this verifier agree on every byte, not just on
    /// the one token.
    #[test]
    fn the_test_vector_is_its_claims_signed_as_decision_15_says() {
        assert_eq!(
            mint_raw(VECTOR_CLAIMS.as_bytes(), &key(VECTOR_SECRET)),
            VECTOR_TOKEN
        );
        let (payload, mac) = parts(VECTOR_TOKEN);
        assert_eq!(
            URL_SAFE_NO_PAD.decode(payload).unwrap(),
            VECTOR_CLAIMS.as_bytes()
        );
        assert_eq!(mac, mac_over(payload));
    }

    /// `exp > now - 5 s`: the vector (exp 1795000060) holds through
    /// 1795000064 and is refused from 1795000065 on.
    #[test]
    fn the_vector_holds_until_exp_plus_the_leeway_and_no_longer() {
        assert!(verifies_at(VECTOR_TOKEN, VECTOR_EXP));
        assert!(verifies_at(VECTOR_TOKEN, VECTOR_EXP + 4));
        assert!(!verifies_at(VECTOR_TOKEN, VECTOR_EXP + 5));
        assert!(!verifies_at(VECTOR_TOKEN, VECTOR_EXP + 6));
        assert!(!verifies_at(VECTOR_TOKEN, VECTOR_EXP + 3600));
        // Sub-second: still refused half a second past the line.
        let late = at(VECTOR_EXP + 5) + Duration::from_millis(500);
        assert!(verify(VECTOR_TOKEN, &vector_secret(), late).is_none());
        let just_in = at(VECTOR_EXP + 4) + Duration::from_millis(999);
        assert!(verify(VECTOR_TOKEN, &vector_secret(), just_in).is_some());
    }

    /// `iat <= now + 5 s`: a hub clock up to five seconds ahead is tolerated,
    /// and no more.
    #[test]
    fn a_token_issued_up_to_the_leeway_ahead_is_accepted_and_no_further() {
        assert!(verifies_at(VECTOR_TOKEN, VECTOR_IAT));
        assert!(verifies_at(VECTOR_TOKEN, VECTOR_IAT - 5));
        assert!(!verifies_at(VECTOR_TOKEN, VECTOR_IAT - 6));
        assert!(!verifies_at(VECTOR_TOKEN, 0));
        let before_epoch = UNIX_EPOCH - Duration::from_secs(1);
        assert!(verify(VECTOR_TOKEN, &vector_secret(), before_epoch).is_none());
    }

    // --- The secret -------------------------------------------------------

    #[test]
    fn a_mac_under_previous_is_accepted() {
        let rotated = secret(OTHER_SECRET, Some(VECTOR_SECRET));
        assert_eq!(
            verify(VECTOR_TOKEN, &rotated, at(1_795_000_030)),
            Some(nelly())
        );
    }

    /// Review Focus 1: a token MACed with another secret.
    #[test]
    fn a_mac_under_another_secret_is_refused() {
        let now = at(1_795_000_030);
        assert!(verify(VECTOR_TOKEN, &secret(OTHER_SECRET, None), now).is_none());
        let a_third = STANDARD.encode([0x55u8; 32]);
        assert!(verify(VECTOR_TOKEN, &secret(OTHER_SECRET, Some(&a_third)), now).is_none());

        // The vector's claims MACed under the other key, presented to the
        // vector's secret.
        let forged = mint_raw(VECTOR_CLAIMS.as_bytes(), &key(OTHER_SECRET));
        assert!(!verifies_at(&forged, 1_795_000_030));
    }

    /// The same key and claims under another context (the portal's session
    /// cookie's, say) is not a hub token.
    #[test]
    fn a_mac_under_another_context_is_refused() {
        let session = format!(
            "v1.{}",
            crypto::sign(
                &key(VECTOR_SECRET),
                crypto::CTX_SESSION,
                VECTOR_CLAIMS.as_bytes()
            )
        );
        assert!(!verifies_at(&session, 1_795_000_030));
    }

    // --- The format -------------------------------------------------------

    /// Review Focus 1: a version other than `v1`.
    #[test]
    fn a_version_other_than_v1_is_refused() {
        let rest = VECTOR_TOKEN.strip_prefix("v1.").unwrap();
        for token in [
            format!("v2.{rest}"),
            format!("V1.{rest}"),
            format!("v10.{rest}"),
            format!("v1{rest}"),
            format!(" v1.{rest}"),
            format!(".{rest}"),
            rest.to_string(),
        ] {
            assert!(!verifies_at(&token, 1_795_000_030), "{token}");
        }
    }

    /// Review Focus 1: a truncated token — every proper prefix of the vector,
    /// and the vector with anything after it.
    #[test]
    fn a_truncated_or_extended_token_is_refused() {
        for end in 0..VECTOR_TOKEN.len() {
            let cut = &VECTOR_TOKEN[..end];
            assert!(!verifies_at(cut, 1_795_000_030), "{cut}");
        }
        for tail in ["=", "==", ".", ".x", " ", "\n", "A"] {
            let token = format!("{VECTOR_TOKEN}{tail}");
            assert!(!verifies_at(&token, 1_795_000_030), "{token:?}");
        }
        // A part dropped from the middle.
        let (payload, mac) = parts(VECTOR_TOKEN);
        assert!(!verifies_at(&format!("v1.{payload}"), 1_795_000_030));
        assert!(!verifies_at(&format!("v1.{mac}"), 1_795_000_030));
        assert!(!verifies_at(&format!("v1..{mac}"), 1_795_000_030));
        assert!(!verifies_at(&format!("v1.{payload}."), 1_795_000_030));
    }

    /// Review Focus 1: a re-encoded payload. The MAC is over the payload's
    /// TEXT, so another spelling of the same claims fails it; and a signer
    /// that MACs another spelling is still refused, because base64url without
    /// padding is the only encoding a token has.
    #[test]
    fn a_re_encoded_payload_is_refused() {
        let (payload, mac) = parts(VECTOR_TOKEN);
        let now = 1_795_000_030;

        // Padded: under the vector's MAC, and MACed again.
        let padded = URL_SAFE.encode(VECTOR_CLAIMS);
        assert_ne!(
            padded, payload,
            "the vector's payload needs padding to test this"
        );
        assert!(!verifies_at(&format!("v1.{padded}.{mac}"), now));
        assert!(!verifies_at(&resigned(&padded), now));

        // Standard alphabet: claims whose base64url has a `-` or `_`, so the
        // standard spelling (`+`, `/`) differs.
        let mut odd = claims();
        odd.insert("name".into(), json!("n?>~?>~?>~"));
        let json = serde_json::to_vec(&odd).unwrap();
        let url = URL_SAFE_NO_PAD.encode(&json);
        let standard = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&json);
        assert_ne!(
            url, standard,
            "the claims must spell differently to test this"
        );
        assert!(
            verifies_at(&resigned(&url), now),
            "the url-safe spelling is the token"
        );
        assert!(!verifies_at(&resigned(&standard), now));
        assert!(!verifies_at(&resigned(&STANDARD.encode(&json)), now));
    }

    /// The MAC's own encoding is base64url without padding, and nothing else.
    #[test]
    fn a_re_encoded_mac_is_refused() {
        let (payload, mac) = parts(VECTOR_TOKEN);
        let bytes = URL_SAFE_NO_PAD.decode(mac).unwrap();
        for other in [
            STANDARD.encode(&bytes),
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(&bytes),
            URL_SAFE.encode(&bytes),
        ] {
            assert_ne!(other, mac);
            assert!(
                !verifies_at(&format!("v1.{payload}.{other}"), 1_795_000_030),
                "{other}"
            );
        }
    }

    #[test]
    fn malformed_tokens_are_refused_rather_than_panicking() {
        let long = "A".repeat(100_000);
        for bad in [
            "",
            ".",
            "..",
            "...",
            "v1",
            "v1.",
            "v1..",
            "v1...",
            "v1.a.b",
            "v1.!!!.???",
            "v1.ż.ł",
            "v1.\u{0}.\u{0}",
            &format!("v1.{long}.{long}"),
        ] {
            assert!(!verifies_at(bad, 1_795_000_030), "accepted {bad:?}");
        }
    }

    // --- The claims -------------------------------------------------------

    /// Review Focus 1: a too-long-lived token. `exp - iat <= 60 s`, checked
    /// on the token itself, whatever the clock says.
    #[test]
    fn a_token_living_longer_than_sixty_seconds_is_refused() {
        let now = VECTOR_IAT;
        for (lifetime, accepted) in [
            (60, true),
            (59, true),
            (0, true),
            (61, false),
            (3600, false),
        ] {
            let mut c = claims();
            c.insert("exp".into(), json!(VECTOR_IAT + lifetime));
            assert_eq!(verifies_at(&mint(&c), now), accepted, "lifetime {lifetime}");
        }
        // An `exp` before its `iat` is no lifetime at all.
        let mut c = claims();
        c.insert("exp".into(), json!(VECTOR_IAT - 1));
        assert!(!verifies_at(&mint(&c), VECTOR_IAT - 3));
    }

    /// Review Focus 1: an expired token, minted fresh rather than aged.
    #[test]
    fn an_expired_token_is_refused() {
        let mut c = claims();
        c.insert("iat".into(), json!(VECTOR_IAT - 120));
        c.insert("exp".into(), json!(VECTOR_IAT - 60));
        assert!(!verifies_at(&mint(&c), VECTOR_IAT));
    }

    #[test]
    fn a_sub_that_is_not_discord_and_a_snowflake_is_refused() {
        for sub in [
            "80351110224678912",
            "discord:",
            "discord:abc",
            "discord:8035111022467891a",
            "discord: 80351110224678912",
            "discord:80351110224678912 ",
            "discord:-1",
            "discord:+80351110224678912",
            "discord:99999999999999999999",
            "Discord:80351110224678912",
            "github:80351110224678912",
            "discord:80351110224678912:x",
        ] {
            let mut c = claims();
            c.insert("sub".into(), json!(sub));
            assert!(!verifies_at(&mint(&c), 1_795_000_030), "{sub:?}");
        }
        let mut c = claims();
        c.insert("sub".into(), json!(42));
        assert!(!verifies_at(&mint(&c), 1_795_000_030));
    }

    /// A snowflake is written one way: `0` alone or a leading `0` is another
    /// spelling of an id (or of none), and would name another key
    /// (`discord-0<id>-key`) for the same account.
    #[test]
    fn a_sub_that_is_not_a_canonical_snowflake_is_refused() {
        for sub in [
            "discord:0",
            "discord:00",
            "discord:080351110224678912",
            "discord:0080351110224678912",
        ] {
            let mut c = claims();
            c.insert("sub".into(), json!(sub));
            assert!(!verifies_at(&mint(&c), 1_795_000_030), "{sub:?}");
        }
        // The smallest and largest canonical ones still are.
        for sub in ["discord:1", "discord:18446744073709551615"] {
            let mut c = claims();
            c.insert("sub".into(), json!(sub));
            assert!(verifies_at(&mint(&c), 1_795_000_030), "{sub:?}");
        }
    }

    #[test]
    fn a_wrong_iss_or_aud_is_refused() {
        for (claim, value) in [
            ("iss", json!("https://sorobanscan.com/api/hub/")),
            ("iss", json!("http://sorobanscan.com/api/hub")),
            ("iss", json!("https://evil.example/api/hub")),
            ("iss", json!("")),
            ("aud", json!("x402")),
            ("aud", json!("Prices")),
            ("aud", json!(["prices"])),
            ("aud", json!("")),
        ] {
            let mut c = claims();
            c.insert(claim.into(), value.clone());
            assert!(!verifies_at(&mint(&c), 1_795_000_030), "{claim} = {value}");
        }
    }

    #[test]
    fn each_act_is_read_and_nothing_else_is() {
        for (text, act) in [
            ("read", Act::Read),
            ("reveal", Act::Reveal),
            ("revoke", Act::Revoke),
            ("issue", Act::Issue),
        ] {
            let mut c = claims();
            c.insert("act".into(), json!(text));
            let caller = verify(&mint(&c), &vector_secret(), at(1_795_000_030)).unwrap();
            assert_eq!(caller.act, act);
            assert_eq!(act.as_str(), text);
        }
        for text in [
            json!("admin"),
            json!("READ"),
            json!(""),
            json!(null),
            json!(1),
        ] {
            let mut c = claims();
            c.insert("act".into(), text.clone());
            assert!(!verifies_at(&mint(&c), 1_795_000_030), "{text}");
        }
    }

    /// Every claim the spec names is required; a mistyped one is a refusal,
    /// not a default.
    #[test]
    fn a_missing_or_mistyped_claim_is_refused() {
        for claim in ["iss", "aud", "sub", "name", "iat", "exp", "jti", "act"] {
            let mut c = claims();
            c.remove(claim);
            assert!(!verifies_at(&mint(&c), 1_795_000_030), "without {claim}");
        }
        for (claim, value) in [
            ("iat", json!("1795000000")),
            ("iat", json!(1_795_000_000.0)),
            ("iat", json!(-1)),
            ("exp", json!(null)),
            ("jti", json!(1)),
            ("name", json!(null)),
        ] {
            let mut c = claims();
            c.insert(claim.into(), value.clone());
            assert!(!verifies_at(&mint(&c), 1_795_000_030), "{claim} = {value}");
        }
    }

    /// A claim the spec does not name is ignored, as a JWT's would be.
    #[test]
    fn an_extra_claim_is_ignored() {
        let mut c = claims();
        c.insert("nbf".into(), json!(0));
        assert_eq!(
            verify(&mint(&c), &vector_secret(), at(1_795_000_030)),
            Some(nelly())
        );
    }

    #[test]
    fn a_signed_payload_that_is_not_the_claims_object_is_refused() {
        let duplicated =
            VECTOR_CLAIMS.replace(r#""act":"reveal"}"#, r#""act":"reveal","act":"issue"}"#);
        for payload in ["", "not json", "[]", "null", "{}", &duplicated] {
            let token = mint_raw(payload.as_bytes(), &key(VECTOR_SECRET));
            assert!(!verifies_at(&token, 1_795_000_030), "{payload:?}");
        }
    }

    /// The caller is logged by `jti` and `act`; its Discord id never.
    #[test]
    fn a_callers_debug_never_shows_its_discord_id() {
        let printed = format!("{:?}", nelly());
        assert!(!printed.contains("80351110224678912"), "{printed}");
        assert!(printed.contains("AAECAwQFBgcICQoLDA0ODw"), "{printed}");
    }
}
