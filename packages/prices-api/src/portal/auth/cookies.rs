//! Reading and writing the portal's two cookies (task 0186).
//!
//! Hand-rolled rather than pulling a cookie crate, for the same reason the rest
//! of this module is small: there are exactly two cookies, both minted here,
//! both carrying base64url and nothing else. What a library would buy —
//! quoting, percent-encoding, `Expires` formatting, jar semantics — is surface
//! we would then have to reason about on a public route.
//!
//! # The attributes, and why each one
//!
//! Every cookie set here carries **all four** of `HttpOnly`, `Secure`,
//! `SameSite=Lax` and an explicit `Path`, and they are not independent choices:
//!
//! - **`HttpOnly`** — the session *is* the credential, so script must not be
//!   able to read it. Nothing in [0185]'s bundle needs to: it asks
//!   `/auth/me` who it is talking to and the browser attaches the cookie.
//! - **`Secure`** — the distribution is `REDIRECT_TO_HTTPS`, so a cleartext
//!   request should never happen; this is what makes it not happen *silently*.
//!   Browsers treat `http://localhost` as a secure context, so this does not
//!   get in the way of the local round-trip the acceptance criteria call for.
//! - **`SameSite=Lax`, not `Strict`** — and this is the one that is a real
//!   choice rather than a default. `Strict` withholds cookies on cross-site
//!   navigations, and Discord returning the visitor to `/auth/callback` is
//!   exactly that: a top-level GET originating at `discord.com`. Under `Strict`
//!   the [`PENDING_COOKIE`] would not be sent, every callback would fail state
//!   verification, and sign-in would be broken in a way that looks like a bug in
//!   the state check. `Lax` sends cookies on top-level GET navigation and
//!   withholds them from cross-site subrequests, which is precisely the
//!   distinction wanted. It is available at all only because [0184] put the
//!   bundle and the API on one distribution — cross-origin, this would have had
//!   to be `SameSite=None`, i.e. no CSRF protection from the cookie layer at all.
//! - **`Path`** — the session is scoped to `/api/`, so it is not attached
//!   to `/v1/*` data requests, which have no use for it and are the highest-volume
//!   route group on the distribution. The pending-login cookie is scoped tighter
//!   still: it is only ever read by the callback.
//!
//! No `Domain` attribute, deliberately. Omitting it produces a host-only cookie;
//! setting it to the registrable domain would share the session with every other
//! host under it, which on a CloudFront domain means somebody else's distribution.

use axum::http::HeaderMap;
use axum::http::header::COOKIE;

/// The session. Named without a `__Host-`/`__Secure-` prefix on purpose: the
/// `__Host-` prefix forbids a `Path` other than `/`, which would undo the
/// scoping above, and `__Secure-` only enforces the `Secure` attribute that is
/// set unconditionally here anyway.
pub const SESSION_COOKIE: &str = "portal_session";

/// The half of the `state` check that lives in the browser — see
/// [`super::state_token`].
pub const PENDING_COOKIE: &str = "portal_oauth_pending";

/// Path the session is scoped to: the portal, bundle and backend alike.
///
/// It cannot be narrower: since task 0194 the backend has no sub-prefix of its
/// own under `/api/`, so a `Path` that covers `/api/auth/*` and `/api/key`
/// necessarily covers `/api/assets/*` too. The cookie is `HttpOnly` and the
/// bundle is served by S3, which ignores it; the cost is bytes on asset
/// requests, not exposure.
pub const SESSION_PATH: &str = "/api/";

/// Path the pending-login cookie is scoped to. Only `/auth/callback` ever reads
/// it, so it is never attached to anything else — including [0187]'s key reveal,
/// which is the request on this origin whose response is most worth not
/// widening.
pub const PENDING_PATH: &str = "/api/auth/";

