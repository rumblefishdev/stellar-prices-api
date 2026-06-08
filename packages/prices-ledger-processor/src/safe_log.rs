//! Error redaction wrappers for log emission.
//!
//! Mirrors BE's `safe_error_message` / `safe_bad_response_token`
//! (`crates/indexer/src/handler/mod.rs:436-485`).
//!
//! Logging policy: NEVER stringify an upstream error whose `Display`
//! could embed row data (ClickHouse `BadResponse` is the canonical
//! example — its body echoes offending row values into the message).
//! Emit fixed labels plus, for HTTP/CH responses, only the leading
//! `Code: NNN` or HTTP status token.

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
}
