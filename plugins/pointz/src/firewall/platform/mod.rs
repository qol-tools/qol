#[cfg(not(target_os = "windows"))]
mod fallback;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(target_os = "windows"))]
pub(crate) use fallback::{allow, check};
#[cfg(target_os = "windows")]
pub(crate) use windows::{allow, check};