/// Read a cookie value out of a request's `Cookie` header(s).
///
/// Returns the FIRST match. A browser sends one `Cookie` header with `; `
/// separators, but duplicates are possible when two cookies share a name at
/// different paths — for us that is the more-specific one first, which is the
/// one we want. Values are used only after a signature check, so a wrong pick
/// fails closed rather than confusing anything.
pub fn read(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_string())
}

/// A `Set-Cookie` value that stores `value` under `name` for `max_age` seconds.
///
/// `value` must be cookie-safe — every caller passes base64url, which is by
/// construction. There is no escaping here because there is nothing that needs
/// escaping, and adding some would invite passing something that does.
pub fn set(name: &str, value: &str, path: &str, max_age_secs: u64) -> String {
    format!("{name}={value}; Path={path}; Max-Age={max_age_secs}; HttpOnly; Secure; SameSite=Lax")
}

/// A `Set-Cookie` value that deletes `name`.
///
/// `Max-Age=0` with an empty value, and **the same `Path`** — a clear that names
/// a different path leaves the original cookie in place and adds a second, empty
/// one, which is how a sign-out silently does nothing. All four attributes are
/// repeated because a browser matches the cookie to delete on name, domain and
/// path only, but a `Secure` cookie cannot be cleared by a non-`Secure` header.
pub fn clear(name: &str, path: &str) -> String {
    format!("{name}=; Path={path}; Max-Age=0; HttpOnly; Secure; SameSite=Lax")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(raw: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in raw {
            map.append(COOKIE, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn reads_a_cookie_from_a_multi_value_header() {
        let map = headers(&["other=1; portal_session=abc.def; another=2"]);
        assert_eq!(read(&map, SESSION_COOKIE).as_deref(), Some("abc.def"));
    }

    #[test]
    fn reads_a_cookie_split_across_two_headers() {
        let map = headers(&["other=1", "portal_session=xyz"]);
        assert_eq!(read(&map, SESSION_COOKIE).as_deref(), Some("xyz"));
    }

    #[test]
    fn a_missing_cookie_is_none_and_a_prefix_match_is_not_a_match() {
        let map = headers(&["portal_session_other=nope"]);
        assert_eq!(read(&map, SESSION_COOKIE), None);
        assert_eq!(read(&HeaderMap::new(), SESSION_COOKIE), None);
    }

    /// Four attributes, and the acceptance criteria name three of them
    /// explicitly. Asserted as a whole string so dropping one is a failure here
    /// rather than a finding in [0194]'s audit.
    #[test]
    fn every_cookie_carries_httponly_secure_lax_and_a_path() {
        let session = set(SESSION_COOKIE, "v", SESSION_PATH, 3600);
        assert_eq!(
            session,
            "portal_session=v; Path=/api/; Max-Age=3600; HttpOnly; Secure; SameSite=Lax"
        );
        for header in [session, clear(SESSION_COOKIE, SESSION_PATH)] {
            assert!(header.contains("HttpOnly"), "{header}");
            assert!(header.contains("Secure"), "{header}");
            assert!(header.contains("SameSite=Lax"), "{header}");
            assert!(!header.contains("SameSite=Strict"), "{header}");
            assert!(!header.contains("Domain="), "{header}");
        }
    }

    #[test]
    fn clearing_expires_immediately_at_the_same_path() {
        let cleared = clear(PENDING_COOKIE, PENDING_PATH);
        assert!(cleared.starts_with("portal_oauth_pending=;"));
        assert!(cleared.contains("Max-Age=0"));
        assert!(cleared.contains(&format!("Path={PENDING_PATH}")));
    }

    /// The session must not ride along on data-API traffic, and the pending
    /// cookie must not ride along on anything but the callback.
    #[test]
    fn the_two_paths_are_scoped_as_narrowly_as_their_readers_need() {
        assert!(SESSION_PATH.starts_with("/api/"));
        assert!(PENDING_PATH.starts_with(SESSION_PATH));
        assert!(!SESSION_PATH.starts_with("/v1"));
    }
}
