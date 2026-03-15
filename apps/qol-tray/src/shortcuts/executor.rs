use anyhow::{anyhow, Result};

use super::model::{AppRef, Shortcut, ShortcutAction};
use super::platform;

pub fn execute(shortcut: &Shortcut) -> Result<()> {
    if !shortcut.enabled {
        return Err(anyhow!("shortcut '{}' is disabled", shortcut.id));
    }
    match &shortcut.action {
        ShortcutAction::OpenUrl {
            url,
            browser_override,
        } => open_url(url, browser_override.as_ref()),
        ShortcutAction::LaunchApp { app } => launch_app(app),
    }
}

fn open_url(url: &str, browser: Option<&AppRef>) -> Result<()> {
    let browser = match browser {
        Some(b) => b,
        None => return open::that(url).map_err(|e| anyhow!("failed to open url: {}", e)),
    };
    platform::open_url_in_browser(url, browser)
}

fn launch_app(app: &AppRef) -> Result<()> {
    platform::launch_app(app)
}
