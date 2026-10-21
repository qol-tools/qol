use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DIR: &str = "detached";

fn dir(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(DIR)
}

fn entry(runtime_dir: &Path, pid: u32) -> PathBuf {
    dir(runtime_dir).join(pid.to_string())
}

pub fn register(runtime_dir: &Path, pid: u32) -> io::Result<()> {
    let identity = crate::process_identity(pid)?;
    fs::create_dir_all(dir(runtime_dir))?;
    let staging = dir(runtime_dir).join(format!("{pid}.staging"));
    fs::write(&staging, identity)?;
    fs::rename(staging, entry(runtime_dir, pid))
}

pub fn unregister(runtime_dir: &Path, pid: u32) {
    let _ = fs::remove_file(entry(runtime_dir, pid));
}

pub fn registered(runtime_dir: &Path) -> HashSet<u32> {
    let Ok(entries) = fs::read_dir(dir(runtime_dir)) else {
        return HashSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let pid = path.file_name()?.to_str()?.parse::<u32>().ok()?;
            let identity = fs::read_to_string(&path).ok()?;
            if crate::process_identity_matches(pid, identity.trim()) {
                return Some(pid);
            }
            let _ = fs::remove_file(&path);
            None
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_live_process_stays_listed_until_it_unregisters() {
        let root = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        register(root.path(), pid).unwrap();
        assert_eq!(registered(root.path()), HashSet::from([pid]));
        unregister(root.path(), pid);
        assert!(registered(root.path()).is_empty());
    }

    #[test]
    fn entries_of_replaced_processes_are_pruned_and_other_files_are_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        fs::create_dir_all(dir(root.path())).unwrap();
        fs::write(entry(root.path(), pid), "linux:another-boot:24:1:1").unwrap();
        fs::write(dir(root.path()).join("notes.txt"), "kept").unwrap();
        assert!(registered(root.path()).is_empty());
        assert!(!entry(root.path(), pid).exists());
        assert!(dir(root.path()).join("notes.txt").exists());
    }

    #[test]
    fn a_missing_registry_lists_nothing() {
        let root = tempfile::tempdir().unwrap();
        assert!(registered(&root.path().join("missing")).is_empty());
    }
}
