use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

#[derive(Clone)]
pub(super) struct HttpSecurity {
    token: Arc<str>,
    port: u16,
}

impl HttpSecurity {
    pub(super) fn initialize(port: u16) -> Result<Self> {
        let token = load_or_create_token()?;
        std::env::set_var(qol_conventions::ENV_HTTP_TOKEN, token.as_ref());
        Ok(Self { token, port })
    }
}

pub(crate) fn browser_url(route: &str, port: u16) -> String {
    let token = current_token();
    qol_conventions::local_hash_url_with_token(route, port, token.as_deref())
}

pub(crate) fn current_token() -> Option<String> {
    load_token().ok()
}

pub(super) async fn require_local_host(
    State(security): State<HttpSecurity>,
    request: Request,
    next: Next,
) -> Response {
    if !host_is_allowed(request.headers(), security.port) {
        return (StatusCode::FORBIDDEN, "Local Host header required").into_response();
    }
    next.run(request).await
}

pub(super) async fn require_api_access(
    State(security): State<HttpSecurity>,
    request: Request,
    next: Next,
) -> Response {
    if !request_is_authorized(request.headers(), &security.token) {
        return (StatusCode::UNAUTHORIZED, "Authentication required").into_response();
    }
    if is_mutating_method(request.method())
        && (has_cross_site_fetch_metadata(request.headers())
            || has_untrusted_origin(request.headers(), security.port))
    {
        return (StatusCode::FORBIDDEN, "Cross-site request blocked").into_response();
    }
    next.run(request).await
}

fn load_or_create_token() -> Result<Arc<str>> {
    if let Ok(token) = load_token() {
        let path = crate::paths::http_auth_token_path()?;
        qol_fs::atomic_write_private(&path, token.as_bytes())
            .with_context(|| format!("failed to secure HTTP auth token at {}", path.display()))?;
        return Ok(Arc::from(token));
    }
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("failed to generate HTTP auth token: {error}"))?;
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let path = crate::paths::http_auth_token_path()?;
    qol_fs::atomic_write_private(&path, token.as_bytes())
        .with_context(|| format!("failed to persist HTTP auth token at {}", path.display()))?;
    Ok(Arc::from(token))
}

fn load_token() -> Result<String> {
    qol_plugin_api::host_exec::read_auth_token()
        .map_err(|error| anyhow::anyhow!("failed to load HTTP auth token: {error}"))
}

fn host_is_allowed(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = header_string(headers, header::HOST.as_str()) else {
        return false;
    };
    [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ]
    .iter()
    .any(|allowed| host.eq_ignore_ascii_case(allowed))
}

fn request_is_authorized(headers: &HeaderMap, token: &str) -> bool {
    if header_string(headers, qol_conventions::HTTP_AUTH_HEADER) == Some(token) {
        return true;
    }
    header_string(headers, header::COOKIE.as_str()).is_some_and(|cookies| {
        cookies.split(';').any(|cookie| {
            let Some((name, value)) = cookie.trim().split_once('=') else {
                return false;
            };
            name == qol_conventions::HTTP_AUTH_FRAGMENT_KEY && value == token
        })
    })
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
    !matches!(fetch_site, "same-origin" | "same-site" | "none")
}

fn has_untrusted_origin(headers: &HeaderMap, port: u16) -> bool {
    let Some(origin) = header_string(headers, header::ORIGIN.as_str()) else {
        return false;
    };
    ![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("http://[::1]:{port}"),
    ]
    .iter()
    .any(|allowed| origin.eq_ignore_ascii_case(allowed))
}

