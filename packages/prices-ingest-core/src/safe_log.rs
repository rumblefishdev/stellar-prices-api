//! Error redaction for log emission.
//!
//! Mirrors BE's `safe_error_message` / `safe_bad_response_token`
//! (`crates/indexer/src/handler/mod.rs:436-485`).
//!
//! Logging policy: NEVER stringify an upstream error whose `Display`
//! could embed row data (ClickHouse `BadResponse` is the canonical
//! example — its body echoes offending row values into the message).
//! Emit fixed labels plus, for HTTP/CH responses, only the leading
//! `Code: NNN` or HTTP status token.
//!
//! This lives in the shared core (not the Lambda crate) so the redaction
//! is applied at the *source*: [`IngestError`](crate::IngestError)'s own
//! `Display` routes the ClickHouse variant through here, so every consumer
//! of the shared [`OhlcvWriter`](crate::OhlcvWriter) — the live Lambda
//! *and* the SDEX backfill — is leak-safe without each re-implementing it.

/// Extract ONLY the leading code/status token from a wire-error body.
/// Returns `"Code: NNN"` for a CH exception body, `"HTTP NNN"` for a
/// plain HTTP status line, or `"detail suppressed"` for anything else
/// where we cannot prove the remainder is data-free.
pub fn safe_response_token(msg: &str) -> String {
    if let Some(rest) = msg.strip_prefix("Code: ") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            return format!("Code: {digits}");
        }
    }
    let leading: String = msg.chars().take_while(char::is_ascii_digit).collect();
    if leading.len() == 3 {
        return format!("HTTP {leading}");
    }
    "detail suppressed".to_string()
}

/// Redact a ClickHouse client error into a leak-safe label. Used by
/// [`IngestError`](crate::IngestError)'s `Display` so the offending-row
/// body a `BadResponse` echoes never reaches a log line.
///
/// Matches on the *variant*, never on `err.to_string()`: the crate's `Display`
/// prefixes each variant with a label (`BadResponse` renders as
/// `"bad response: Code: NNN. ..."`), so token-scanning the rendered string
/// never matches a leading `Code: ` and collapses every server exception to
/// `"detail suppressed"` — throwing away the one token this is meant to keep.
pub fn redact_clickhouse(err: &clickhouse::error::Error) -> String {
    use clickhouse::error::Error as E;
    match err {
        // The server's response body — the only place row data can appear.
        // Keep just the leading code token.
        E::BadResponse(body) => safe_response_token(body),
        // Transport-level. The inner error can carry the request URL (which may
        // embed credentials), so emit a fixed label, not its `Display`. Still
        // distinguishes "never reached the server" from "server said no".
        E::Network(_) => "network error".to_string(),
        E::Compression(_) => "compression error".to_string(),
        E::Decompression(_) => "decompression error".to_string(),
        E::InvalidParams(_) => "invalid params".to_string(),
        // Crate-fixed, data-free messages — safe to surface verbatim.
        E::RowNotFound
        | E::SequenceMustHaveLength
        | E::DeserializeAnyNotSupported
        | E::NotEnoughData
        | E::InvalidUtf8Encoding(_)
        | E::TimedOut => err.to_string(),
        // `Custom`/`Other`/`Unsupported` can embed serde field values or
        // arbitrary upstream text; the enum is also `#[non_exhaustive]`, so
        // anything new defaults to suppressed.
        _ => "detail suppressed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ch_exception_extracts_code() {
        assert_eq!(
            safe_response_token("Code: 241. DB::Exception: foo bar=12345"),
            "Code: 241"
        );
    }

    #[test]
    fn http_status_extracts_three_digits() {
        assert_eq!(
            safe_response_token("503 Service Unavailable: backend timeout"),
            "HTTP 503"
        );
    }

    #[test]
    fn proxy_html_suppresses_everything() {
        assert_eq!(
            safe_response_token("<html><body>Bad Gateway</body></html>"),
            "detail suppressed"
        );
    }

    #[test]
    fn malformed_code_prefix_suppresses() {
        assert_eq!(safe_response_token("Code: abc."), "detail suppressed");
    }

    // The tests above feed `safe_response_token` a RAW body — which is not what
    // `redact_clickhouse` passes it in production. These drive the real types.

    #[test]
    fn bad_response_keeps_code_despite_crate_display_prefix() {
        let err = clickhouse::error::Error::BadResponse(
            "Code: 516. DB::Exception: default: Authentication failed".into(),
        );
        // Guard the premise: the crate's Display is what broke the old scan.
        assert!(err.to_string().starts_with("bad response: "));
        assert_eq!(redact_clickhouse(&err), "Code: 516");
    }

    #[test]
    fn bad_response_body_never_leaks_row_data() {
        let err = clickhouse::error::Error::BadResponse(
            "Code: 241. DB::Exception: Memory limit exceeded, row asset=SECRET".into(),
        );
        let out = redact_clickhouse(&err);
        assert_eq!(out, "Code: 241");
        assert!(!out.contains("SECRET"));
    }

    #[test]
    fn network_error_is_labelled_and_drops_inner_url() {
        let err = clickhouse::error::Error::Network(Box::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "http://user:hunter2@host:8123/",
        )));
        let out = redact_clickhouse(&err);
        assert_eq!(out, "network error");
        assert!(!out.contains("hunter2"));
    }

    #[test]
    fn data_free_variant_surfaces_verbatim() {
        assert_eq!(
            redact_clickhouse(&clickhouse::error::Error::TimedOut),
            "timeout expired"
        );
    }

    #[test]
    fn custom_variant_stays_suppressed() {
        let err = clickhouse::error::Error::Custom("field balance=12345 mismatched".into());
        assert_eq!(redact_clickhouse(&err), "detail suppressed");
    }

    /// End-to-end through the type operators actually see. This is the case the
    /// original suite missed: a real prod auth failure logged as
    /// `"ingest: clickhouse: detail suppressed"`, hiding `Code: 516`.
    #[test]
    fn ingest_error_display_surfaces_the_code() {
        let err = crate::IngestError::Clickhouse(clickhouse::error::Error::BadResponse(
            "Code: 516. DB::Exception: default: Authentication failed".into(),
        ));
        assert_eq!(err.to_string(), "clickhouse: Code: 516");
    }
}
