use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use qol_mcp::{
    jsonrpc::{error_response, ErrorCode},
    Caller,
};
use std::sync::Arc;

pub(super) type SharedHost = Arc<dyn qol_mcp::ToolHost + Send + Sync>;

pub(super) fn router_with_host(host: SharedHost) -> Router {
    Router::new()
        .route("/", post(post_message).get(reject_get).delete(end_session))
        .with_state(host)
}

async fn post_message(State(host): State<SharedHost>, headers: HeaderMap, body: Bytes) -> Response {
    let caller = Caller {
        agent_home: crate::features::agents::header_agent_home(&headers),
    };
    let message = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(value) => value,
        Err(error) => {
            return jsonrpc_error(
                StatusCode::BAD_REQUEST,
                ErrorCode::ParseError,
                &error.to_string(),
            )
        }
    };
    match tokio::task::spawn_blocking(move || qol_mcp::handle(host.as_ref(), message, &caller))
        .await
    {
        Ok(Some(value)) => (StatusCode::OK, Json(value)).into_response(),
        Ok(None) => StatusCode::ACCEPTED.into_response(),
        Err(error) => jsonrpc_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::InternalError,
            &error.to_string(),
        ),
    }
}

fn jsonrpc_error(status: StatusCode, code: ErrorCode, message: &str) -> Response {
    (
        status,
        Json(error_response(serde_json::Value::Null, code, message)),
    )
        .into_response()
}

async fn reject_get() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

async fn end_session() -> StatusCode {
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use qol_conventions::HTTP_AGENT_HOME_HEADER;
    use tower::ServiceExt;

    struct FakeHost;

    impl qol_mcp::ToolHost for FakeHost {
        fn server_info(&self) -> qol_mcp::ServerInfo {
            qol_mcp::ServerInfo {
                name: "fake".to_string(),
                version: "0.0.0".to_string(),
            }
        }

        fn list(&self) -> Vec<qol_mcp::ToolSpec> {
            vec![qol_mcp::ToolSpec {
                name: "echo".to_string(),
                description: "echo the arguments".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            }]
        }

        fn call(
            &self,
            _name: &str,
            arguments: serde_json::Value,
            caller: &qol_mcp::Caller,
        ) -> qol_mcp::ToolResult {
            qol_mcp::ToolResult::structured(serde_json::json!({
                "arguments": arguments,
                "agent_home": caller.agent_home,
            }))
        }
    }

    async fn send(method: &'static str, body: Option<&str>) -> Response {
        send_with_agent_home(method, body, None).await
    }

    async fn send_with_agent_home(
        method: &'static str,
        body: Option<&str>,
        agent_home: Option<&str>,
    ) -> Response {
        let mut builder = axum::http::Request::builder().method(method).uri("/");
        if let Some(agent_home) = agent_home {
            builder = builder.header(HTTP_AGENT_HOME_HEADER, agent_home);
        }
        let request = match body {
            Some(payload) => builder
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        router_with_host(Arc::new(FakeHost))
            .oneshot(request)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn initialize_returns_server_info_with_requested_protocol_version() {
        let response = send(
            "POST",
            Some(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(value["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(value["result"]["serverInfo"]["name"], "fake");
    }

    #[tokio::test]
    async fn notification_returns_accepted_with_empty_body() {
        let response = send(
            "POST",
            Some(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let payload = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(payload.is_empty());
    }

    #[tokio::test]
    async fn invalid_json_returns_parse_error() {
        let response = send("POST", Some("not json")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(value["error"]["code"], serde_json::json!(-32700));
    }

    #[tokio::test]
    async fn get_is_method_not_allowed() {
        let response = send("GET", None).await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn delete_ends_the_session() {
        let response = send("DELETE", None).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn agent_home_header_reaches_the_host_trimmed_and_blank_is_dropped() {
        let echo = r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"echo","arguments":{}}}"#;
        let response =
            send_with_agent_home("POST", Some(echo), Some("  /home/k/.claude-work ")).await;
        let payload = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(
            value["result"]["structuredContent"]["agent_home"],
            "/home/k/.claude-work"
        );
        for agent_home in [None, Some("   ")] {
            let response = send_with_agent_home("POST", Some(echo), agent_home).await;
            let payload = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
            assert_eq!(
                value["result"]["structuredContent"]["agent_home"],
                serde_json::Value::Null
            );
        }
    }
}
