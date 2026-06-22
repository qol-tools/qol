#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub(crate) use fallback::{start, Monitor};
#[cfg(target_os = "macos")]
pub(crate) use macos::{start, Monitor};
