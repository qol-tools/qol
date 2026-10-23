use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use qol_config::placement::Spot;
use qol_plugin_daemon::notification::gate::NativeHandler;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::super::types::MAX_CONFIG_SIZE;
use super::http_json::{self, blocking};

type HttpResult<T> = Result<T, Box<Response>>;

const MESSAGE_SECONDS_MIN: f64 = qol_theme::STAY_MESSAGE_LEAST.as_secs_f64();
const MESSAGE_SECONDS_MAX: f64 = qol_theme::STAY_MESSAGE_MOST.as_secs_f64();
const DO_NOT_DISTURB_QUERY: &str = "os_do_not_disturb";

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct NotificationsConfig {
    handler: NativeHandler,
    placement: String,
    message_seconds: f64,
}

#[derive(Deserialize)]
struct NotificationsConfigRequest {
    #[serde(flatten)]
    values: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct QueryResponse {
    value: String,
}

fn current_config() -> NotificationsConfig {
    NotificationsConfig {
        handler: crate::features::notifications::native_handler(),
        placement: crate::features::notifications::placement().token(),
        message_seconds: crate::features::notifications::message_seconds(),
    }
}

pub(in super::super) async fn get_config() -> impl IntoResponse {
    blocking("notifications config", get_config_inner).await
}

pub(in super::super) async fn set_config(body: axum::body::Bytes) -> impl IntoResponse {
    blocking("notifications config", move || set_config_inner(body)).await
}

pub(in super::super) async fn get_query(Path(query): Path<String>) -> impl IntoResponse {
    blocking("notifications query", move || query_inner(&query)).await
}

fn get_config_inner() -> HttpResult<Response> {
    config_response(&current_config())
}

fn set_config_inner(body: axum::body::Bytes) -> HttpResult<Response> {
    let request: NotificationsConfigRequest = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    let mut changes = Vec::new();
    for (field, value) in &request.values {
        changes.push(parsed_change(field, value)?);
    }
    for change in changes {
        change
            .apply()
            .map_err(|error| Box::new(bad_request(&format!("{error:#}"))))?;
    }
    config_response(&current_config())
}

fn query_inner(query: &str) -> HttpResult<Response> {
    if query != DO_NOT_DISTURB_QUERY {
        return Err(Box::new(
            (StatusCode::NOT_FOUND, format!("unknown query: {query}")).into_response(),
        ));
    }
    let value = match qol_plugin_daemon::notification::platform::os_do_not_disturb() {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Unknown",
    };
    let json = http_json::encode_json(
        &QueryResponse {
            value: value.to_string(),
        },
        "Failed to serialize notifications query",
    )?;
    Ok(http_json::json_response(json))
}

enum Change {
    Handler(NativeHandler),
    Placement(Spot),
    MessageSeconds(f64),
}

impl Change {
    fn apply(self) -> anyhow::Result<()> {
        match self {
            Self::Handler(handler) => {
                let previous = crate::features::notifications::native_handler();
                if handler == previous {
                    return Ok(());
                }
                let result = crate::features::notifications::set_native_handler(handler);
                crate::features::notifications::apply_native_handler(previous);
                result
            }
            Self::Placement(spot) => crate::features::notifications::set_placement(spot),
            Self::MessageSeconds(seconds) => {
                crate::features::notifications::set_message_seconds(seconds)
            }
        }
    }
}

fn parsed_change(field: &str, value: &serde_json::Value) -> HttpResult<Change> {
    match field {
        "handler" => match value.as_str() {
            Some("qol") => Ok(Change::Handler(NativeHandler::Qol)),
            Some("os") => Ok(Change::Handler(NativeHandler::Os)),
            Some("both") => Ok(Change::Handler(NativeHandler::Both)),
            _ => Err(Box::new(bad_request(&format!(
                "{field} expects qol, os, or both"
            )))),
        },
        "placement" => value
            .as_str()
            .and_then(Spot::parse)
            .map(Change::Placement)
            .ok_or_else(|| {
                Box::new(bad_request(&format!(
                    "{field} expects a spot such as top-right, or its x,y fractions"
                )))
            }),
        "message_seconds" => value
            .as_f64()
            .map(|seconds| (seconds * 10.0).round() / 10.0)
            .filter(|seconds| (MESSAGE_SECONDS_MIN..=MESSAGE_SECONDS_MAX).contains(seconds))
            .map(Change::MessageSeconds)
            .ok_or_else(|| {
                Box::new(bad_request(&format!(
                    "{field} expects {MESSAGE_SECONDS_MIN} to {MESSAGE_SECONDS_MAX} seconds"
                )))
            }),
        _ => Err(Box::new(bad_request(&format!("unknown field: {field}")))),
    }
}

fn config_response(config: &NotificationsConfig) -> HttpResult<Response> {
    let json = http_json::encode_json(config, "Failed to serialize notification settings")?;
    Ok(http_json::json_response(json))
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, message.to_string()).into_response()
}

