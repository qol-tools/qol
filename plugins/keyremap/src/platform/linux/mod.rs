use anyhow::Result;
use qol_headless::CommandResult;

use super::virtual_keyboard_absent::MACOS_ONLY;
use super::{
    ConfigInspection, DriverState, HelperState, LayoutGap, PlatformAdapter, Probe,
    SecureInputHolder, TrustStatus,
};

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        "Linux"
    }

    fn supported(&self) -> bool {
        false
    }

    fn launch(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn reload(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn toggle(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn kill(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn hid_helper(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn install_hid_helper(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn uninstall_hid_helper(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn inspect_config(&self) -> Result<ConfigInspection> {
        anyhow::bail!("typed key-remap configuration is only available on macOS")
    }

    fn trust_status(&self) -> TrustStatus {
        TrustStatus::from_trusted(false)
    }

    fn virtual_hid_driver(&self) -> Probe<DriverState> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }

    fn virtual_hid_daemon(&self) -> Probe<bool> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }

    fn hid_helper_state(&self) -> Probe<HelperState> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }

    fn secure_input(&self) -> Probe<Option<SecureInputHolder>> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }

    fn layout_gaps(&self) -> Probe<Vec<LayoutGap>> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }
}

fn unsupported() -> CommandResult {
    CommandResult::runtime_error(
        "keyremap: only macOS is supported (requires CGEventTap and Accessibility APIs)",
    )
}
