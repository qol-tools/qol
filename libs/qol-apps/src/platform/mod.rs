#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub(crate) use fallback::{bundle_info, spotlight_app_paths};
#[cfg(target_os = "macos")]
pub(crate) use macos::{bundle_info, spotlight_app_paths};
