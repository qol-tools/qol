mod cinnamon;
mod gnome;
mod kde;
mod naming;

use anyhow::Result;

use crate::theme::ColorScheme;

pub(super) use cinnamon::Cinnamon;
pub(super) use gnome::Gnome;
pub(super) use kde::Kde;

pub(super) trait DesktopBackend {
    fn current_scheme(&self) -> Result<ColorScheme>;
    fn apply(&self, target: ColorScheme) -> Result<()>;
}

pub(super) fn installed_themes() -> Vec<String> {
    let mut roots = vec![
        std::path::PathBuf::from("/usr/share/themes"),
        std::path::PathBuf::from("/usr/local/share/themes"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(std::path::PathBuf::from(home).join(".themes"));
    }
    let mut names = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Ok(name) = entry.file_name().into_string() {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
    }
    names
}
