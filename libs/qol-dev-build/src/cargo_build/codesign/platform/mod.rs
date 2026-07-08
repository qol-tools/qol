#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod other;

#[cfg(target_os = "macos")]
pub(super) use macos::codesign_debug_binaries;
#[cfg(not(target_os = "macos"))]
pub(super) use other::codesign_debug_binaries;
