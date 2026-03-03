mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

use std::path::PathBuf;

use crate::desktop_entry::DesktopEntry;

pub type AppEntry = DesktopEntry;

pub trait AppsProvider: Send + Sync {
    fn load_entries(&self) -> Vec<AppEntry>;
}

pub(crate) fn watch_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos::app_dirs()
    }
    #[cfg(target_os = "linux")]
    {
        linux::application_dirs()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

pub fn default_provider() -> Box<dyn AppsProvider> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxAppsProvider)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacosAppsProvider)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Box::new(fallback::FallbackAppsProvider)
    }
}
