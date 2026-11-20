use anyhow::Result;
use qol_headless::{CommandResult, DoctorCheckResult};
use qol_platform::PermissionState;

use crate::config::Inspection;
use crate::control::Host;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod engine;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod virtual_keyboard_absent;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::NAME as ABSENT_NAME;
#[cfg(target_os = "linux")]
use linux::NAME as ABSENT_NAME;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use engine as runner;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use virtual_keyboard_absent as runner;

#[cfg(target_os = "macos")]
pub(crate) use macos::Adapter as Platform;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) use virtual_keyboard_absent::Adapter as Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Adapter as Platform;

pub(crate) struct ConfigInspection {
    pub(crate) source: bool,
    pub(crate) enabled: bool,
    pub(crate) char_rules: usize,
    pub(crate) char_swaps: usize,
    pub(crate) key_rules: usize,
    pub(crate) mouse_rules: usize,
    pub(crate) scroll_rules: usize,
    pub(crate) issues: Vec<String>,
}

pub(crate) trait PlatformAdapter: Clone + Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn supported(&self) -> bool;
    fn launch(&self, host: &Host) -> Result<CommandResult>;
    fn reload(&self, host: &Host) -> Result<CommandResult> {
        Ok(runner::reload(host))
    }
    fn toggle(&self, host: &Host) -> Result<CommandResult> {
        Ok(runner::toggle(host))
    }
    fn kill(&self, host: &Host) -> Result<CommandResult> {
        Ok(runner::kill(host))
    }
    fn hid_helper(&self) -> Result<CommandResult>;
    fn install_hid_helper(&self) -> Result<CommandResult>;
    fn uninstall_hid_helper(&self) -> Result<CommandResult>;
    fn karabiner_rules(&self, host: &Host) -> Result<CommandResult>;
    fn karabiner_sync(&self, host: &Host) -> Result<CommandResult>;
    fn karabiner_remove(&self, host: &Host) -> Result<CommandResult>;
    fn inspect_config(&self, inspected: &Inspection) -> Result<ConfigInspection>;
    fn input_permission(&self) -> PermissionState {
        qol_platform::permission_status(qol_platform::Permission::InputCapture)
    }
    fn virtual_hid_driver(&self) -> Result<DoctorCheckResult>;
    fn virtual_hid_daemon(&self, host: &Host) -> DoctorCheckResult;
    fn hid_helper_state(&self) -> DoctorCheckResult;
    fn key_backend(&self, host: &Host) -> DoctorCheckResult;
    fn karabiner_block(&self, host: &Host) -> DoctorCheckResult;
    fn secure_input(&self, host: &Host) -> DoctorCheckResult;
    fn layout_characters(&self, host: &Host) -> DoctorCheckResult;
}
