use qol_desktop_settings::desktop::{classify_desktop, DesktopEnvironment};
use qol_platform::LinuxDisplayBackend;

use crate::xdg::desktops_from;

pub(super) struct Session {
    pub raw_desktop: String,
    pub desktop: DesktopEnvironment,
    pub display: LinuxDisplayBackend,
}

impl Session {
    pub(super) fn current() -> Self {
        let raw = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        Self::from_parts(&raw, qol_platform::linux_display_backend())
    }

    pub(super) fn from_parts(raw_desktop: &str, display: LinuxDisplayBackend) -> Self {
        Self {
            raw_desktop: raw_desktop.to_string(),
            desktop: classify_desktop(raw_desktop),
            display,
        }
    }

    pub(super) fn desktops(&self) -> Vec<String> {
        desktops_from(&self.raw_desktop)
    }
}