fn header_string<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    headers.get(key).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{HeaderValue, Request},
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    fn protected_app() -> Router {
        let security = HttpSecurity {
            token: Arc::from("secret"),
            port: qol_conventions::DEFAULT_PORT,
        };
        Router::new()
            .route(
                "/api/test",
                get(|| async { StatusCode::OK }).post(|| async { StatusCode::OK }),
            )
            .layer(middleware::from_fn_with_state(
                security.clone(),
                require_api_access,
            ))
            .layer(middleware::from_fn_with_state(security, require_local_host))
    }

    #[tokio::test]
    async fn middleware_requires_exact_host_and_token_and_blocks_cross_site_writes() {
        let port = qol_conventions::DEFAULT_PORT;
        let cases = [
            (
                Request::get("/api/test").body(Body::empty()).unwrap(),
                StatusCode::FORBIDDEN,
            ),
            (
                Request::get("/api/test")
                    .header(header::HOST, format!("localhost:{port}"))
                    .body(Body::empty())
                    .unwrap(),
                StatusCode::UNAUTHORIZED,
            ),
            (
                Request::get("/api/test")
                    .header(header::HOST, format!("localhost:{port}"))
                    .header(qol_conventions::HTTP_AUTH_HEADER, "secret")
                    .body(Body::empty())
                    .unwrap(),
                StatusCode::OK,
            ),
            (
                Request::get("/api/test")
                    .header(header::HOST, format!("evil.localhost:{port}"))
                    .header(qol_conventions::HTTP_AUTH_HEADER, "secret")
                    .body(Body::empty())
                    .unwrap(),
                StatusCode::FORBIDDEN,
            ),
            (
                Request::post("/api/test")
                    .header(header::HOST, format!("localhost:{port}"))
                    .header(qol_conventions::HTTP_AUTH_HEADER, "secret")
                    .header("sec-fetch-site", "cross-site")
                    .body(Body::empty())
                    .unwrap(),
                StatusCode::FORBIDDEN,
            ),
        ];

        for (request, expected) in cases {
            let response = protected_app().oneshot(request).await.unwrap();
            assert_eq!(response.status(), expected);
        }
    }

    #[test]
    fn host_requires_exact_local_name_and_port() {
        let port = qol_conventions::DEFAULT_PORT;
        let cases = [
            (format!("127.0.0.1:{port}"), true),
            (format!("localhost:{port}"), true),
            (format!("[::1]:{port}"), true),
            ("localhost".to_string(), false),
            (format!("evil.localhost:{port}"), false),
            ("127.0.0.1:80".to_string(), false),
        ];
        for (host, expected) in cases {
            let headers = headers_with(header::HOST.as_str(), &host);
            assert_eq!(host_is_allowed(&headers, port), expected, "host={host}");
        }
    }

    #[test]
    fn token_header_and_cookie_are_accepted() {
        let cases = [
            (qol_conventions::HTTP_AUTH_HEADER, "secret", true),
            (qol_conventions::HTTP_AUTH_HEADER, "wrong", false),
            (header::COOKIE.as_str(), "other=x; qol_token=secret", true),
            (header::COOKIE.as_str(), "qol_token=wrong", false),
        ];
        for (header_name, value, expected) in cases {
            let headers = headers_with(header_name, value);
            assert_eq!(
                request_is_authorized(&headers, "secret"),
                expected,
                "header={header_name} value={value}"
            );
        }
    }

    #[test]
    fn local_origin_uses_the_actual_bound_port() {
        let allowed = headers_with(header::ORIGIN.as_str(), "http://localhost:43210");
        let stale_origin = format!("http://localhost:{}", qol_conventions::DEFAULT_PORT);
        let stale = headers_with(header::ORIGIN.as_str(), &stale_origin);

        assert!(!has_untrusted_origin(&allowed, 43210));
        assert!(has_untrusted_origin(&stale, 43210));
    }

    #[test]
    fn fetch_metadata_blocks_cross_site_values() {
        let cases = [
            ("same-origin", false),
            ("same-site", false),
            ("none", false),
            ("cross-site", true),
            ("unexpected", true),
        ];
        for (value, expected) in cases {
            let headers = headers_with("sec-fetch-site", value);
            assert_eq!(
                has_cross_site_fetch_metadata(&headers),
                expected,
                "value={value}"
            );
        }
    }

    fn headers_with(key: &str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let name = header::HeaderName::from_bytes(key.as_bytes()).unwrap();
        headers.insert(name, HeaderValue::from_str(value).unwrap());
        headers
    }
}
