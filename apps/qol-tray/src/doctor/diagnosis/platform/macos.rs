use super::super::{AppKeyWriter, GsettingsBackend, SymbolicHotkeyWriter};
use anyhow::{anyhow, Context, Result};
use std::process::Command;

pub(crate) struct Platform;

impl SymbolicHotkeyWriter for Platform {
    fn disable(&mut self, hotkey_id: u32) -> Result<()> {
        let value =
            "{ enabled = 0; value = { parameters = (0, 0, 0); type = standard; }; }".to_string();
        let output = Command::new("defaults")
            .args([
                "write",
                "com.apple.symbolichotkeys",
                "AppleSymbolicHotKeys",
                "-dict-add",
                &hotkey_id.to_string(),
                &value,
            ])
            .output()
            .with_context(|| {
                format!("failed to invoke defaults write for symbolichotkey {hotkey_id}")
            })?;
        if !output.status.success() {
            return Err(anyhow!(
                "defaults write symbolichotkey {hotkey_id} exited with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        Ok(())
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
    fn read(&mut self, _schema: &str, _key: &str) -> Result<String> {
        Err(anyhow!("gsettings mutation is only supported on Linux"))
    }

    fn write(&mut self, _schema: &str, _key: &str, _value: &str) -> Result<()> {
        Err(anyhow!("gsettings mutation is only supported on Linux"))
    }
}
