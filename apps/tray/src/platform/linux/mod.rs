#[cfg(feature = "linux_evdev")]
pub(crate) mod classify;
#[cfg(feature = "linux_evdev")]
pub(crate) mod keycode;

pub(crate) const APP_DESKTOP_ENTRY: &str = "qol-tray.desktop";
pub(crate) const SETTINGS_SURFACE_DESKTOP_ENTRY: &str = "qol-settings-surface.desktop";
pub(crate) const INSTALLER_DESKTOP_ENTRIES: [&str; 2] =
    [APP_DESKTOP_ENTRY, SETTINGS_SURFACE_DESKTOP_ENTRY];
