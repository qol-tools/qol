use anyhow::Result;
use qol_headless::{CommandResult, DoctorCheckResult};
use qol_platform::PermissionState;

use super::virtual_keyboard_absent::unavailable;
use super::{ConfigInspection, PlatformAdapter};

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        "Unsupported OS"
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
        anyhow::bail!("typed key-remap configuration is only available on macOS and Windows")
    }

    fn input_permission(&self) -> PermissionState {
        qol_platform::permission_status(qol_platform::Permission::InputCapture)
    }

    fn virtual_hid_driver(&self) -> Result<DoctorCheckResult> {
        Ok(unavailable("virtual_hid_driver"))
    }

    fn virtual_hid_daemon(&self) -> DoctorCheckResult {
        unavailable("virtual_hid_daemon")
    }

    fn hid_helper_state(&self) -> DoctorCheckResult {
        unavailable("hid_helper")
    }

    fn secure_input(&self) -> DoctorCheckResult {
        unavailable("secure_input")
    }

    fn layout_characters(&self) -> DoctorCheckResult {
        unavailable("layout_characters")
    }
}

fn unsupported() -> CommandResult {
    CommandResult::runtime_error("keyremap: only macOS and Windows are supported")
}
