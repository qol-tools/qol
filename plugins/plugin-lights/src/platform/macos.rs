use anyhow::{Context, Result};
use std::process::{Command, Stdio};

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-lights/";

pub fn open_settings() -> Result<()> {
    Command::new("open")
        .arg(SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}
