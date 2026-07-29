mod backends;
mod desktop;
mod gsettings;

use anyhow::{bail, Result};

use crate::theme::ColorScheme;

use super::ThemePlatform;
use backends::DesktopBackend;
pub(crate) use desktop::{classify_desktop, DesktopEnvironment};

pub struct Platform;

impl ThemePlatform for Platform {
    fn current_scheme(&self) -> Result<ColorScheme> {
        detect_backend()?.current_scheme()
    }

    fn apply_scheme(&self, target: ColorScheme) -> Result<()> {
        detect_backend()?.apply(target)
    }
}

fn detect_backend() -> Result<Box<dyn DesktopBackend>> {
    let raw = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    backend_for(&raw)
}

fn backend_for(raw: &str) -> Result<Box<dyn DesktopBackend>> {
    match classify_desktop(raw) {
        DesktopEnvironment::Gnome => Ok(Box::new(backends::Gnome)),
        DesktopEnvironment::Cinnamon => Ok(Box::new(backends::Cinnamon)),
        DesktopEnvironment::Kde => Ok(Box::new(backends::Kde)),
        DesktopEnvironment::Unknown => {
            bail!("unsupported desktop environment for theme switching: {raw:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_detection_table() {
        let cases = [
            ("X-Cinnamon", true),
            ("cinnamon", true),
            ("KDE", true),
            ("ubuntu:GNOME", true),
            ("GNOME", true),
            ("", false),
            ("Hyprland", false),
        ];
        for (input, expected) in cases {
            assert_eq!(backend_for(input).is_ok(), expected, "input: {input}");
        }
    }
}
