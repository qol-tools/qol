use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use qol_agent_homes::Registry;

pub(crate) fn header_agent_home(headers: &HeaderMap) -> Option<String> {
    headers
        .get(qol_conventions::HTTP_AGENT_HOME_HEADER)
        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/agents", get(get_agents))
}

async fn get_agents() -> Response {
    match tokio::task::spawn_blocking(Registry::load).await {
        Ok(registry) => Json(serde_json::json!({"homes": registry.homes()})).into_response(),
        Err(error) => {
            log::error!("agents homes join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "agents homes join error").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use qol_agent_homes::Harness;
    use tower::ServiceExt;

    #[tokio::test]
    async fn agents_route_lists_a_home_for_every_harness() {
        let response = routes::<()>()
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        let homes = value["homes"].as_array().expect("homes array");
        let required_keys = ["harness", "id", "path", "shared", "default", "declared"];
        for home in homes {
            for key in required_keys {
                assert!(home.get(key).is_some(), "row missing key {key}: {home}");
            }
        }
        for harness in Harness::ALL {
            assert!(
                homes.iter().any(|home| {
                    home["harness"] == harness.id()
                        && home["id"].as_str().is_some_and(|id| !id.is_empty())
                }),
                "harness: {}",
                harness.id()
            );
        }
    }

    #[test]
    fn header_agent_home_decodes_utf8_home_paths() {
        let mut headers = HeaderMap::new();
        headers.insert(
            qol_conventions::HTTP_AGENT_HOME_HEADER,
            axum::http::HeaderValue::from_bytes("/home/k/ren\u{e9}".as_bytes()).unwrap(),
        );
        assert_eq!(
            header_agent_home(&headers).as_deref(),
            Some("/home/k/ren\u{e9}")
        );
    }
}
