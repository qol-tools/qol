use anyhow::{Context, Result};
use std::process::{Command, Stdio};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn open_settings() -> Result<()> {
    let settings_url = qol_conventions::settings_url(PLUGIN_ID);
    Command::new("xdg-open")
        .arg(&settings_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}
