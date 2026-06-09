//! Error redaction wrappers — duplicated from 0038's prices-ledger-processor.
//! Consolidation into a shared `prices-common` crate is tracked as
//! follow-up work (see G-note Part A.3 / Issues Encountered).
//!
//! Policy: NEVER stringify an upstream error whose `Display` could
//! embed row data (ClickHouse `BadResponse` is the canonical
//! example). Emit fixed labels plus, for HTTP/CH responses, only
//! the leading `Code: NNN` or HTTP status token.

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
}
