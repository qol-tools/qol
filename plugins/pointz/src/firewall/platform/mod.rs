#[cfg(not(target_os = "windows"))]
mod fallback;
#[cfg(any(target_os = "windows", test))]
mod rule;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(target_os = "windows"))]
pub(crate) use fallback::{allow, check, remove};
#[cfg(target_os = "windows")]
pub(crate) use windows::{allow, check, remove};
