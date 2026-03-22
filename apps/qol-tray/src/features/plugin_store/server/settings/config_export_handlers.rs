use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::types::AppState;

#[derive(Serialize)]
struct ConfigBundle {
    version: u32,
    exported_at: String,
    hotkeys: Value,
    shortcuts: Value,
    task_runner: Value,
    plugin_configs: Value,
    installed_plugins: Vec<String>,
}

#[derive(Deserialize)]
struct ConfigBundle2 {
    hotkeys: Option<Value>,
    shortcuts: Option<Value>,
    task_runner: Option<Value>,
    plugin_configs: Option<Value>,
}

pub(in super::super) async fn export_config(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    let hotkeys = read_json_file_or_default(&crate::paths::hotkeys_path);
    let shortcuts = read_json_file_or_default(&crate::paths::shortcuts_path);
    let task_runner = read_json_file_or_default(&crate::paths::task_runner_config_path);
    let plugin_configs = read_json_file_or_default(&crate::paths::plugin_configs_path);

    let installed_plugins: Vec<String> = match state.plugin_manager.lock() {
        Ok(pm) => pm.plugins().map(|p| p.id.to_string()).collect(),
        Err(_) => Vec::new(),
    };

    let bundle = ConfigBundle {
        version: 1,
        exported_at: chrono::Local::now().to_rfc3339(),
        hotkeys,
        shortcuts,
        task_runner,
        plugin_configs,
        installed_plugins,
    };

    Json(bundle).into_response()
}

pub(in super::super) async fn import_config(body: axum::body::Bytes) -> impl IntoResponse {
    let bundle: ConfigBundle2 = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    if let Some(hotkeys) = &bundle.hotkeys {
        write_json_config(&crate::paths::hotkeys_path, hotkeys);
    }
    if let Some(shortcuts) = &bundle.shortcuts {
        write_json_config(&crate::paths::shortcuts_path, shortcuts);
    }
    if let Some(task_runner) = &bundle.task_runner {
        write_json_config(&crate::paths::task_runner_config_path, task_runner);
    }
    if let Some(plugin_configs) = &bundle.plugin_configs {
        write_json_config(&crate::paths::plugin_configs_path, plugin_configs);
    }

    crate::hotkeys::trigger_reload();

    StatusCode::OK.into_response()
}

fn read_json_file_or_default(path_fn: &dyn Fn() -> anyhow::Result<std::path::PathBuf>) -> Value {
    let path = match path_fn() {
        Ok(p) => p,
        Err(_) => return Value::Null,
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

fn write_json_config(path_fn: &dyn Fn() -> anyhow::Result<std::path::PathBuf>, value: &Value) {
    let Ok(path) = path_fn() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(content) = serde_json::to_string_pretty(value) else {
        return;
    };
    let _ = std::fs::write(path, content);
}
