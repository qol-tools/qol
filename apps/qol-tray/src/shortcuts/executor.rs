use anyhow::{anyhow, Result};
use std::fmt;

use super::model::{AppRef, Shortcut, ShortcutAction};
use super::platform;

#[derive(Debug)]
pub enum ExecuteByIdError {
    InvalidId(String),
    LoadFailed(anyhow::Error),
    NotFound(String),
    ExecuteFailed(anyhow::Error),
}

impl fmt::Display for ExecuteByIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(message) => formatter.write_str(message),
            Self::LoadFailed(error) => write!(formatter, "failed to load shortcuts: {error}"),
            Self::NotFound(id) => write!(formatter, "shortcut '{id}' not found"),
            Self::ExecuteFailed(error) => write!(formatter, "failed to execute shortcut: {error}"),
        }
    }
}

impl std::error::Error for ExecuteByIdError {}

pub fn execute_by_id(id: &str) -> std::result::Result<(), ExecuteByIdError> {
    super::validation::validate_id(id).map_err(ExecuteByIdError::InvalidId)?;
    let config = super::store::load().map_err(ExecuteByIdError::LoadFailed)?;
    let shortcut = super::store::find_by_id(&config, id)
        .ok_or_else(|| ExecuteByIdError::NotFound(id.to_string()))?;
    execute(&shortcut).map_err(ExecuteByIdError::ExecuteFailed)
}

pub fn execute(shortcut: &Shortcut) -> Result<()> {
    if !shortcut.enabled {
        log::warn!("Shortcut skipped: id={} reason=disabled", shortcut.id);
        qol_runtime::probe!("SHORTCUT_SKIP", "id={} reason=disabled", shortcut.id);
        return Err(anyhow!("shortcut '{}' is disabled", shortcut.id));
    }
    let action = shortcut.action.kind();
    log::info!("Shortcut executing: id={} action={}", shortcut.id, action);
    qol_runtime::probe!("SHORTCUT_EXEC", "id={} action={action}", shortcut.id);

    let result = match &shortcut.action {
        ShortcutAction::OpenUrl {
            url,
            browser_override,
        } => open_url(url, browser_override.as_ref()),
        ShortcutAction::LaunchApp { app } => launch_app(app),
        ShortcutAction::PluginAction { plugin_id, action } => run_plugin_action(plugin_id, action),
    };

    match &result {
        Ok(()) => {
            log::info!("Shortcut completed: id={} action={}", shortcut.id, action);
            qol_runtime::probe!("SHORTCUT_OK", "id={} action={action}", shortcut.id);
        }
        Err(error) => {
            log::error!(
                "Shortcut failed: id={} action={} error={:#}",
                shortcut.id,
                action,
                error
            );
            qol_runtime::probe!(
                "SHORTCUT_FAIL",
                "id={} action={action} error={}",
                shortcut.id,
                trace_error(error)
            );
        }
    }

    result
}

fn open_url(url: &str, browser: Option<&AppRef>) -> Result<()> {
    let browser = match browser {
        Some(browser) => browser,
        None => {
            log::debug!("Shortcut open_url using default browser");
            return qol_apps::desktop_integration::open_with_default_app(url)
                .map_err(|error| anyhow!("failed to open url: {error}"));
        }
    };
    log::debug!(
        "Shortcut open_url using browser override: kind={}",
        app_ref_kind(browser)
    );
    platform::open_url_in_browser(url, browser)
}

fn launch_app(app: &AppRef) -> Result<()> {
    log::debug!("Shortcut launch_app: kind={}", app_ref_kind(app));
    platform::launch_app(app)
}

fn run_plugin_action(plugin_id: &str, action: &str) -> Result<()> {
    let path = format!("/api/plugins/{plugin_id}/actions/{action}");
    match qol_plugin_api::host_exec::post_to_daemon(&path, "") {
        Ok((status, _)) if (200..300).contains(&status) => Ok(()),
        Ok((status, body)) if body.is_empty() => {
            Err(anyhow!("plugin action request failed with HTTP {}", status))
        }
        Ok((_, body)) => Err(anyhow!(body)),
        Err(_) => Err(anyhow!("qol-tray is not running")),
    }
}

fn app_ref_kind(app: &AppRef) -> &'static str {
    match app {
        AppRef::BundleId { .. } => "bundle_id",
        AppRef::Path { .. } => "path",
        AppRef::Name { .. } => "name",
    }
}

fn trace_error(error: &anyhow::Error) -> String {
    error
        .to_string()
        .chars()
        .take(160)
        .map(|c| if c.is_ascii_control() { '_' } else { c })
        .collect()
}
