use super::super::{AppKeyWriter, SymbolicHotkeyWriter, NATIVE_NOTIFICATIONS_ID};
use crate::doctor::framework::CheckReport;
use anyhow::{anyhow, Context, Result};
use qol_plugin_daemon::notification::gate::NativeHandler;
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

pub(crate) fn native_notifications_diagnosis() -> CheckReport {
    if crate::features::notifications::native_handler() == NativeHandler::Qol {
        return CheckReport::ok("native notifications are disabled; qol toasts are used");
    }
    CheckReport::warn(
        "qol system notifications are attributed to Script Editor - disable them in System Settings > Notifications if unwanted",
        NATIVE_NOTIFICATIONS_ID,
        Vec::new(),
    )
}
