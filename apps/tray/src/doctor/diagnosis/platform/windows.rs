use super::super::{AppKeyWriter, SymbolicHotkeyWriter};
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
    fn clear(&mut self, app_key: &str) -> Result<()> {
        if !is_safe_app_key(app_key) {
            return Err(anyhow!("unsafe Windows AppKey identifier: {app_key}"));
        }
        let key_path =
            format!(r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\AppKey\{app_key}");
        let output = Command::new("reg")
            .args(["delete", &key_path, "/v", "ShortcutKeys", "/f"])
            .output()
            .with_context(|| format!("failed to invoke reg delete for AppKey {app_key}"))?;
        if !output.status.success() {
            return Err(anyhow!(
                "reg delete AppKey {app_key} exited with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        Ok(())
    }
}

fn is_safe_app_key(app_key: &str) -> bool {
    !app_key.is_empty()
        && app_key.len() <= 16
        && app_key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}
