use super::Platform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub(crate) fn create() -> impl Platform {
    linux::LinuxQueries::new()
}

#[cfg(target_os = "macos")]
pub(crate) fn create() -> impl Platform {
    macos::MacQueries::new(std::process::id() as i32)
}
