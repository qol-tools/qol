#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(super) fn binary_name() -> &'static str {
    unix::binary_name()
}
#[cfg(windows)]
pub(super) fn binary_name() -> &'static str {
    windows::binary_name()
}

#[cfg(unix)]
pub(super) fn spawn_delayed(binary: &std::path::Path) -> Result<(), String> {
    unix::spawn_delayed(binary)
}
#[cfg(windows)]
pub(super) fn spawn_delayed(binary: &std::path::Path) -> Result<(), String> {
    windows::spawn_delayed(binary)
}

#[cfg(not(any(unix, windows)))]
compile_error!("Self-recompile restart is not implemented for this platform");
