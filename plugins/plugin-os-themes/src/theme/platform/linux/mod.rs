mod backends;
mod gsettings;

use anyhow::{bail, Result};

use crate::theme::ColorScheme;

use super::ThemePlatform;
use backends::DesktopBackend;

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
    for part in raw.split(':') {
        match part.trim().to_ascii_lowercase().as_str() {
            "x-cinnamon" | "cinnamon" => return Ok(Box::new(backends::Cinnamon)),
            "gnome" => return Ok(Box::new(backends::Gnome)),
            "kde" => return Ok(Box::new(backends::Kde)),
            _ => {}
        }
    }
    bail!("unsupported desktop environment for theme switching: {raw:?}")
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
