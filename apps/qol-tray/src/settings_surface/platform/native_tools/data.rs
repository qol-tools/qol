use std::time::Duration;

use anyhow::{bail, Context};
use qol_runtime::local_http::{Client, Method};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::hotkeys::{HotkeyBinding, HotkeyConfig};
use crate::shortcuts::model::{Shortcut, ShortcutsConfig};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(65);

#[derive(Clone, Debug, Deserialize)]
pub(super) struct PluginOption {
    pub(super) uid: String,
    pub(super) name: String,
    #[serde(default)]
    pub(super) loaded: bool,
    #[serde(default)]
    pub(super) actions: Vec<ActionOption>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ActionOption {
    pub(super) id: String,
    pub(super) label: String,
}

#[derive(Debug, Deserialize)]
struct InstalledResponse {
    plugins: Vec<PluginOption>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct RegistrationError {
    pub(super) key: String,
    pub(super) error: String,
}

#[derive(Debug)]
pub(super) struct ToolsData {
    pub(super) shortcuts: Vec<Shortcut>,
    pub(super) hotkeys: Vec<HotkeyBinding>,
    pub(super) plugins: Vec<PluginOption>,
    pub(super) registration_errors: Vec<RegistrationError>,
}

#[derive(Debug, Deserialize)]
pub(super) struct NativeCaptureResult {
    pub(super) native: bool,
    pub(super) key: Option<String>,
    pub(super) canceled: bool,
}

pub(super) fn load() -> anyhow::Result<ToolsData> {
    let shortcuts: ShortcutsConfig = get_json(qol_conventions::api_routes::SHORTCUTS)?;
    let hotkeys: HotkeyConfig = get_json(qol_conventions::api_routes::HOTKEYS)?;
    let installed: InstalledResponse = get_json("/api/installed")?;
    let registration_errors =
        get_json(qol_conventions::api_routes::HOTKEY_ERRORS).unwrap_or_default();
    let mut plugins = installed
        .plugins
        .into_iter()
        .filter(|plugin| plugin.loaded)
        .collect::<Vec<_>>();
    plugins.sort_by_key(|plugin| plugin.name.to_lowercase());
    Ok(ToolsData {
        shortcuts: shortcuts.shortcuts,
        hotkeys: hotkeys.hotkeys,
        plugins,
        registration_errors,
    })
}

pub(super) fn create_shortcut(shortcut: &Shortcut) -> anyhow::Result<Vec<Shortcut>> {
    let config: ShortcutsConfig = send_json(Method::Post, "/api/shortcuts", shortcut)?;
    Ok(config.shortcuts)
}

pub(super) fn update_shortcut(shortcut: &Shortcut) -> anyhow::Result<Vec<Shortcut>> {
    let route = qol_conventions::api_routes::shortcut(&shortcut.id);
    let config: ShortcutsConfig = send_json(Method::Put, &route, shortcut)?;
    Ok(config.shortcuts)
}

pub(super) fn delete_shortcut(id: &str) -> anyhow::Result<Vec<Shortcut>> {
    let route = qol_conventions::api_routes::shortcut(id);
    let config: ShortcutsConfig = request_json(Method::Delete, &route, None, REQUEST_TIMEOUT)?;
    Ok(config.shortcuts)
}

pub(super) fn run_shortcut(id: &str) -> anyhow::Result<()> {
    let route = format!("{}/run", qol_conventions::api_routes::shortcut(id));
    request_text(Method::Post, &route, Some("{}"), REQUEST_TIMEOUT).map(drop)
}

pub(super) fn save_hotkeys(hotkeys: &[HotkeyBinding]) -> anyhow::Result<()> {
    let config = HotkeyConfig {
        hotkeys: hotkeys.to_vec(),
    };
    let body = serde_json::to_string(&config)?;
    request_text(
        Method::Put,
        qol_conventions::api_routes::HOTKEYS,
        Some(&body),
        REQUEST_TIMEOUT,
    )
    .map(drop)
}

pub(super) fn capture_hotkey(session_id: u64) -> anyhow::Result<NativeCaptureResult> {
    let route = format!("/api/hotkeys/recording/{session_id}/capture");
    request_json(Method::Post, &route, Some("{}"), CAPTURE_TIMEOUT)
}

pub(super) fn cancel_hotkey_capture(session_id: u64) -> anyhow::Result<()> {
    let route = format!("/api/hotkeys/recording/{session_id}");
    request_text(Method::Delete, &route, None, REQUEST_TIMEOUT).map(drop)
}

fn get_json<T: DeserializeOwned>(route: &str) -> anyhow::Result<T> {
    request_json(Method::Get, route, None, REQUEST_TIMEOUT)
}

fn send_json<T: DeserializeOwned, B: Serialize>(
    method: Method,
    route: &str,
    body: &B,
) -> anyhow::Result<T> {
    let body = serde_json::to_string(body)?;
    request_json(method, route, Some(&body), REQUEST_TIMEOUT)
}

fn request_json<T: DeserializeOwned>(
    method: Method,
    route: &str,
    body: Option<&str>,
    timeout: Duration,
) -> anyhow::Result<T> {
    let body = request_text(method, route, body, timeout)?;
    serde_json::from_str(&body).with_context(|| format!("{route} returned invalid JSON"))
}

fn request_text(
    method: Method,
    route: &str,
    body: Option<&str>,
    timeout: Duration,
) -> anyhow::Result<String> {
    let token = std::env::var(qol_conventions::ENV_HTTP_TOKEN)
        .context("tray HTTP authentication token is unavailable")?;
    let response = Client::new(qol_conventions::DEFAULT_PORT, token)
        .with_io_timeout(timeout)
        .request(method, route, body)
        .with_context(|| format!("request to {route} failed"))?;
    if !(200..300).contains(&response.status) {
        bail!("{}", response.body.trim());
    }
    Ok(response.body)
}
