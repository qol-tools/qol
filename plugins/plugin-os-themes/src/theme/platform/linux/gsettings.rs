use std::process::Command;

use anyhow::{bail, Context, Result};

pub(super) fn get(schema: &str, key: &str) -> Result<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .context("failed to run gsettings")?;
    if !output.status.success() {
        bail!(
            "gsettings get {schema} {key} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(unquote(String::from_utf8_lossy(&output.stdout).trim()))
}

pub(super) fn set(schema: &str, key: &str, value: &str) -> Result<()> {
    let status = Command::new("gsettings")
        .args(["set", schema, key, value])
        .status()
        .context("failed to run gsettings")?;
    if !status.success() {
        bail!("gsettings set {schema} {key} {value} failed");
    }
    Ok(())
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .unwrap_or(value)
        .to_string()
}
