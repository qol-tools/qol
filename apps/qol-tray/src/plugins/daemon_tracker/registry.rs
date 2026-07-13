use std::collections::HashSet;
use std::path::Path;

pub(crate) fn register(pids_dir: &Path, plugin_id: &str, pid: u32) {
    write_pid_file(pids_dir, plugin_id, pid);
}

pub(crate) fn unregister(pids_dir: &Path, plugin_id: &str, _pid: u32) {
    remove_pid_file(pids_dir, plugin_id);
}

pub(crate) fn clear_all(pids_dir: &Path) {
    clear_pid_files(pids_dir);
}

pub(crate) fn tracked_pids(pids_dir: &Path) -> impl Iterator<Item = (String, u32)> {
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

pub(crate) fn tracked_pid_set(pids_dir: &Path) -> HashSet<i32> {
    tracked_pids(pids_dir).map(|(_, pid)| pid as i32).collect()
}

fn write_pid_file(pids_dir: &Path, plugin_id: &str, pid: u32) {
    let _ = std::fs::create_dir_all(pids_dir);
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::write(&path, pid.to_string());
}

fn remove_pid_file(pids_dir: &Path, plugin_id: &str) {
    let path = pids_dir.join(format!("{}.pid", plugin_id));
    let _ = std::fs::remove_file(&path);
}

fn clear_pid_files(pids_dir: &Path) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn register_writes_pid_file() {
        let tmp = TempDir::new().unwrap();
        register(tmp.path(), "foo", 12345);

        let content = std::fs::read_to_string(tmp.path().join("foo.pid")).unwrap();
        assert_eq!(content.trim(), "12345");
    }

    #[test]
    fn unregister_removes_pid_file() {
        let tmp = TempDir::new().unwrap();
        register(tmp.path(), "foo", 12345);
        unregister(tmp.path(), "foo", 12345);
        assert!(!tmp.path().join("foo.pid").exists());
    }

    #[test]
    fn unregister_missing_is_noop() {
        let tmp = TempDir::new().unwrap();
        unregister(tmp.path(), "nonexistent", 1);
    }

    #[test]
    fn tracked_pids_returns_all_entries() {
        let tmp = TempDir::new().unwrap();
        register(tmp.path(), "a", 111);
        register(tmp.path(), "b", 222);

        let mut pids: Vec<_> = tracked_pids(tmp.path()).collect();
        pids.sort_by_key(|(id, _)| id.clone());

        assert_eq!(pids, vec![("a".to_string(), 111), ("b".to_string(), 222)]);
    }

    #[test]
    fn tracked_pids_skips_corrupt_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("bad.pid"), "not-a-number").unwrap();
        register(tmp.path(), "good", 42);

        let pids: Vec<_> = tracked_pids(tmp.path()).collect();
        assert_eq!(pids, vec![("good".to_string(), 42)]);
    }

    #[test]
    fn clear_all_removes_pid_files() {
        let tmp = TempDir::new().unwrap();
        register(tmp.path(), "a", 1);
        register(tmp.path(), "b", 2);
        clear_all(tmp.path());
        assert!(tracked_pids(tmp.path()).next().is_none());
    }
}
