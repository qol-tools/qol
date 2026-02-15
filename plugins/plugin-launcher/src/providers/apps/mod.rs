mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

use crate::desktop_entry::DesktopEntry;

pub type AppEntry = DesktopEntry;

pub trait AppsProvider: Send + Sync {
    fn load_entries(&self) -> Vec<AppEntry>;
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
