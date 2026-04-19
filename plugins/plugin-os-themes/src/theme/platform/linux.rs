//! Linux placeholder for OS-wide theming.
//!
//! OS-wide theming on Linux has no unified API — GTK, Qt, KDE, GNOME, icon
//! themes, cursor themes, and window manager decorations all have separate
//! mechanisms. Likely future entry points:
//!   - GTK: ~/.config/gtk-3.0/settings.ini + gsettings org.gnome.desktop.interface
//!   - Qt:  ~/.config/qt5ct/qt5ct.conf
//!   - Icons: ~/.local/share/icons / ~/.icons/default/index.theme
//!   - Cursors: see cursor/platform/linux.rs (first implementation)

use anyhow::{anyhow, Result};

use super::ThemePlatform;

pub struct Platform;

impl ThemePlatform for Platform {
    fn apply_theme(&self, _name: &str) -> Result<()> {
        Err(anyhow!(
            "plugin-os-themes: OS-wide theming is not implemented on Linux yet"
        ))
    }
}
