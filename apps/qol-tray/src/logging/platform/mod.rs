#[cfg(any(
    test,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::log_dir;
#[cfg(target_os = "linux")]
pub(crate) use linux::log_dir;
#[cfg(target_os = "macos")]
pub(crate) use macos::log_dir;
#[cfg(target_os = "windows")]
pub(crate) use windows::log_dir;

#[cfg(test)]
mod tests;
