mod fallback;
#[cfg(target_os = "linux")]
mod linux;

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
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(fallback::FallbackAppsProvider)
    }
}
