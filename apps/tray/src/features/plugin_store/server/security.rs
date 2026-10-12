use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
    Router,
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
        if let Err(error) = announce_token_proof() {
            log::warn!("qol clients will send the HTTP token without a server proof: {error:#}");
        }
        Ok(Self { token, port })
    }
}

pub(super) fn proof_routes(security: HttpSecurity) -> Router {
    Router::new()
        .route(qol_conventions::HTTP_AUTH_PROOF_PATH, get(prove_token))
        .with_state(security)
}

#[derive(serde::Deserialize)]
struct ProofQuery {
    nonce: String,
}

async fn prove_token(
    State(security): State<HttpSecurity>,
    Query(query): Query<ProofQuery>,
) -> Response {
    match qol_runtime::local_http::token_proof(&security.token, &query.nonce) {
        Some(proof) => proof.into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            "the nonce must be 32 to 128 hex digits",
        )
            .into_response(),
    }
}

fn announce_token_proof() -> Result<()> {
    let path = qol_config::http_auth_proof_path()
        .context("Could not determine the HTTP token proof path")?;
    qol_fs::atomic_write_private(&path, b"1")
        .with_context(|| format!("failed to write {}", path.display()))
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

    #[tokio::test]
    async fn the_token_proof_needs_a_local_host_and_a_well_formed_nonce_but_no_token() {
        let port = qol_conventions::DEFAULT_PORT;
        let security = HttpSecurity {
            token: Arc::from("secret"),
            port,
        };
        let nonce = "00112233445566778899aabbccddeeff";
        let expected = qol_runtime::local_http::token_proof("secret", nonce).unwrap();
        let cases = [
            (
                format!("localhost:{port}"),
                nonce,
                StatusCode::OK,
                Some(expected),
            ),
            (
                format!("localhost:{port}"),
                "0011",
                StatusCode::BAD_REQUEST,
                None,
            ),
            (
                format!("evil.localhost:{port}"),
                nonce,
                StatusCode::FORBIDDEN,
                None,
            ),
        ];

        for (host, nonce, status, proof) in cases {
            let app = proof_routes(security.clone()).layer(middleware::from_fn_with_state(
                security.clone(),
                require_local_host,
            ));
            let request = Request::get(format!(
                "{}?nonce={nonce}",
                qol_conventions::HTTP_AUTH_PROOF_PATH
            ))
            .header(header::HOST, &host)
            .body(Body::empty())
            .unwrap();
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), status, "{host} {nonce}");
            if let Some(proof) = proof {
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                assert_eq!(body, proof.as_bytes());
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_that_asks_for_proof_reaches_the_api_of_the_real_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let security = HttpSecurity {
            token: Arc::from("secret"),
            port,
        };
        let app = Router::new()
            .route("/api/test", get(|| async { "reached" }))
            .layer(middleware::from_fn_with_state(
                security.clone(),
                require_api_access,
            ))
            .merge(proof_routes(security.clone()))
            .layer(middleware::from_fn_with_state(security, require_local_host));
        tokio::spawn(async move { axum::serve(listener, app).await });

        let response = tokio::task::spawn_blocking(move || {
            qol_runtime::local_http::Client::new(port, "secret")
                .with_server_proof(true)
                .request(qol_runtime::local_http::Method::Get, "/api/test", None)
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!((response.status, response.body.as_str()), (200, "reached"));
    }

    fn headers_with(key: &str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let name = header::HeaderName::from_bytes(key.as_bytes()).unwrap();
        headers.insert(name, HeaderValue::from_str(value).unwrap());
        headers
    }
}

#[cfg(test)]
mod peer_settings_tests {
    use super::{require_api_access, require_local_host, HttpSecurity};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    #[tokio::test]
    async fn peer_settings_routes_share_token_and_origin_policy() {
        for route in ["/peers/admin", "/peers/invitation", "/peers/catalog"] {
            for (host, token, origin, fetch_site, expected) in [
                (
                    "localhost:9876",
                    "",
                    "http://localhost:9876",
                    "same-origin",
                    StatusCode::UNAUTHORIZED,
                ),
                (
                    "localhost:9876",
                    "wrong",
                    "http://localhost:9876",
                    "same-origin",
                    StatusCode::UNAUTHORIZED,
                ),
                (
                    "localhost:9876",
                    "secret",
                    "https://foreign.example",
                    "same-origin",
                    StatusCode::FORBIDDEN,
                ),
                (
                    "localhost:9876",
                    "secret",
                    "http://localhost:9876",
                    "cross-site",
                    StatusCode::FORBIDDEN,
                ),
                (
                    "foreign:9876",
                    "secret",
                    "http://localhost:9876",
                    "same-origin",
                    StatusCode::FORBIDDEN,
                ),
                (
                    "localhost:9876",
                    "secret",
                    "http://localhost:9876",
                    "same-origin",
                    StatusCode::BAD_REQUEST,
                ),
            ] {
                let security = HttpSecurity {
                    token: Arc::from("secret"),
                    port: 9876,
                };
                let app = super::super::settings::peer_routes()
                    .with_state(super::super::settings::PeerCatalogState(Arc::new(
                        std::sync::Mutex::new(crate::plugins::PluginManager::new()),
                    )))
                    .layer(middleware::from_fn_with_state(
                        security.clone(),
                        require_api_access,
                    ))
                    .layer(middleware::from_fn_with_state(security, require_local_host));
                let request = Request::builder()
                    .method(if route == "/peers/catalog" {
                        "GET"
                    } else {
                        "POST"
                    })
                    .uri(route)
                    .header("host", host)
                    .header(qol_conventions::HTTP_AUTH_HEADER, token)
                    .header("origin", origin)
                    .header("sec-fetch-site", fetch_site)
                    .body(Body::from("invalid"))
                    .unwrap();
                let expected =
                    if route == "/peers/catalog" && host == "localhost:9876" && token == "secret" {
                        StatusCode::OK
                    } else {
                        expected
                    };
                let response = app.oneshot(request).await.unwrap();
                if expected == StatusCode::OK || expected == StatusCode::BAD_REQUEST {
                    assert_eq!(response.headers()["cache-control"], "no-store");
                }
                assert_eq!(
                    response.status(),
                    expected,
                    "{route} {host} {origin} {fetch_site}"
                );
            }
        }
    }
}
