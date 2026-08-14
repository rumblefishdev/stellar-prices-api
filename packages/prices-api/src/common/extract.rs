//! Wrapper extractors that answer in the canonical [`ErrorEnvelope`] voice.
//!
//! Axum's built-in `Query`/`Json`/`Path` reject *before* a handler runs, in
//! axum's own voice: `text/plain` bodies, and 415/422 for body failures. That
//! breaks the API contract (task 0119: every invalid input is a `400` with the
//! standard envelope), so handlers take these wrappers instead — same parsed
//! value, rejection routed through [`errors`].
//!
//! [`ErrorEnvelope`]: crate::common::errors::ErrorEnvelope

use axum::extract::{FromRequest, FromRequestParts, Json, Path, Query, Request};
use axum::http::request::Parts;
use axum::response::Response;
use serde::de::DeserializeOwned;

use crate::common::errors;

/// [`Query`] that rejects with `400` + `invalid_query` envelope (instead of
/// axum's `text/plain`). Covers non-numeric/overflowing values, duplicate
/// keys, and invalid percent-encoding in the query string.
pub struct ValidatedQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ValidatedQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(Self(value)),
            Err(rej) => Err(errors::bad_request(errors::INVALID_QUERY, rej.body_text())),
        }
    }
}

/// [`Path`] that rejects with `400` + `invalid_id` envelope. Only the
/// asset-identifier routes use path params, so a path-layer failure (bad
/// percent-encoding, invalid UTF-8) is by construction a malformed identifier.
pub struct ValidatedPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ValidatedPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(value)) => Ok(Self(value)),
            Err(rej) => Err(errors::bad_request(errors::INVALID_ID, rej.body_text())),
        }
    }
}

/// [`Json`] that rejects with `400` + `invalid_body` envelope — uniformly, for
/// every body-layer failure axum would otherwise scatter across statuses:
/// malformed JSON (400), wrong shape (422), missing `Content-Type` (415), and
/// an over-limit body (413).
pub struct ValidatedJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rej) => Err(errors::bad_request(errors::INVALID_BODY, rej.body_text())),
        }
    }
}
