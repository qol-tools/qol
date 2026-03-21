use super::Plugin;
use std::path::Path;

#[cfg(unix)]
mod orphan_kill;
pub mod platform;

pub fn kill_orphan_daemons() {
    platform::kill_orphan_daemons();
}

pub fn clean_stale_sockets(plugins: &[Plugin]) {
    platform::clean_stale_sockets(plugins);
}

pub fn save_plugin_pid(pids_dir: &Path, plugin_id: &str, pid: u32) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::write(&path, pid.to_string());
}

pub fn remove_plugin_pid(pids_dir: &Path, plugin_id: &str) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::remove_file(&path);
}

pub fn list_tracked_pids(pids_dir: &Path) -> impl Iterator<Item = (String, u32)> {
    std::fs::read_dir(pids_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "pid" {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let pid: u32 = std::fs::read_to_string(&path).ok()?.trim().parse().ok()?;
            Some((id, pid))
        })
}

pub fn clear_all_pids(pids_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(pids_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "pid") {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(unix)]
pub(crate) use orphan_kill::{kill_from_pid_files, ManagedRoots};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_and_load_pid_roundtrip() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "foo", 12345);

        let pid_file = tmp.path().join("foo.pid");
        assert!(pid_file.exists());

        let content = std::fs::read_to_string(&pid_file).unwrap();
        assert_eq!(content.trim(), "12345");
    }

    #[test]
    fn remove_plugin_pid_deletes_file() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "foo", 12345);
        remove_plugin_pid(tmp.path(), "foo");
        assert!(!tmp.path().join("foo.pid").exists());
    }

    #[test]
    fn remove_plugin_pid_noop_when_missing() {
        let tmp = TempDir::new().unwrap();
        remove_plugin_pid(tmp.path(), "nonexistent");
    }

    #[test]
    fn list_tracked_pids_returns_all_entries() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "a", 111);
        save_plugin_pid(tmp.path(), "b", 222);

        let mut pids: Vec<_> = list_tracked_pids(tmp.path()).collect();
        pids.sort_by_key(|(id, _)| id.clone());

        assert_eq!(pids.len(), 2);
        assert_eq!(pids[0], ("a".to_string(), 111));
        assert_eq!(pids[1], ("b".to_string(), 222));
    }

    #[test]
    fn list_tracked_pids_skips_corrupt_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("bad.pid"), "not-a-number").unwrap();
        save_plugin_pid(tmp.path(), "good", 42);

        let pids: Vec<_> = list_tracked_pids(tmp.path()).collect();
        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0], ("good".to_string(), 42));
    }

    #[test]
    fn clear_all_pids_removes_all_pid_files() {
        let tmp = TempDir::new().unwrap();
        save_plugin_pid(tmp.path(), "a", 1);
        save_plugin_pid(tmp.path(), "b", 2);
        clear_all_pids(tmp.path());
        assert!(list_tracked_pids(tmp.path()).next().is_none());
    }
}
