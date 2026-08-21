use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use qol_conventions::DEFAULT_PORT;
use qol_runtime::local_http::{Client, Method, Session};

#[derive(serde::Deserialize)]
struct ActionResult {
    #[serde(default)]
    data: Option<serde_json::Value>,
}

pub(super) fn config_path(plugin_id: &str) -> anyhow::Result<PathBuf> {
    qol_config::plugin_config_write_path(plugin_id)
        .ok_or_else(|| anyhow::anyhow!("no plugin config path available"))
}

pub(super) fn panel_base(plugin_id: &str) -> String {
    if plugin_id == qol_conventions::CORE_PANEL_ID {
        "/api/core".to_string()
    } else {
        format!("/api/plugins/{plugin_id}")
    }
}

fn tray_config_route(base: &str) -> String {
    format!("{base}/config")
}

pub(super) fn query(base: &str, query: &str) -> Result<serde_json::Value, String> {
    let route = format!("{base}/queries/{query}");
    let (status, body) = tray_http_session(&route).map_err(|error| error.to_string())?;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "panel={base} phase=runtime-query query={query} status={status}"
    );
    if status != 200 {
        return Err(format!("query `{query}` failed with HTTP {status}: {body}"));
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("query `{query}` returned invalid JSON: {error}"))
}

pub(super) fn run_action(
    base: &str,
    action: &str,
    input: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let route = format!("{base}/actions/{action}");
    let body = serde_json::to_string(input).map_err(|error| error.to_string())?;
    let (status, response) =
        tray_http(Method::Post, &route, Some(&body)).map_err(|error| error.to_string())?;
    if (200..300).contains(&status) {
        return action_result_data(&response);
    }
    Err(format!(
        "action `{action}` failed with HTTP {status}: {response}"
    ))
}

fn action_result_data(response: &str) -> Result<Option<serde_json::Value>, String> {
    let result: ActionResult = serde_json::from_str(response)
        .map_err(|error| format!("action returned invalid JSON: {error}"))?;
    Ok(result.data)
}

pub(super) fn daemon_port(base: &str) -> Option<u16> {
    let route = format!("{base}/config-form");
    let (status, body) = tray_http_session(&route).ok()?;
    if status != 200 {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()?
        .get("daemonPort")?
        .as_u64()?
        .try_into()
        .ok()
}

pub(super) fn load_values(base: &str, path: Option<&Path>) -> serde_json::Value {
    if let Ok((200, body)) = tray_http(Method::Get, &tray_config_route(base), None) {
        if let Ok(values) = serde_json::from_str(&body) {
            return values;
        }
    }
    path.and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub(super) fn save_values(base: &str, path: Option<&Path>, values: &serde_json::Value) {
    let body = match serde_json::to_string(values) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("[{base}] settings serialize failed: {error:#}");
            return;
        }
    };
    match tray_http(Method::Put, &tray_config_route(base), Some(&body)) {
        Ok((200, _)) => {
            qol_runtime::probe!(
                "SETTINGS_PERSIST",
                "panel={base} transport=tray outcome=saved"
            );
            return;
        }
        Ok((status, payload)) => {
            qol_runtime::probe!(
                "SETTINGS_PERSIST",
                "panel={base} transport=tray outcome=rejected status={status}"
            );
            eprintln!(
                "[{base}] settings save rejected by tray ({status}): {}",
                payload.trim()
            );
            return;
        }
        Err(error) => {
            qol_runtime::probe!(
                "SETTINGS_PERSIST",
                "panel={base} transport=tray outcome=unavailable"
            );
            eprintln!("[{base}] tray unreachable, saving locally: {error:#}");
        }
    }
    let Some(path) = path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(path, body) {
        qol_runtime::probe!(
            "SETTINGS_PERSIST",
            "panel={base} transport=file outcome=failed"
        );
        eprintln!("[{base}] settings save failed: {error:#}");
        return;
    }
    qol_runtime::probe!(
        "SETTINGS_PERSIST",
        "panel={base} transport=file outcome=saved"
    );
}

fn tray_http(method: Method, route: &str, body: Option<&str>) -> anyhow::Result<(u16, String)> {
    let response = tray_client()?.request(method, route, body)?;
    Ok((response.status, response.body))
}

fn tray_client() -> anyhow::Result<Client> {
    use anyhow::Context as _;

    let token = std::env::var(qol_conventions::ENV_HTTP_TOKEN)
        .context("tray HTTP authentication token is unavailable")?;
    Ok(Client::new(DEFAULT_PORT, token).with_io_timeout(Duration::from_secs(2)))
}

thread_local! {
    static QUERY_SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

fn tray_http_session(route: &str) -> anyhow::Result<(u16, String)> {
    QUERY_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        let session = match &mut *session {
            Some(session) => session,
            none => none.insert(Session::new(tray_client()?)),
        };
        let response = session.request(Method::Get, route, None)?;
        Ok((response.status, response.body))
    })
}

#[cfg(test)]
mod tests {
    use super::action_result_data;

    #[test]
    fn panel_base_maps_plugin_and_core_ids() {
        assert_eq!(super::panel_base("plugin-foo"), "/api/plugins/plugin-foo");
        assert_eq!(
            super::panel_base(qol_conventions::CORE_PANEL_ID),
            "/api/core"
        );
    }

    #[test]
    fn action_result_extracts_optional_daemon_data() {
        let cases = [
            (r#"{"success":true,"message":"Action dispatched"}"#, None),
            (
                r#"{"success":true,"message":"Action dispatched","data":{"dark":true}}"#,
                Some(serde_json::json!({"dark": true})),
            ),
        ];
        for (response, expected) in cases {
            assert_eq!(action_result_data(response).unwrap(), expected);
        }
    }

    #[test]
    fn action_result_rejects_invalid_json() {
        let error = action_result_data("not json").unwrap_err();
        assert!(error.starts_with("action returned invalid JSON:"));
    }
}
