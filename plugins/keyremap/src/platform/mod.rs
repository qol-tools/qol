use anyhow::Result;
use qol_headless::CommandResult;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::Adapter as Platform;
#[cfg(target_os = "linux")]
pub(crate) use linux::Adapter as Platform;
#[cfg(target_os = "macos")]
pub(crate) use macos::Adapter as Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Adapter as Platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustStatus {
    Trusted,
    NotTrusted,
}

impl TrustStatus {
    pub(crate) fn from_trusted(trusted: bool) -> Self {
        if trusted {
            Self::Trusted
        } else {
            Self::NotTrusted
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecureInputHolder {
    pub(crate) pid: i32,
    pub(crate) app: String,
}

#[cfg(not(target_os = "macos"))]
pub(crate) const MACOS_ONLY: &str = "the virtual keyboard path only exists on macOS";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Probe<T> {
    Known(T),
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtensionState {
    Activated,
    NotActivated,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DriverState {
    pub(crate) installed_version: Option<String>,
    pub(crate) extension: ExtensionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelperReport {
    pub(crate) virtual_keyboard_ready: bool,
    pub(crate) input_monitoring: bool,
    pub(crate) seized: Vec<String>,
    pub(crate) conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HelperState {
    NotInstalled,
    NotRunning,
    VersionMismatch { helper: u32, expected: u32 },
    Running(HelperReport),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutGap {
    pub(crate) rule: String,
    pub(crate) character: String,
}

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

pub(crate) const INPUT_MONITORING_FIX: &str = "In System Settings > Privacy & Security > Input Monitoring, click +, press Cmd+Shift+G, open /Library/PrivilegedHelperTools/com.qol-tools.keyremap.hid-helper and turn it on. Then run: sudo launchctl kickstart -k system/com.qol-tools.keyremap.hid-helper";

pub(crate) trait PlatformAdapter: Clone + Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn supported(&self) -> bool;
    fn launch(&self) -> Result<CommandResult>;
    fn reload(&self) -> Result<CommandResult>;
    fn toggle(&self) -> Result<CommandResult>;
    fn kill(&self) -> Result<CommandResult>;
    fn hid_helper(&self) -> Result<CommandResult>;
    fn install_hid_helper(&self) -> Result<CommandResult>;
    fn uninstall_hid_helper(&self) -> Result<CommandResult>;
    fn inspect_config(&self) -> Result<ConfigInspection>;
    fn trust_status(&self) -> TrustStatus;
    fn virtual_hid_driver(&self) -> Probe<DriverState>;
    fn virtual_hid_daemon(&self) -> Probe<bool>;
    fn hid_helper_state(&self) -> Probe<HelperState>;
    fn secure_input(&self) -> Probe<Option<SecureInputHolder>>;
    fn layout_gaps(&self) -> Probe<Vec<LayoutGap>>;
}
