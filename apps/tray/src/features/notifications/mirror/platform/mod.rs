#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub(super) use fallback::{access, activate, is_safe_banner_id, sync};
#[cfg(target_os = "macos")]
pub(super) use macos::{access, activate, is_safe_banner_id, sync};
