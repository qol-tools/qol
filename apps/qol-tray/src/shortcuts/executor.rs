use anyhow::{anyhow, Result};

use super::model::{AppRef, Shortcut, ShortcutAction};

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
    open_url_in_browser(url, browser)
}

#[cfg(target_os = "macos")]
fn open_url_in_browser(url: &str, browser: &AppRef) -> Result<()> {
    let mut cmd = std::process::Command::new("open");
    match browser {
        AppRef::BundleId { id } => {
            cmd.args(["-b", id, url]);
        }
        AppRef::Path { path } | AppRef::Name { name: path } => {
            cmd.args(["-a", path, url]);
        }
    }
    cmd.spawn()
        .map_err(|e| anyhow!("failed to open url in browser: {}", e))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_url_in_browser(url: &str, browser: &AppRef) -> Result<()> {
    let bin = match browser {
        AppRef::BundleId { id } => id.as_str(),
        AppRef::Path { path } => path.as_str(),
        AppRef::Name { name } => name.as_str(),
    };
    std::process::Command::new(bin)
        .arg(url)
        .spawn()
        .map_err(|e| anyhow!("failed to open url in browser: {}", e))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn open_url_in_browser(_url: &str, _browser: &AppRef) -> Result<()> {
    Err(anyhow!(
        "browser override is not supported on this platform"
    ))
}

#[cfg(target_os = "macos")]
fn launch_app(app: &AppRef) -> Result<()> {
    let mut cmd = std::process::Command::new("open");
    match app {
        AppRef::BundleId { id } => {
            cmd.args(["-b", id]);
        }
        AppRef::Path { path } => {
            cmd.arg(path);
        }
        AppRef::Name { name } => {
            cmd.args(["-a", name]);
        }
    }
    cmd.spawn()
        .map_err(|e| anyhow!("failed to launch app: {}", e))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch_app(app: &AppRef) -> Result<()> {
    let bin = match app {
        AppRef::BundleId { id } => id.as_str(),
        AppRef::Path { path } => path.as_str(),
        AppRef::Name { name } => name.as_str(),
    };
    std::process::Command::new(bin)
        .spawn()
        .map_err(|e| anyhow!("failed to launch app: {}", e))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn launch_app(_app: &AppRef) -> Result<()> {
    Err(anyhow!("launch_app is not supported on this platform"))
}
