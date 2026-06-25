use anyhow::{anyhow, Result};

use super::model::{AppRef, Shortcut, ShortcutAction};
use super::platform;

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
            return open::that(url).map_err(|e| anyhow!("failed to open url: {}", e));
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
    match crate::local_http::post_to_daemon(&path, "") {
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

#[cfg(debug_assertions)]
fn trace_error(error: &anyhow::Error) -> String {
    error
        .to_string()
        .chars()
        .take(160)
        .map(|c| if c.is_ascii_control() { '_' } else { c })
        .collect()
}
