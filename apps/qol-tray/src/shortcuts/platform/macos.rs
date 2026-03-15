use anyhow::{anyhow, Result};

use super::super::model::AppRef;

pub(super) fn open_url_in_browser(url: &str, browser: &AppRef) -> Result<()> {
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

pub(super) fn launch_app(app: &AppRef) -> Result<()> {
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
