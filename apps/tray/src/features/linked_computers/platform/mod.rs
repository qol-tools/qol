#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub(super) use fallback::computer_name;
#[cfg(target_os = "macos")]
pub(super) use macos::computer_name;
