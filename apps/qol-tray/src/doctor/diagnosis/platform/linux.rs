use super::super::{AppKeyWriter, GsettingsBackend, SymbolicHotkeyWriter};
use anyhow::{anyhow, Context, Result};
use std::process::Command;

pub(crate) struct Platform;

impl SymbolicHotkeyWriter for Platform {
    fn disable(&mut self, _hotkey_id: u32) -> Result<()> {
        Err(anyhow!(
            "symbolic hotkey mutation is only supported on macOS"
        ))
    }
}

impl AppKeyWriter for Platform {
    fn clear(&mut self, _app_key: &str) -> Result<()> {
        Err(anyhow!(
            "Windows AppKey mutation is only supported on Windows"
        ))
    }
}

impl GsettingsBackend for Platform {
    fn read(&mut self, schema: &str, key: &str) -> Result<String> {
        let output = Command::new("gsettings")
            .args(["get", schema, key])
            .output()
            .with_context(|| format!("failed to invoke gsettings get {schema} {key}"))?;
        if !output.status.success() {
            return Err(anyhow!(
                "gsettings get {schema} {key} exited with status {}",
                output.status
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn write(&mut self, schema: &str, key: &str, value: &str) -> Result<()> {
        let output = Command::new("gsettings")
            .args(["set", schema, key, value])
            .output()
            .with_context(|| format!("failed to invoke gsettings set {schema} {key}"))?;
        if !output.status.success() {
            return Err(anyhow!(
                "gsettings set {schema} {key} exited with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        Ok(())
    }
}
