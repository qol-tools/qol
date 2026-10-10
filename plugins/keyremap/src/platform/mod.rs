use anyhow::Result;
use qol_headless::{CommandResult, DoctorCheckResult};
use qol_platform::PermissionState;

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
use engine as control;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use virtual_keyboard_absent as control;

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
    fn launch(&self) -> Result<CommandResult>;
    fn reload(&self) -> Result<CommandResult> {
        Ok(control::reload())
    }
    fn toggle(&self) -> Result<CommandResult> {
        Ok(control::toggle())
    }
    fn kill(&self) -> Result<CommandResult> {
        Ok(control::kill())
    }
    fn hid_helper(&self) -> Result<CommandResult>;
    fn install_hid_helper(&self) -> Result<CommandResult>;
    fn uninstall_hid_helper(&self) -> Result<CommandResult>;
    fn inspect_config(&self) -> Result<ConfigInspection>;
    fn input_permission(&self) -> PermissionState {
        qol_platform::permission_status(qol_platform::Permission::InputCapture)
    }
    fn virtual_hid_driver(&self) -> Result<DoctorCheckResult>;
    fn virtual_hid_daemon(&self) -> DoctorCheckResult;
    fn hid_helper_state(&self) -> DoctorCheckResult;
    fn secure_input(&self) -> DoctorCheckResult;
    fn layout_characters(&self) -> DoctorCheckResult;
}
