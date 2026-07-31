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

fn tray_config_route(plugin_id: &str) -> String {
    format!("/api/plugins/{plugin_id}/config")
}

pub(super) fn query(plugin_id: &str, query: &str) -> Result<serde_json::Value, String> {
    let route = format!("/api/plugins/{plugin_id}/queries/{query}");
    let (status, body) = tray_http_session(&route).map_err(|error| error.to_string())?;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={plugin_id} phase=runtime-query query={query} status={status}"
    );
    if status != 200 {
        return Err(format!("query `{query}` failed with HTTP {status}: {body}"));
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("query `{query}` returned invalid JSON: {error}"))
}

pub(super) fn run_action(
    plugin_id: &str,
    action: &str,
    input: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let route = format!("/api/plugins/{plugin_id}/actions/{action}");
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

pub(super) fn load_values(plugin_id: &str, path: &Path) -> serde_json::Value {
    if let Ok((200, body)) = tray_http(Method::Get, &tray_config_route(plugin_id), None) {
        if let Ok(values) = serde_json::from_str(&body) {
            return values;
        }
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub(super) fn save_values(plugin_id: &str, path: &Path, values: &serde_json::Value) {
    let body = match serde_json::to_string(values) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("[{plugin_id}] settings serialize failed: {error:#}");
            return;
        }
    };
    match tray_http(Method::Put, &tray_config_route(plugin_id), Some(&body)) {
        Ok((200, _)) => {
            qol_runtime::probe!(
                "SETTINGS_PERSIST",
                "plugin={plugin_id} transport=tray outcome=saved"
            );
            return;
        }
        Ok((status, payload)) => {
            qol_runtime::probe!(
                "SETTINGS_PERSIST",
                "plugin={plugin_id} transport=tray outcome=rejected status={status}"
            );
            eprintln!(
                "[{plugin_id}] settings save rejected by tray ({status}): {}",
                payload.trim()
            );
            return;
        }
        Err(error) => {
            qol_runtime::probe!(
                "SETTINGS_PERSIST",
                "plugin={plugin_id} transport=tray outcome=unavailable"
            );
            eprintln!("[{plugin_id}] tray unreachable, saving locally: {error:#}");
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(path, body) {
        qol_runtime::probe!(
            "SETTINGS_PERSIST",
            "plugin={plugin_id} transport=file outcome=failed"
        );
        eprintln!("[{plugin_id}] settings save failed: {error:#}");
        return;
    }
    qol_runtime::probe!(
        "SETTINGS_PERSIST",
        "plugin={plugin_id} transport=file outcome=saved"
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