#[cfg(test)]
mod tests {
    use super::{get_config_inner, query_inner, set_config_inner, NotificationsConfig};
    use qol_plugin_daemon::notification::gate::NativeHandler;

    async fn body_of(response: axum::response::Response) -> (u16, String) {
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    async fn config_of(response: axum::response::Response) -> NotificationsConfig {
        let (status, body) = body_of(response).await;
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    fn put(
        body: serde_json::Value,
    ) -> Result<axum::response::Response, Box<axum::response::Response>> {
        set_config_inner(axum::body::Bytes::from(body.to_string()))
    }

    #[tokio::test]
    async fn the_page_reads_and_saves_every_notification_setting() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        let initial = config_of(get_config_inner().unwrap()).await;
        assert_eq!(
            initial,
            NotificationsConfig {
                handler: NativeHandler::Qol,
                placement: "top-right".into(),
                message_seconds: qol_theme::STAY_MESSAGE.as_secs_f64(),
            }
        );
        let saved = config_of(
            put(serde_json::json!({
                "handler": "qol",
                "placement": "0.2,0.7",
                "message_seconds": 2.5,
            }))
            .unwrap(),
        )
        .await;
        assert_eq!(saved.placement, "0.2,0.7");
        assert_eq!(saved.message_seconds, 2.5);
        let snapped = config_of(put(serde_json::json!({ "placement": "0,1" })).unwrap()).await;
        assert_eq!(snapped.placement, "bottom-left");
        assert_eq!(config_of(get_config_inner().unwrap()).await, snapped);
    }

    #[tokio::test]
    async fn a_bad_value_is_refused_before_anything_is_saved() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        for body in [
            serde_json::json!({ "placement": "middle" }),
            serde_json::json!({ "placement": "1.5,0" }),
            serde_json::json!({ "placement": "bottom-left", "message_seconds": 90 }),
            serde_json::json!({ "placement": "bottom-left", "handler": "pager" }),
            serde_json::json!({ "placement": "bottom-left", "theme": "slate" }),
        ] {
            let refused = put(body.clone()).unwrap_err();
            assert_eq!(refused.status().as_u16(), 400, "{body}");
        }
        let config = config_of(get_config_inner().unwrap()).await;
        assert_eq!(config.placement, "top-right");
    }

    #[test]
    fn only_the_do_not_disturb_query_is_answered() {
        assert!(query_inner("os_do_not_disturb").is_ok());
        assert!(query_inner("profiles").is_err());
    }

    #[test]
    fn do_not_disturb_row_explains_what_it_mirrors() {
        let spec = qol_config::contract::parse_spec_str(include_str!(
            "../../../../settings_surface/platform/notifications-config.toml"
        ))
        .unwrap();
        let field = spec.field("do_not_disturb").unwrap();
        let description = field.description.as_deref().unwrap_or_default();
        assert!(description.contains("Follows your system"), "{description}");
        assert!(
            description.contains("qol toasts still show"),
            "{description}"
        );
    }
}
