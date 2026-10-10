use anyhow::Result;
use qol_headless::{CommandResult, DoctorCheckResult};

use super::{ConfigInspection, PlatformAdapter, ABSENT_NAME};

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        ABSENT_NAME
    }

    fn supported(&self) -> bool {
        false
    }

    fn launch(&self) -> Result<CommandResult> {
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

pub(super) fn reload() -> CommandResult {
    unsupported()
}

pub(super) fn toggle() -> CommandResult {
    unsupported()
}

pub(super) fn kill() -> CommandResult {
    unsupported()
}

fn unsupported() -> CommandResult {
    CommandResult::runtime_error("keyremap: only macOS and Windows are supported")
}

fn unavailable(id: &str) -> DoctorCheckResult {
    DoctorCheckResult::fail(id, "Key Remap has no input hooks on this platform.")
}
