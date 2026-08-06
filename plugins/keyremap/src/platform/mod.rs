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
    fn reload(&self) -> Result<CommandResult>;
    fn kill(&self) -> Result<CommandResult>;
    fn inspect_config(&self) -> Result<ConfigInspection>;
    fn trust_status(&self) -> TrustStatus;
}
