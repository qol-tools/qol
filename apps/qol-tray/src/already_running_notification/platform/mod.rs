#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub(crate) fn show() {
    #[cfg(target_os = "linux")]
    return linux::show();

    #[cfg(target_os = "macos")]
    return macos::show();

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fallback::show();
}
