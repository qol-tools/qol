use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const STATUS_FILE: &str = "windows-hooks.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HookStatus {
    pub(super) pid: u32,
    pub(super) identity: Option<String>,
    pub(super) installed: bool,
    pub(super) error: Option<String>,
    pub(super) rearms: u32,
    pub(super) last_rearm_unix: Option<u64>,
}

impl HookStatus {
    pub(super) fn installed(pid: u32) -> Self {
        Self {
            pid,
            identity: qol_process::process_identity(pid).ok(),
            installed: true,
            error: None,
            rearms: 0,
            last_rearm_unix: None,
        }
    }

    pub(super) fn failed(pid: u32, error: String) -> Self {
        Self {
            installed: false,
            error: Some(error),
            ..Self::installed(pid)
        }
    }

    pub(super) fn rearmed(&mut self, at_unix: u64) {
        self.rearms = self.rearms.saturating_add(1);
        self.last_rearm_unix = Some(at_unix);
    }

    pub(super) fn stopped(&self) -> Self {
        Self {
            installed: false,
            ..self.clone()
        }
    }

    pub(super) fn reporter_alive(&self) -> bool {
        self.identity
            .as_deref()
            .is_some_and(|identity| qol_process::process_identity_matches(self.pid, identity))
    }
}

pub(super) fn write(status: &HookStatus) {
    let Some(path) = path() else {
        return;
    };
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let body = serde_json::to_vec(status).map_err(std::io::Error::other)?;
            std::fs::write(&path, body)
        });
    if let Err(error) = written {
        log::warn!(
            "could not record the Windows hook state in {}: {error}",
            path.display()
        );
    }
}

pub(super) fn read() -> Option<HookStatus> {
    serde_json::from_slice(&std::fs::read(path()?).ok()?).ok()
}

pub(super) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn path() -> Option<PathBuf> {
    let id = qol_config::plugin_id_from_env(crate::cli::PLUGIN_ID);
    Some(
        qol_config::data_dir()?
            .join("plugins")
            .join(id)
            .join(STATUS_FILE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_survives_the_file_round_trip() {
        let mut status = HookStatus::installed(4_242);
        status.rearmed(1_700_000_000);
        let body = serde_json::to_vec(&status).unwrap();
        assert_eq!(
            serde_json::from_slice::<HookStatus>(&body).ok(),
            Some(status.clone())
        );
        assert!(serde_json::from_slice::<HookStatus>(b"not json").is_err());
        let stopped = status.stopped();
        assert!(!stopped.installed);
        assert_eq!(stopped.rearms, 1);
        assert_eq!(stopped.last_rearm_unix, Some(1_700_000_000));
    }

    #[test]
    fn only_the_reporting_process_counts_as_alive() {
        assert!(HookStatus::installed(std::process::id()).reporter_alive());
        let reused = HookStatus {
            identity: Some("another process".into()),
            ..HookStatus::installed(std::process::id())
        };
        assert!(!reused.reporter_alive());
        let unknown = HookStatus {
            identity: None,
            ..HookStatus::installed(std::process::id())
        };
        assert!(!unknown.reporter_alive());
    }

    #[test]
    fn a_failed_install_keeps_its_error_and_is_not_installed() {
        let status = HookStatus::failed(7, "access denied".into());
        assert!(!status.installed);
        assert_eq!(status.error.as_deref(), Some("access denied"));
        assert_eq!(status.rearms, 0);
    }
}
