use axum::{
    extract::Request,
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::types::DEFAULT_UI_SERVER_PORT;

pub(super) async fn reject_cross_site_mutations(request: Request, next: Next) -> Response {
    if !is_mutating_method(request.method()) {
        return next.run(request).await;
    }
    if has_cross_site_fetch_metadata(request.headers()) {
        return (StatusCode::FORBIDDEN, "Cross-site request blocked").into_response();
    }
    if has_untrusted_origin(request.headers()) {
        return (StatusCode::FORBIDDEN, "Cross-site request blocked").into_response();
    }
    next.run(request).await
}

fn is_mutating_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn has_cross_site_fetch_metadata(headers: &HeaderMap) -> bool {
    let Some(fetch_site) = header_string(headers, "sec-fetch-site") else {
        return false;
    };
    if fetch_site == "same-origin" {
        return false;
    }
    if fetch_site == "same-site" {
        return false;
    }
    if fetch_site == "none" {
        return false;
    }
    true
}

fn has_untrusted_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = header_string(headers, header::ORIGIN.as_str()) else {
        return false;
    };
    !is_allowed_local_origin(origin)
}

fn is_allowed_local_origin(origin: &str) -> bool {
    let port = DEFAULT_UI_SERVER_PORT;
    if origin == format!("http://127.0.0.1:{port}") {
        return true;
    }
    if origin == format!("http://localhost:{port}") {
        return true;
    }
    origin == format!("http://[::1]:{port}")
}

fn header_string<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    headers.get(key).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use proptest::prelude::*;

    #[test]
    fn allowed_local_origins_are_accepted() {
        let port = DEFAULT_UI_SERVER_PORT;
        assert!(is_allowed_local_origin(&format!("http://127.0.0.1:{port}")));
        assert!(is_allowed_local_origin(&format!("http://localhost:{port}")));
        assert!(is_allowed_local_origin(&format!("http://[::1]:{port}")));
    }

    #[test]
    fn non_local_origins_are_rejected() {
        assert!(!is_allowed_local_origin("https://example.com"));
        assert!(!is_allowed_local_origin("http://127.0.0.1:80"));
        assert!(!is_allowed_local_origin("http://evil.localhost:42700"));
    }

    #[test]
    fn is_mutating_method_only_allows_write_verbs() {
        let mutating = [Method::POST, Method::PUT, Method::PATCH, Method::DELETE];
        for method in mutating {
            assert!(is_mutating_method(&method));
        }

        let readonly = [
            Method::GET,
            Method::HEAD,
            Method::OPTIONS,
            Method::TRACE,
            Method::CONNECT,
        ];
        for method in readonly {
            assert!(!is_mutating_method(&method));
        }
    }

    #[test]
    fn has_cross_site_fetch_metadata_blocks_unknown_or_cross_site_values() {
        let cases = vec![
            ("same-origin", false),
            ("same-site", false),
            ("none", false),
            ("cross-site", true),
            ("unexpected", true),
            ("SAME-ORIGIN", true),
        ];

        for (value, expected) in cases {
            let headers = headers_with("sec-fetch-site", value);
            assert_eq!(has_cross_site_fetch_metadata(&headers), expected);
        }

        let headers = HeaderMap::new();
        assert!(!has_cross_site_fetch_metadata(&headers));
    }

    #[test]
    fn has_untrusted_origin_accepts_missing_and_local_only() {
        let headers = HeaderMap::new();
        assert!(!has_untrusted_origin(&headers));

        let port = DEFAULT_UI_SERVER_PORT;
        let allowed = vec![
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
            format!("http://[::1]:{port}"),
        ];
        for origin in allowed {
            let headers = headers_with(header::ORIGIN.as_str(), &origin);
            assert!(!has_untrusted_origin(&headers));
        }

        let blocked = vec![
            "https://example.com",
            "http://127.0.0.1:80",
            "http://localhost:9999",
            "http://evil.localhost:42700",
        ];
        for origin in blocked {
            let headers = headers_with(header::ORIGIN.as_str(), origin);
            assert!(has_untrusted_origin(&headers));
        }
    }

    fn headers_with(key: &str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let name = header::HeaderName::from_bytes(key.as_bytes()).unwrap();
        headers.insert(name, HeaderValue::from_str(value).unwrap());
        headers
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_unknown_fetch_site_values_are_blocked(value in "[a-z-]{1,24}") {
            prop_assume!(value != "same-origin");
            prop_assume!(value != "same-site");
            prop_assume!(value != "none");
            let headers = headers_with("sec-fetch-site", &value);
            prop_assert!(has_cross_site_fetch_metadata(&headers));
        }

        #[test]
        fn prop_only_exact_local_origins_are_allowed(origin in "[ -~]{0,80}") {
            let port = DEFAULT_UI_SERVER_PORT;
            let local_v4 = format!("http://127.0.0.1:{port}");
            let local_name = format!("http://localhost:{port}");
            let local_v6 = format!("http://[::1]:{port}");
            prop_assume!(origin != local_v4);
            prop_assume!(origin != local_name);
            prop_assume!(origin != local_v6);
            prop_assert!(!is_allowed_local_origin(&origin));
        }
    }
}
