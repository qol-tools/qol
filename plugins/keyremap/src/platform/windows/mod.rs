mod doctor;
mod foreground;
mod hook;
mod keys;
mod machine;
mod profile;
mod service;
mod status;

use anyhow::Result;
use qol_headless::{CommandResult, DoctorCheckResult};

use super::engine;
use super::{ConfigInspection, PlatformAdapter};
use crate::config::Inspection;
use crate::control::Host;

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        qol_platform::Os::Windows.label()
    }

    fn supported(&self) -> bool {
        true
    }

    fn launch(&self, host: &Host) -> Result<CommandResult> {
        service::run(host)?;
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

    fn karabiner_rules(&self, _host: &Host) -> Result<CommandResult> {
        Ok(karabiner_is_macos_only())
    }

    fn karabiner_sync(&self, _host: &Host) -> Result<CommandResult> {
        Ok(karabiner_is_macos_only())
    }

    fn karabiner_remove(&self, _host: &Host) -> Result<CommandResult> {
        Ok(karabiner_is_macos_only())
    }

    fn inspect_config(&self, inspected: &Inspection) -> Result<ConfigInspection> {
        Ok(engine::inspection(inspected, keys::modifier_key_issues))
    }

    fn virtual_hid_driver(&self) -> Result<DoctorCheckResult> {
        Ok(doctor::driver())
    }

    fn virtual_hid_daemon(&self, host: &Host) -> DoctorCheckResult {
        doctor::daemon(&host.plugin_id)
    }

    fn hid_helper_state(&self) -> DoctorCheckResult {
        doctor::helper()
    }

    fn key_backend(&self, _host: &Host) -> DoctorCheckResult {
        doctor::key_backend()
    }

    fn karabiner_block(&self, _host: &Host) -> DoctorCheckResult {
        doctor::karabiner_rules()
    }

    fn secure_input(&self, _host: &Host) -> DoctorCheckResult {
        doctor::secure_input()
    }

    fn layout_characters(&self, host: &Host) -> DoctorCheckResult {
        doctor::layout(&engine::remap::character_targets(&service::resolve((host
            .read_config)(
        ))))
    }
}

fn karabiner_is_macos_only() -> CommandResult {
    CommandResult::runtime_error(
        "keyremap: Karabiner-Elements only exists on macOS; Windows remaps in low-level hooks",
    )
}
