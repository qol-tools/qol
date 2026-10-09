#[cfg(not(target_os = "windows"))]
mod other;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(target_os = "windows"))]
pub(crate) use other::{allow, check};
#[cfg(target_os = "windows")]
pub(crate) use windows::{allow, check};
