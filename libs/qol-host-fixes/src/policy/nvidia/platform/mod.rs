use crate::policy::{ResidencyOwnerId, ResidentPolicy};
use anyhow::Result;

use super::PolicyStatusView;

pub trait NvidiaPolicyBackend {
    fn status(policy: &ResidentPolicy) -> Result<PolicyStatusView>;
    fn enable(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()>;
    fn disable(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()>;
    fn join(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()>;
    fn transfer(policy: &ResidentPolicy, new_owner: &ResidencyOwnerId) -> Result<()>;
    fn run_resident_policy_cli(args: &[String]) -> Result<i32>;
    fn crash_point(point: &str) -> Result<()>;
    fn expected_fingerprint_owner() -> Option<(u32, u32)>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::LinuxNvidia as Backend;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::MacosNvidia as Backend;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::WindowsNvidia as Backend;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::FallbackNvidia as Backend;
