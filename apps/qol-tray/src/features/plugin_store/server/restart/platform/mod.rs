#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(unix, not(target_os = "macos")))]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(super) fn binary_name() -> &'static str {
    "qol-tray"
}
#[cfg(windows)]
pub(super) fn binary_name() -> &'static str {
    windows::binary_name()
}

#[cfg(target_os = "macos")]
pub(super) fn exec_restart(binary: &std::path::Path) -> Result<(), String> {
    macos::exec_restart(binary)
}
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn exec_restart(binary: &std::path::Path) -> Result<(), String> {
    unix::exec_restart(binary)
}
#[cfg(windows)]
pub(super) fn exec_restart(binary: &std::path::Path) -> Result<(), String> {
    windows::exec_restart(binary)
}

#[cfg(not(any(unix, windows)))]
compile_error!("Self-recompile restart is not implemented for this platform");
