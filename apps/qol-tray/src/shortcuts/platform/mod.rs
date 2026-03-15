#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

use anyhow::Result;

use super::model::AppRef;

pub(super) fn open_url_in_browser(url: &str, browser: &AppRef) -> Result<()> {
    #[cfg(target_os = "linux")]
    return linux::open_url_in_browser(url, browser);

    #[cfg(target_os = "macos")]
    return macos::open_url_in_browser(url, browser);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fallback::open_url_in_browser(url, browser)
}

pub(super) fn launch_app(app: &AppRef) -> Result<()> {
    #[cfg(target_os = "linux")]
    return linux::launch_app(app);

    #[cfg(target_os = "macos")]
    return macos::launch_app(app);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fallback::launch_app(app)
}
