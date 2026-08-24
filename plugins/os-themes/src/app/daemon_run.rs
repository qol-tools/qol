use std::sync::{mpsc, Arc};

use anyhow::{ensure, Result};

#[cfg(target_os = "linux")]
use qol_host_fixes::residency::HostResidency;

use super::daemon;
use crate::cursor::{CursorPlatform, Platform, RunState};

pub fn run() -> Result<()> {
    if daemon::send_ping() {
        return Ok(());
    }

    Platform.install_signal_handlers();

    #[cfg(target_os = "linux")]
    crate::session::recover();

    let control = Arc::new(RunState::new());
    let (tx, rx) = mpsc::channel();
    ensure!(
        daemon::start_listener(tx),
        "failed to start daemon listener"
    );

    let listener_control = Arc::clone(&control);
    std::thread::spawn(move || handle_daemon_commands(rx, listener_control));

    let result = supervise_effect(control);

    #[cfg(target_os = "linux")]
    restore_on_exit(current_residency());

    result
}

#[cfg(target_os = "linux")]
type ResidencyCheck = Arc<dyn Fn() -> bool + Send + Sync>;

#[cfg(target_os = "linux")]
fn current_residency() -> ResidencyCheck {
    Arc::new(|| HostResidency::current().is_resident())
}

#[cfg(target_os = "linux")]
fn restore_on_exit(is_resident: ResidencyCheck) {
    if !is_resident() {
        crate::session::restore_exit();
    }
}

fn supervise_effect(control: Arc<RunState>) -> Result<()> {
    let effect = Platform.create_effect();

    loop {
        control.reset();
        let config = crate::config::load();
        let result = effect.run(&config, control.as_ref());
        if control.reload_requested() {
            continue;
        }

        daemon::cleanup();
        return result;
    }
}

fn handle_daemon_commands(rx: mpsc::Receiver<daemon::Command>, control: Arc<RunState>) {
    while let Ok(command) = rx.recv() {
        match command {
            daemon::Command::Kill => {
                control.request_shutdown();
                break;
            }
            daemon::Command::Reload => control.request_reload(),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
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
        let id = crate::theme::session::id_for(SCHEMA, KEY);
        let store = crate::theme::session::store();
        let _ = store.delete(&id);
        crate::theme::session::record_baseline(SCHEMA, KEY, value).unwrap();
    }

    fn load_snapshot() -> Option<crate::theme::session::ThemeSnapshot> {
        crate::theme::session::load(&crate::theme::session::id_for(SCHEMA, KEY)).unwrap()
    }

    fn resident() -> ResidencyCheck {
        Arc::new(|| true)
    }

    fn portable() -> ResidencyCheck {
        Arc::new(|| false)
    }

    #[test]
    fn portable_exit_restores_the_baseline_and_marks_the_snapshot_clean() {
        let sandbox = Sandbox::new("portable");
        sandbox.fake_gsettings();
        sandbox.with_env(|| {
            seed_baseline("prefer-dark");
            restore_on_exit(portable());
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
            restore_on_exit(resident());
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
