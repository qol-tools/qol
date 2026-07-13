use anyhow::Result;

use crate::theme::ColorScheme;

use super::super::gsettings;
use super::{installed_themes, naming, DesktopBackend};

const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";

pub(in super::super) struct Gnome;

impl DesktopBackend for Gnome {
    fn current_scheme(&self) -> Result<ColorScheme> {
        let scheme = gsettings::get(INTERFACE_SCHEMA, "color-scheme")?;
        if scheme == "prefer-dark" {
            Ok(ColorScheme::Dark)
        } else {
            Ok(ColorScheme::Light)
        }
    }

    fn apply(&self, target: ColorScheme) -> Result<()> {
        let color_scheme = match target {
            ColorScheme::Light => "default",
            ColorScheme::Dark => "prefer-dark",
        };
        gsettings::set(INTERFACE_SCHEMA, "color-scheme", color_scheme)?;
        apply_gtk_theme(target);
        Ok(())
    }
}

fn apply_gtk_theme(target: ColorScheme) {
    let Ok(current) = gsettings::get(INTERFACE_SCHEMA, "gtk-theme") else {
        return;
    };
    let resolved = naming::resolve(&current, target, &installed_themes());
    match resolved {
        Ok(name) => {
            if let Err(error) = gsettings::set(INTERFACE_SCHEMA, "gtk-theme", &name) {
                eprintln!("[os-themes] gtk theme not updated: {error:#}");
            }
        }
        Err(error) => eprintln!("[os-themes] gtk theme not updated: {error:#}"),
    }
}
