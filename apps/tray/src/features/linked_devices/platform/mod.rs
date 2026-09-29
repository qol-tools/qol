#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub(super) use fallback::device_name;
#[cfg(target_os = "macos")]
pub(super) use macos::device_name;
