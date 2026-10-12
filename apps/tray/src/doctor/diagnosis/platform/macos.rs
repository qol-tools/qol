use super::super::{AppKeyWriter, SymbolicHotkeyWriter, NATIVE_NOTIFICATIONS_ID};
use crate::doctor::framework::CheckReport;
use crate::features::notifications::MirrorAccess;
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
        return qol_only_diagnosis(
            crate::features::notifications::mirror_access(),
            qol_plugin_daemon::notification::platform::os_do_not_disturb(),
        );
    }
    CheckReport::warn(
        "qol system notifications are attributed to Script Editor - disable them in System Settings > Notifications if unwanted",
        NATIVE_NOTIFICATIONS_ID,
        Vec::new(),
    )
}

fn qol_only_diagnosis(access: MirrorAccess, do_not_disturb: Option<bool>) -> CheckReport {
    if let MirrorAccess::Unreadable(reason) = access {
        return CheckReport::warn(
            format!(
                "other apps' notifications cannot be shown as qol toasts ({reason}); \
                 grant qol-tray Full Disk Access in System Settings > Privacy & Security"
            ),
            NATIVE_NOTIFICATIONS_ID,
            Vec::new(),
        );
    }
    if do_not_disturb == Some(false) {
        return CheckReport::warn(
            "other apps' notifications show as qol toasts and as macOS banners; \
             turn on Do Not Disturb to keep only qol toasts",
            NATIVE_NOTIFICATIONS_ID,
            Vec::new(),
        );
    }
    CheckReport::ok("qol toasts are used; other apps' macOS notifications are shown as qol toasts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qol_only_diagnosis_cases() {
        let cases = [
            (MirrorAccess::Readable, Some(true), 0, "shown as qol toasts"),
            (MirrorAccess::Readable, None, 0, "shown as qol toasts"),
            (MirrorAccess::Readable, Some(false), 1, "Do Not Disturb"),
            (
                MirrorAccess::Unreadable("authorization denied".to_string()),
                Some(true),
                1,
                "Full Disk Access",
            ),
        ];
        for (access, do_not_disturb, issues, needle) in cases {
            let report = qol_only_diagnosis(access.clone(), do_not_disturb);
            assert_eq!(report.issues.len(), issues, "{access:?} {do_not_disturb:?}");
            assert!(report.summary.contains(needle), "{}", report.summary);
        }
    }
}
