//! The three primitives the portal's sign-in is built from: OS randomness,
//! keyed signatures over short JSON payloads, and the PKCE challenge derivation
//! (task 0186).
//!
//! Nothing here is portal-specific beyond the choice of encoding. It is a
//! separate module because it is the part where a mistake is silent: a signature
//! compared with `==`, a nonce from a userspace PRNG, or two differently-shaped
//! payloads accepted by the same verifier all still pass every functional test.
//!
//! # Why one key signs three different things
//!
//! The session cookie, the `state` parameter and the cookie that binds `state`
//! to the browser are all authenticated with the same HMAC key — the operator
//! provisions one, not three. That is only safe with **domain separation**: the
//! MAC is taken over `context || 0x00 || payload`, so a value minted as one kind
//! of token cannot be presented as another. Without it, a `state` parameter —
//! which the holder sees, because it travels through their browser's address bar
//! — would verify as a session cookie, and sign-in would be a self-service
//! session forgery. The separator is a NUL because none of the three context
//! strings can contain one, so the concatenation is unambiguous.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::auth::ct_eq;

type HmacSha256 = Hmac<Sha256>;

/// Domain separator for the `state` parameter that travels to Discord.
pub const CTX_STATE: &str = "portal-oauth-state-v1";
/// Domain separator for the cookie that binds a `state` to this browser.
pub const CTX_PENDING: &str = "portal-oauth-pending-v1";
/// Domain separator for the session cookie.
pub const CTX_SESSION: &str = "portal-session-v1";

/// Base64url, no padding — the encoding every token here uses.
///
/// Unpadded on purpose: these values land in a `Set-Cookie` header and in a
/// query string, and `=` is meaningful in both.
pub fn b64_encode(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

/// Inverse of [`b64_encode`]. `None` on anything that is not valid base64url —
/// which for every caller here means "not a token we minted".
pub fn b64_decode(text: &str) -> Option<Vec<u8>> {
    B64.decode(text).ok()
}

/// `len` bytes of OS entropy, base64url-encoded.
///
/// Panics if the OS cannot supply randomness. That is the right response and not
/// laziness: every caller is minting a value whose unguessability is the entire
/// security property (a CSRF nonce, a PKCE verifier), and there is no degraded
/// mode in which continuing is better than not starting. The same stance as
/// `lib.rs` taking on the OpenAPI document.
pub fn random_token(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("the OS must be able to supply randomness");
    b64_encode(&bytes)
}

/// Sign `payload` under `context`, returning `<payload>.<mac>`, both base64url.
///
/// The MAC covers the *encoded* payload rather than the raw bytes so that
/// verification never has to decode attacker-supplied input before it has
/// authenticated it.
pub fn sign(key: &[u8], context: &str, payload: &[u8]) -> String {
    let encoded = b64_encode(payload);
    let mac = mac(key, context, encoded.as_bytes());
    format!("{encoded}.{}", b64_encode(&mac))
}

/// Verify a token produced by [`sign`] under the same `context`, returning the
/// payload bytes.
///
/// `None` covers every rejection — malformed, wrong context, wrong key, altered
/// payload — with no distinction, because the caller has nothing useful to do
/// with the difference and telling them apart is a probing oracle.
pub fn verify(key: &[u8], context: &str, token: &str) -> Option<Vec<u8>> {
    let (encoded, signature) = token.split_once('.')?;
    let presented = b64_decode(signature)?;
    let expected = mac(key, context, encoded.as_bytes());
    // Constant time, via the same helper the `X-API-Key` gate uses. `==` on two
    // byte slices short-circuits at the first difference, which leaks how much
    // of a forged MAC was right — enough, over enough attempts, to build one.
    if !ct_eq(&expected, &presented) {
        return None;
    }
    b64_decode(encoded)
}

/// The RFC 7636 S256 code challenge for a verifier: base64url(SHA-256(verifier)).
///
/// Discord's OAuth2 documentation never mentions PKCE (recorded by task 0156),
/// and for a confidential client with an HTTPS redirect it is not documented as
/// required. We send it anyway — it costs one hash and it is the difference
/// between an intercepted authorization code being usable and being inert.
pub fn pkce_challenge(verifier: &str) -> String {
    b64_encode(&Sha256::digest(verifier.as_bytes()))
}

fn mac(key: &[u8], context: &str, message: &[u8]) -> Vec<u8> {
    // `new_from_slice` on HMAC accepts any key length; the error variant is
    // unreachable for `Hmac`, which is why the type's own docs recommend this
    // call over `new`. Length is enforced where the key is loaded instead
    // (`secret.rs` refuses a short one), which is the place that can produce a
    // useful message about it.
    let mut hmac = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    hmac.update(context.as_bytes());
    hmac.update(&[0u8]);
    hmac.update(message);
    hmac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"a-test-signing-key-of-adequate-length";

    #[test]
    fn a_signed_payload_round_trips() {
        let token = sign(KEY, CTX_SESSION, b"hello");
        assert_eq!(
            verify(KEY, CTX_SESSION, &token).as_deref(),
            Some(&b"hello"[..])
        );
    }

    #[test]
    fn a_tampered_payload_is_rejected() {
        let token = sign(KEY, CTX_SESSION, b"user-1");
        let forged = format!(
            "{}.{}",
            b64_encode(b"user-2"),
            token.split_once('.').unwrap().1
        );
        assert!(verify(KEY, CTX_SESSION, &forged).is_none());
    }

    #[test]
    fn a_different_key_is_rejected() {
        let token = sign(KEY, CTX_SESSION, b"hello");
        assert!(verify(b"another-key-entirely", CTX_SESSION, &token).is_none());
    }

    /// The property the whole one-key design rests on. A `state` parameter is
    /// handed to the visitor's browser in a redirect URL, so if it verified as a
    /// session the flow would issue sessions to anyone who called `/auth/login`
    /// and read their own address bar.
    #[test]
    fn a_token_minted_for_one_context_does_not_verify_under_another() {
        let state = sign(KEY, CTX_STATE, b"payload");
        assert!(verify(KEY, CTX_STATE, &state).is_some());
        assert!(verify(KEY, CTX_SESSION, &state).is_none());
        assert!(verify(KEY, CTX_PENDING, &state).is_none());
    }

    #[test]
    fn malformed_tokens_are_rejected_rather_than_panicking() {
        for bad in ["", ".", "no-dot", "!!!.???", "aGk.", ".aGk"] {
            assert!(verify(KEY, CTX_SESSION, bad).is_none(), "accepted {bad:?}");
        }
    }

    #[test]
    fn random_tokens_are_the_requested_size_and_do_not_repeat() {
        let a = random_token(32);
        let b = random_token(32);
        assert_ne!(a, b);
        assert_eq!(b64_decode(&a).unwrap().len(), 32);
    }

    /// The RFC 7636 appendix B test vector, so a future encoding change (padded
    /// base64, hex, standard alphabet) fails here rather than at Discord.
    #[test]
    fn pkce_challenge_matches_the_rfc_7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
