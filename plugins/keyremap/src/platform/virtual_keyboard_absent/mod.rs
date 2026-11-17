use anyhow::Result;
use qol_headless::{CommandResult, DoctorCheckResult};

use super::{ConfigInspection, PlatformAdapter, ABSENT_NAME};
use crate::config::Inspection;
use crate::control::Host;

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        ABSENT_NAME
    }

    fn supported(&self) -> bool {
        false
    }

    fn launch(&self, _host: &Host) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn hid_helper(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn install_hid_helper(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn karabiner_rules(&self, _host: &Host) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn karabiner_sync(&self, _host: &Host) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn karabiner_remove(&self, _host: &Host) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn uninstall_hid_helper(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn inspect_config(&self, _inspected: &Inspection) -> Result<ConfigInspection> {
        anyhow::bail!("typed key-remap configuration is only available on macOS and Windows")
    }

    fn virtual_hid_driver(&self) -> Result<DoctorCheckResult> {
        Ok(unavailable("virtual_hid_driver"))
    }

    fn virtual_hid_daemon(&self, _host: &Host) -> DoctorCheckResult {
        unavailable("virtual_hid_daemon")
    }

    fn hid_helper_state(&self) -> DoctorCheckResult {
        unavailable("hid_helper")
    }

    fn key_backend(&self, _host: &Host) -> DoctorCheckResult {
        unavailable("key_backend")
    }

    fn karabiner_block(&self, _host: &Host) -> DoctorCheckResult {
        unavailable("karabiner_rules")
    }

    fn secure_input(&self, _host: &Host) -> DoctorCheckResult {
        unavailable("secure_input")
    }

    fn layout_characters(&self, _host: &Host) -> DoctorCheckResult {
        unavailable("layout_characters")
    }
}

pub(super) fn reload(_host: &Host) -> CommandResult {
    unsupported()
}

pub(super) fn toggle(_host: &Host) -> CommandResult {
    unsupported()
}

pub(super) fn kill(_host: &Host) -> CommandResult {
    unsupported()
}

fn unsupported() -> CommandResult {
    CommandResult::runtime_error("keyremap: only macOS and Windows are supported")
}

fn unavailable(id: &str) -> DoctorCheckResult {
    DoctorCheckResult::fail(id, "Key Remap has no input hooks on this platform.")
}
