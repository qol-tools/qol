use anyhow::{anyhow, Result};

use super::super::model::AppRef;

pub(in crate::shortcuts) fn open_url_in_browser(url: &str, browser: &AppRef) -> Result<()> {
    let bin = match browser {
        AppRef::BundleId { id } => id.as_str(),
        AppRef::Path { path } => path.as_str(),
        AppRef::Name { name } => name.as_str(),
    };
    let mut command = std::process::Command::new(bin);
    command.arg(url);
    qol_conventions::scrub_launched_child_env(&mut command);
    command
        .spawn()
        .map_err(|e| anyhow!("failed to open url in browser: {}", e))?;
    Ok(())
}

pub(in crate::shortcuts) fn launch_app(app: &AppRef) -> Result<()> {
    let bin = match app {
        AppRef::BundleId { id } => id.as_str(),
        AppRef::Path { path } => path.as_str(),
        AppRef::Name { name } => name.as_str(),
    };
    let mut command = std::process::Command::new(bin);
    qol_conventions::scrub_launched_child_env(&mut command);
    command
        .spawn()
        .map_err(|e| anyhow!("failed to launch app: {}", e))?;
    Ok(())
}
