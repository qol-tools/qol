mod backends;
mod desktop;
mod gsettings;
mod kconfig;

use anyhow::{bail, Result};

use crate::session::{RestoreMode, RestoreReport};
use crate::theme::ColorScheme;

use super::snapshot as theme_session;
use super::ThemePlatform;
use backends::DesktopBackend;
use desktop::{classify_desktop, DesktopEnvironment};

pub struct Platform;

impl ThemePlatform for Platform {
    fn current_scheme(&self) -> Result<ColorScheme> {
        detect_backend()?.current_scheme()
    }

    fn apply_scheme(&self, target: ColorScheme) -> Result<()> {
        detect_backend()?.apply(target)
    }

    fn restore(&self, mode: RestoreMode, report: &mut RestoreReport) {
        restore(mode, report);
    }
}

impl Platform {
    pub(crate) fn desktop_name(raw: &str) -> Option<&'static str> {
        match classify_desktop(raw) {
            DesktopEnvironment::Gnome => Some("GNOME"),
            DesktopEnvironment::Cinnamon => Some("Cinnamon"),
            DesktopEnvironment::Kde => Some("KDE"),
            DesktopEnvironment::Unknown => None,
        }
    }
}

pub(crate) fn snapshot_key(schema: &str, key: &str) -> Result<()> {
    let value = gsettings::get(schema, key)?;
    theme_session::record_baseline(schema, key, &value)
}

fn restore(mode: RestoreMode, report: &mut RestoreReport) {
    theme_session::restore(mode, report, |snapshot| {
        gsettings::set(&snapshot.schema, &snapshot.key, &snapshot.value)
    });
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

#[cfg(test)]
mod exit_restore_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::theme_session;
    use crate::session::restore_exit_when;
    use crate::ENV_LOCK;

    const SCHEMA: &str = "org.gnome.desktop.interface";
    const KEY: &str = "color-scheme";

    struct Sandbox {
        root: std::path::PathBuf,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "os-themes-daemon-run-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("bin")).expect("create sandbox bin dir");
            Sandbox { root }
        }

        fn fake_gsettings(&self) {
            let path = self.root.join("bin").join("gsettings");
            let log = self.root.join("gsettings.log");
            std::fs::write(
                &path,
                format!("#!/bin/sh\necho \"$@\" >> \"{}\"\nexit 0\n", log.display()),
            )
            .expect("write fake gsettings");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("make fake gsettings executable");
        }

        fn with_env(&self, f: impl FnOnce()) {
            let _guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous_path = std::env::var_os("PATH");
            let previous_data = std::env::var_os("XDG_DATA_HOME");
            std::env::set_var("PATH", self.root.join("bin"));
            std::env::set_var("XDG_DATA_HOME", self.root.join("data"));
            f();
            match previous_path {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
            match previous_data {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn seed_baseline(value: &str) {
        let id = theme_session::id_for(SCHEMA, KEY);
        let store = theme_session::store();
        let _ = store.delete(&id);
        theme_session::record_baseline(SCHEMA, KEY, value).unwrap();
    }

    fn load_snapshot() -> Option<theme_session::ThemeSnapshot> {
        theme_session::load(&theme_session::id_for(SCHEMA, KEY)).unwrap()
    }

    #[test]
    fn portable_exit_restores_the_baseline_and_marks_the_snapshot_clean() {
        let sandbox = Sandbox::new("portable");
        sandbox.fake_gsettings();
        sandbox.with_env(|| {
            seed_baseline("prefer-dark");
            restore_exit_when(true);
            let log = std::fs::read_to_string(sandbox.root.join("gsettings.log")).unwrap();
            assert_eq!(
                log,
                format!("set {SCHEMA} {KEY} prefer-dark\n"),
                "a portable exit returns the pre-qol baseline to the host"
            );
            let snapshot =
                load_snapshot().expect("the snapshot stays on disk after a portable exit");
            assert!(
                snapshot.clean,
                "a portable exit marks the baseline snapshot clean"
            );
        });
    }

    #[test]
    fn resident_exit_leaves_the_host_untouched_with_the_snapshot_still_stored() {
        let sandbox = Sandbox::new("resident");
        sandbox.fake_gsettings();
        sandbox.with_env(|| {
            seed_baseline("prefer-dark");
            restore_exit_when(false);
            let log = std::fs::read_to_string(sandbox.root.join("gsettings.log"))
                .unwrap_or_default();
            assert_eq!(
                log, "",
                "a resident exit must never write the host"
            );
            let snapshot = load_snapshot().expect("a resident exit keeps the baseline snapshot on disk");
            assert!(
                !snapshot.clean,
                "a resident exit leaves the snapshot dirty so disabling residency later can restore it"
            );
        });
    }
}
