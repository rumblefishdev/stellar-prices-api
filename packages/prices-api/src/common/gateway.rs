//! The API Gateway resource a request came through (task 0357).
//!
//! The gateway picks a resource, and with it whether an API key is asked for,
//! on the path as it was sent. `lambda_http` then resolves dot segments (`..`,
//! `%2e%2e`) before the router sees the path. So
//! `/api/../v1/assets/native/price` came through the keyless `/api/{proxy+}`
//! and was routed as the keyed `/v1/assets/native/price`, answered with no key.
//! The Lambda refuses a request whose routed path has left its resource.

/// Whether `path`, as the router sees it, lies outside `resource`, the gateway
/// resource the request came through (`/api/{proxy+}`,
/// `/v1/assets/{asset_identifier}/price`). Only the resource's literal part,
/// up to its first `{`, is compared: a dot segment can move a path out of it,
/// and nothing else the gateway lets through can.
pub fn left_its_resource(resource: &str, path: &str) -> bool {
    let literal = resource.split('{').next().unwrap_or_default();
    !path.starts_with(literal)
}

#[cfg(test)]
mod tests {
    use super::left_its_resource;

    #[test]
    fn a_path_that_left_a_keyless_resource_is_caught() {
        // `/api/../v1/assets/native/price` and `/x402/%2e%2e/v1/...` as routed.
        assert!(left_its_resource(
            "/api/{proxy+}",
            "/v1/assets/native/price"
        ));
        assert!(left_its_resource("/x402/{proxy+}", "/v1/prices/batch"));
        assert!(left_its_resource("/api-docs-json", "/v1/assets"));
    }

    #[test]
    fn a_path_inside_its_resource_passes() {
        assert!(!left_its_resource("/api/{proxy+}", "/api/keys"));
        assert!(!left_its_resource(
            "/x402/{proxy+}",
            "/x402/v1/assets/native/price"
        ));
        assert!(!left_its_resource("/{proxy+}", "/v1/assets/native/price"));
        assert!(!left_its_resource(
            "/v1/assets/{asset_identifier}/price",
            "/v1/assets/USDC%3AGA5Z/price"
        ));
        assert!(!left_its_resource("/health", "/health"));
        assert!(!left_its_resource("/", "/"));
    }
}
