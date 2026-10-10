mod doctor;
mod elevation;
mod foreground;
mod hook;
mod keys;
mod machine;
mod profile;
mod service;

use anyhow::Result;
use qol_headless::{CommandResult, DoctorCheckResult};

use super::engine;
use super::{ConfigInspection, PlatformAdapter};

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        "Windows"
    }

    fn supported(&self) -> bool {
        true
    }

    fn launch(&self) -> Result<CommandResult> {
        service::run()?;
        Ok(CommandResult::success(""))
    }

    fn hid_helper(&self) -> Result<CommandResult> {
        Ok(CommandResult::runtime_error(
            "keyremap: the keyboard helper only exists on macOS; Windows remaps in low-level hooks",
        ))
    }

    fn install_hid_helper(&self) -> Result<CommandResult> {
        Ok(CommandResult::success(
            "Windows needs no keyboard helper; nothing was installed.\n",
        ))
    }

    fn uninstall_hid_helper(&self) -> Result<CommandResult> {
        Ok(CommandResult::success(
            "Windows needs no keyboard helper; nothing was removed.\n",
        ))
    }

    fn inspect_config(&self) -> Result<ConfigInspection> {
        engine::inspection(keys::modifier_key_issues)
    }

    fn virtual_hid_driver(&self) -> Result<DoctorCheckResult> {
        Ok(doctor::driver())
    }

    fn virtual_hid_daemon(&self) -> DoctorCheckResult {
        doctor::daemon()
    }

    fn hid_helper_state(&self) -> DoctorCheckResult {
        doctor::helper()
    }

    fn secure_input(&self) -> DoctorCheckResult {
        doctor::secure_input()
    }

    fn layout_characters(&self) -> DoctorCheckResult {
        doctor::layout(&engine::remap::character_targets(&service::load()))
    }
}
