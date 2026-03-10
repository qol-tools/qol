#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("stub_apps platform implementation is required for this target OS");

pub(super) fn sync(
    stubs: &[super::StubInput],
    binary_path: &std::path::Path,
) -> anyhow::Result<()> {
    imp::sync(stubs, binary_path)
}
