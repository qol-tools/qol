use crate::policy::{ResidencyOwnerId, ResidentPolicy};
use anyhow::Result;

use super::{ActiveFileFingerprint, PolicyStatusView};

pub trait NvidiaPolicyBackend {
    fn status(policy: &ResidentPolicy) -> Result<PolicyStatusView>;
    fn enable(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()>;
    fn disable(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()>;
    fn join(policy: &ResidentPolicy, owner: &ResidencyOwnerId) -> Result<()>;
    fn transfer(policy: &ResidentPolicy, new_owner: &ResidencyOwnerId) -> Result<()>;
    fn run_resident_policy_cli(args: &[String]) -> Result<i32>;
    fn crash_point(point: &str) -> Result<()>;
    fn validate_fingerprint_owner(fingerprint: &ActiveFileFingerprint) -> Result<()>;
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
