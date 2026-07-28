use anyhow::Result;

use crate::theme::ColorScheme;

use super::super::gsettings;
use super::{installed_themes, naming, DesktopBackend};

const INTERFACE_SCHEMA: &str = "org.cinnamon.desktop.interface";
const SHELL_SCHEMA: &str = "org.cinnamon.theme";

pub(in super::super) struct Cinnamon;

impl DesktopBackend for Cinnamon {
    fn current_scheme(&self) -> Result<ColorScheme> {
        Ok(naming::classify(&gsettings::get(
            INTERFACE_SCHEMA,
            "gtk-theme",
        )?))
    }

    fn apply(&self, target: ColorScheme) -> Result<()> {
        let installed = installed_themes();
        let current = gsettings::get(INTERFACE_SCHEMA, "gtk-theme")?;
        let gtk_theme = naming::resolve(&current, target, &installed)?;
        gsettings::set(INTERFACE_SCHEMA, "gtk-theme", &gtk_theme)?;
        apply_shell_theme(target, &installed);
        Ok(())
    }
}

fn apply_shell_theme(target: ColorScheme, installed: &[String]) {
    let Ok(current) = gsettings::get(SHELL_SCHEMA, "name") else {
        return;
    };
    let resolved = naming::resolve(&current, target, installed);
    match resolved {
        Ok(name) => {
            if let Err(error) = gsettings::set(SHELL_SCHEMA, "name", &name) {
                eprintln!("[os-themes] shell theme not updated: {error:#}");
            }
        }
        Err(error) => eprintln!("[os-themes] shell theme not updated: {error:#}"),
    }
}
