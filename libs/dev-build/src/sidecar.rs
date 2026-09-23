use std::path::{Path, PathBuf};

pub fn fingerprint_sidecar_path(binary: &Path) -> PathBuf {
    let mut name = binary
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".fingerprint");
    binary.with_file_name(name)
}

pub fn write_fingerprint_sidecar(binary: &Path, fingerprint: &str) -> Result<(), String> {
    let sidecar = fingerprint_sidecar_path(binary);
    let staging = sidecar.with_extension("fingerprint.tmp");
    std::fs::write(&staging, fingerprint).map_err(|error| error.to_string())?;
    std::fs::rename(&staging, &sidecar).map_err(|error| error.to_string())
}

pub fn read_fingerprint_sidecar(binary: &Path) -> Option<String> {
    std::fs::read_to_string(fingerprint_sidecar_path(binary)).ok()
}

pub fn binary_is_fresh(binary: &Path, current_fingerprint: &str) -> bool {
    binary.is_file()
        && read_fingerprint_sidecar(binary).is_some_and(|stored| stored == current_fingerprint)
}

pub fn daemons_needing_restart(spawned: &[(String, PathBuf, String)]) -> Vec<String> {
    spawned
        .iter()
        .filter(|(_, binary, spawn_fingerprint)| {
            read_fingerprint_sidecar(binary).as_deref() != Some(spawn_fingerprint)
        })
        .map(|(daemon, _, _)| daemon.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn binary_with_sidecar(dir: &Path, name: &str, fingerprint: &str) -> PathBuf {
        let binary = dir.join(name);
        fs::write(&binary, b"elf").unwrap();
        write_fingerprint_sidecar(&binary, fingerprint).unwrap();
        binary
    }

    #[test]
    fn sidecar_lives_next_to_the_binary() {
        let binary = Path::new("/tmp/target/debug/launcher");
        assert_eq!(
            fingerprint_sidecar_path(binary),
            Path::new("/tmp/target/debug/launcher.fingerprint")
        );
    }

    #[test]
    fn sidecar_round_trips_and_leaves_no_temp_files() {
        let tmp = TempDir::new().unwrap();
        let binary = binary_with_sidecar(tmp.path(), "launcher", "abc123");
        assert_eq!(
            read_fingerprint_sidecar(&binary),
            Some("abc123".to_string())
        );
        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 2, "unexpected files: {entries:?}");
    }

    #[test]
    fn matching_sidecar_and_existing_binary_is_fresh() {
        let tmp = TempDir::new().unwrap();
        let binary = binary_with_sidecar(tmp.path(), "launcher", "abc123");
        assert!(binary_is_fresh(&binary, "abc123"));
    }

    #[test]
    fn missing_sidecar_is_stale() {
        let tmp = TempDir::new().unwrap();
        let binary = tmp.path().join("launcher");
        fs::write(&binary, b"elf").unwrap();
        assert!(!binary_is_fresh(&binary, "abc123"));
    }

    #[test]
    fn mismatched_sidecar_is_stale() {
        let tmp = TempDir::new().unwrap();
        let binary = binary_with_sidecar(tmp.path(), "launcher", "old");
        assert!(!binary_is_fresh(&binary, "new"));
    }

    #[test]
    fn missing_binary_is_stale_even_with_matching_sidecar() {
        let tmp = TempDir::new().unwrap();
        let binary = binary_with_sidecar(tmp.path(), "launcher", "abc123");
        fs::remove_file(&binary).unwrap();
        assert!(!binary_is_fresh(&binary, "abc123"));
    }

    #[test]
    fn two_checkouts_never_share_freshness() {
        let checkout_a = TempDir::new().unwrap();
        let checkout_b = TempDir::new().unwrap();
        let binary_a = binary_with_sidecar(checkout_a.path(), "launcher", "abc123");
        let binary_b = checkout_b.path().join("launcher");
        fs::write(&binary_b, b"elf").unwrap();
        assert!(binary_is_fresh(&binary_a, "abc123"));
        assert!(!binary_is_fresh(&binary_b, "abc123"));
    }

    #[test]
    fn restart_targets_only_daemons_whose_sidecar_changed() {
        let tmp = TempDir::new().unwrap();
        let launcher = binary_with_sidecar(tmp.path(), "launcher", "v2");
        let monitor = binary_with_sidecar(tmp.path(), "monitor", "v1");
        let spawned = vec![
            ("launcher".to_string(), launcher, "v1".to_string()),
            ("monitor".to_string(), monitor, "v1".to_string()),
        ];
        assert_eq!(daemons_needing_restart(&spawned), vec!["launcher"]);
    }

    #[test]
    fn missing_sidecar_forces_restart() {
        let tmp = TempDir::new().unwrap();
        let binary = tmp.path().join("launcher");
        fs::write(&binary, b"elf").unwrap();
        let spawned = vec![("launcher".to_string(), binary, "v1".to_string())];
        assert_eq!(daemons_needing_restart(&spawned), vec!["launcher"]);
    }
}
