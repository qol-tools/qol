use std::ffi::OsString;

use anyhow::{anyhow, Result};

use super::super::model::AppRef;

pub(in crate::shortcuts) fn open_url_in_browser(url: &str, browser: &AppRef) -> Result<()> {
    qol_apps::shell_execute::shell_execute(&shell_target(browser), &[url.to_string()])
        .map_err(|e| anyhow!("failed to open url in browser: {}", e))
}

pub(in crate::shortcuts) fn launch_app(app: &AppRef) -> Result<()> {
    qol_apps::shell_execute::shell_execute(&shell_target(app), &[])
        .map_err(|e| anyhow!("failed to launch app: {}", e))
}

fn shell_target(app: &AppRef) -> OsString {
    match app {
        AppRef::BundleId { id } => OsString::from(format!("shell:AppsFolder\\{id}")),
        AppRef::Path { path } => OsString::from(path),
        AppRef::Name { name } => OsString::from(name),
    }
}
