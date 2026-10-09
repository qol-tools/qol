#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub(super) use fallback::is_listening_socket;
#[cfg(target_os = "macos")]
pub(super) use macos::is_listening_socket;
