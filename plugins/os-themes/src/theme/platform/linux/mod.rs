mod backends;
mod desktop;
mod gsettings;
mod kconfig;

use anyhow::{bail, Result};

use crate::session::{RestoreMode, RestoreReport};
use crate::theme::session as theme_session;
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

pub(crate) fn snapshot_key(schema: &str, key: &str) -> Result<()> {
    let value = gsettings::get(schema, key)?;
    theme_session::record_baseline(schema, key, &value)
}

pub(crate) fn restore(mode: RestoreMode, report: &mut RestoreReport) {
    let Ok(ids) = theme_session::ids() else {
        return;
    };
    for id in ids {
        let Ok(Some(snapshot)) = theme_session::load(&id) else {
            report.unreadable += 1;
            continue;
        };
        if snapshot.mutations == 0 || snapshot.clean {
            report.nothing_to_restore += 1;
            let _ = theme_session::delete(&id);
            continue;
        }
        match gsettings::set(&snapshot.schema, &snapshot.key, &snapshot.value) {
            Ok(()) => {
                match mode {
                    RestoreMode::Exit => {
                        let mut cleaned = snapshot.clone();
                        cleaned.set_clean();
                        let _ = theme_session::write(&cleaned);
                    }
                    RestoreMode::Recovery => {
                        let _ = theme_session::delete(&id);
                    }
                }
                report.restored += 1;
            }
            Err(error) => {
                eprintln!(
                    "[os-themes] failed to restore pre-qol value of {}:{}: {error:#}",
                    snapshot.schema, snapshot.key
                );
                report.failed += 1;
            }
        }
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
            ("plasma", true),
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
