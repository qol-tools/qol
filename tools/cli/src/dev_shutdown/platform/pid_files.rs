use std::fs;
use std::path::{Path, PathBuf};

use crate::dev_shutdown::TrackedDaemonPid;

pub(super) fn runtime_pids_dir() -> PathBuf {
    qol_config::runtime_dir()
        .map(|path| path.join("pids"))
        .unwrap_or_else(|| PathBuf::from(qol_conventions::RUNTIME_PIDS_DIR_PATH))
}

pub(super) fn tracked_pids_from_dir(pids_dir: &Path) -> Vec<TrackedDaemonPid> {
    let mut daemons: Vec<_> = fs::read_dir(pids_dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "pid" {
                return None;
            }
            let plugin_id = path.file_stem()?.to_str()?.to_string();
            let pid = fs::read_to_string(&path).ok()?.trim().parse().ok()?;
            Some(TrackedDaemonPid {
                plugin_id,
                pid,
                executable: None,
            })
        })
        .collect();
    daemons.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then(left.pid.cmp(&right.pid))
    });
    daemons.dedup();
    daemons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_pids_from_dir_reads_valid_pid_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("plugin-z.pid"), "222").unwrap();
        fs::write(tmp.path().join("plugin-a.pid"), "111\n").unwrap();
        fs::write(tmp.path().join("plugin-b.pid"), "not a pid").unwrap();
        fs::write(tmp.path().join("plugin-c.txt"), "333").unwrap();

        let daemons = tracked_pids_from_dir(tmp.path());

        assert_eq!(
            daemons,
            vec![
                TrackedDaemonPid {
                    plugin_id: "plugin-a".to_string(),
                    pid: 111,
                    executable: None,
                },
                TrackedDaemonPid {
                    plugin_id: "plugin-z".to_string(),
                    pid: 222,
                    executable: None,
                },
            ]
        );
    }
}
