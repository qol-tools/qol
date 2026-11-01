use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RESPAWN_WAIT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(250);
const REPLACED_SUFFIX: &str = " (deleted)";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Restart {
    NotRunning,
    Restarted { old: u32, new: u32 },
    Pending { old: u32 },
    Foreign { pid: u32 },
}

impl Restart {
    pub(super) fn describe(&self) -> String {
        match self {
            Self::NotRunning => "installed; no daemon was running".to_string(),
            Self::Restarted { old, new } => {
                format!("installed; the tray restarted its daemon ({old} -> {new})")
            }
            Self::Pending { old } => format!(
                "installed; stopped daemon {old}, and the tray has not started the new one yet"
            ),
            Self::Foreign { pid } => format!(
                "installed; pid {pid} in the tray's record is not this plugin, so nothing was stopped: restart the tray"
            ),
        }
    }
}

pub(super) fn restart(pids: &Path, id: &str, binary: &Path) -> Result<Restart> {
    let Some(old) = tracked_pid(pids, id).filter(|pid| qol_process::is_pid_alive(*pid)) else {
        return Ok(Restart::NotRunning);
    };
    if !runs_binary(old, binary) {
        return Ok(Restart::Foreign { pid: old });
    }
    qol_process::signal_term_pid(old).with_context(|| format!("stopping daemon {old} of {id}"))?;
    let deadline = Instant::now() + RESPAWN_WAIT;
    while Instant::now() < deadline {
        std::thread::sleep(POLL);
        let respawned =
            tracked_pid(pids, id).filter(|pid| *pid != old && qol_process::is_pid_alive(*pid));
        if let Some(new) = respawned {
            return Ok(Restart::Restarted { old, new });
        }
    }
    Ok(Restart::Pending { old })
}

fn tracked_pid(pids: &Path, id: &str) -> Option<u32> {
    std::fs::read_to_string(pids.join(format!("{id}.pid")))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn runs_binary(pid: u32, binary: &Path) -> bool {
    let wanted = binary
        .canonicalize()
        .unwrap_or_else(|_| binary.to_path_buf());
    let Ok(exe) = std::fs::read_link(PathBuf::from(format!("/proc/{pid}/exe"))) else {
        return false;
    };
    same_binary(&exe.to_string_lossy(), &wanted.to_string_lossy())
}

fn same_binary(exe: &str, wanted: &str) -> bool {
    exe == wanted || exe.strip_suffix(REPLACED_SUFFIX) == Some(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_record_means_no_daemon_to_restart() {
        let pids = tempfile::tempdir().unwrap();
        let restart = restart(pids.path(), "qol-demo", Path::new("/nowhere/qol-demo")).unwrap();
        assert_eq!(restart, Restart::NotRunning);
    }

    #[test]
    fn a_recorded_pid_of_another_program_is_never_stopped() {
        let pids = tempfile::tempdir().unwrap();
        let own = std::process::id();
        std::fs::write(pids.path().join("qol-demo.pid"), format!("{own}\n")).unwrap();
        assert_eq!(tracked_pid(pids.path(), "qol-demo"), Some(own));
        let restart = restart(pids.path(), "qol-demo", Path::new("/nowhere/qol-demo")).unwrap();
        assert_eq!(restart, Restart::Foreign { pid: own });
        assert!(qol_process::is_pid_alive(own));
    }

    #[test]
    fn only_the_binary_itself_or_its_replaced_file_matches() {
        let wanted = "/home/u/.config/qol-tray/plugins/qol-demo/qol-demo";
        assert!(same_binary(wanted, wanted));
        assert!(same_binary(&format!("{wanted} (deleted)"), wanted));
        assert!(!same_binary(&format!("{wanted}-helper"), wanted));
        assert!(!same_binary(&format!("{wanted}.old (deleted)"), wanted));
    }

    #[test]
    fn describe_names_each_outcome() {
        let lines = [
            Restart::NotRunning,
            Restart::Restarted { old: 1, new: 2 },
            Restart::Pending { old: 1 },
            Restart::Foreign { pid: 1 },
        ]
        .map(|restart| restart.describe());
        assert!(lines.iter().all(|line| line.starts_with("installed")));
        assert!(lines[1].contains("1 -> 2"));
        assert!(lines[3].contains("restart the tray"));
    }
}
