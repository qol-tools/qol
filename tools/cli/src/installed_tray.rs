use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const PORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);
const PORT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const QOL_ENV_PREFIX: &str = "QOL_";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InstalledTray {
    Missing,
    Installed {
        binary: PathBuf,
        version: Option<String>,
    },
}

impl InstalledTray {
    pub(crate) fn probe() -> Self {
        let Ok(binary) = qol_apps::tray_install::installed_binary() else {
            return Self::Missing;
        };
        if !binary.is_file() {
            return Self::Missing;
        }
        let version = qol_artifact::inspect_path(&binary)
            .ok()
            .and_then(|artifact| artifact.slices.into_iter().next())
            .map(|slice| slice.identity.version);
        Self::Installed { binary, version }
    }

    pub(crate) fn version_label(version: Option<&str>) -> String {
        version.map_or_else(|| "version unknown".to_string(), |v| format!("v{v}"))
    }
}

pub(crate) struct RestoreOnExit {
    armed: bool,
}

impl RestoreOnExit {
    pub(crate) fn arm() -> Self {
        Self { armed: true }
    }

    pub(crate) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for RestoreOnExit {
    fn drop(&mut self) {
        if self.armed {
            eprintln!("[qol dev] {}", restore());
        }
    }
}

fn restore() -> String {
    let port_released = wait_for_port_release(PORT_RELEASE_TIMEOUT);
    match restore_plan(port_released, InstalledTray::probe()) {
        RestorePlan::PortBusy => {
            "a tray still holds the qol port; not starting the installed qol-tray".to_string()
        }
        RestorePlan::NothingInstalled => {
            "no installed qol-tray to start (run `qol install`)".to_string()
        }
        RestorePlan::Launch { binary, version } => match launch(&binary) {
            Ok(()) => format!(
                "started installed qol-tray {}",
                InstalledTray::version_label(version.as_deref())
            ),
            Err(error) => format!("failed to start {}: {error}", binary.display()),
        },
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RestorePlan {
    PortBusy,
    NothingInstalled,
    Launch {
        binary: PathBuf,
        version: Option<String>,
    },
}

fn restore_plan(port_released: bool, tray: InstalledTray) -> RestorePlan {
    if !port_released {
        return RestorePlan::PortBusy;
    }
    match tray {
        InstalledTray::Missing => RestorePlan::NothingInstalled,
        InstalledTray::Installed { binary, version } => RestorePlan::Launch { binary, version },
    }
}

fn wait_for_port_release(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while crate::dev_server::api_port_open() || crate::host_facade::qol_tray_running() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(PORT_POLL_INTERVAL);
    }
    true
}

fn launch(binary: &Path) -> io::Result<()> {
    let mut command = Command::new(binary);
    strip_qol_env(&mut command, std::env::vars_os().map(|(key, _)| key));
    if let Some(home) = dirs::home_dir() {
        command.current_dir(home);
    }
    qol_process::spawn_detached(&mut command)
}

fn strip_qol_env(command: &mut Command, keys: impl IntoIterator<Item = OsString>) {
    for key in keys {
        if key.to_string_lossy().starts_with(QOL_ENV_PREFIX) {
            command.env_remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(version: Option<&str>) -> InstalledTray {
        InstalledTray::Installed {
            binary: PathBuf::from("/home/x/.local/bin/qol-tray"),
            version: version.map(str::to_string),
        }
    }

    #[test]
    fn a_held_port_never_starts_a_second_tray() {
        assert_eq!(
            restore_plan(false, installed(Some("3.86.5"))),
            RestorePlan::PortBusy
        );
    }

    #[test]
    fn a_free_port_starts_the_installed_tray() {
        assert_eq!(
            restore_plan(true, installed(Some("3.86.5"))),
            RestorePlan::Launch {
                binary: PathBuf::from("/home/x/.local/bin/qol-tray"),
                version: Some("3.86.5".to_string()),
            }
        );
    }

    #[test]
    fn nothing_installed_starts_nothing() {
        assert_eq!(
            restore_plan(true, InstalledTray::Missing),
            RestorePlan::NothingInstalled
        );
    }

    #[test]
    fn version_label_names_an_unreadable_version() {
        assert_eq!(InstalledTray::version_label(Some("3.86.5")), "v3.86.5");
        assert_eq!(InstalledTray::version_label(None), "version unknown");
    }

    #[test]
    fn launch_strips_every_qol_variable_and_keeps_the_rest() {
        let mut command = Command::new("qol-tray");
        strip_qol_env(
            &mut command,
            [
                OsString::from("QOL_TRAY_DAEMON_LISTENER_FD"),
                OsString::from("QOL_DEV_RESUME_TRAY_PID"),
                OsString::from("DISPLAY"),
            ],
        );
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            removed,
            ["QOL_DEV_RESUME_TRAY_PID", "QOL_TRAY_DAEMON_LISTENER_FD"]
        );
    }
}
