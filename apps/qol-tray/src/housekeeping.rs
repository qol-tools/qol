use anyhow::Result;
use std::path::Path;

pub fn run_startup_cleanup(config_dir: &Path) -> Result<()> {
    migrate_dev_files(config_dir);
    clean_legacy_ephemeral(config_dir);
    clean_stale_staging(config_dir);
    Ok(())
}

fn migrate_dev_files(config_dir: &Path) {
    let dev_dir = config_dir.join("dev");
    let migrations = [
        ("dev-links.json", "links.json"),
        ("dev-build-fingerprints.json", "build-fingerprints.json"),
        ("dev-core-log-controls.json", "core-log-controls.json"),
        ("dev-plugin-log-controls.json", "plugin-log-controls.json"),
    ];

    let extra = [
        ("dev.json", "config.json"),
        ("active-worktree.txt", "active-worktree.txt"),
    ];

    let any_exists = migrations
        .iter()
        .chain(extra.iter())
        .any(|(old, _)| config_dir.join(old).exists());

    if !any_exists {
        return;
    }

    let _ = std::fs::create_dir_all(&dev_dir);

    for (old_name, new_name) in migrations.iter().chain(extra.iter()) {
        let old = config_dir.join(old_name);
        let new = dev_dir.join(new_name);
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

fn clean_legacy_ephemeral(config_dir: &Path) {
    for name in [".daemon-pids", ".plugin-cache.json"] {
        let _ = std::fs::remove_file(config_dir.join(name));
    }
}

fn clean_stale_staging(config_dir: &Path) {
    let plugins_dir = config_dir.join("plugins");
    let Ok(entries) = std::fs::read_dir(&plugins_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if is_stale_staging_dir(&name_str) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn is_stale_staging_dir(name: &str) -> bool {
    name.starts_with('.')
        && (name.contains(".installing.")
            || name.contains(".updating.")
            || name.contains(".backup."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migrate_dev_files_moves_to_dev_subdir() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::write(cfg.join("dev-links.json"), "{}").unwrap();
        std::fs::write(cfg.join("dev-build-fingerprints.json"), "{}").unwrap();

        run_startup_cleanup(cfg).unwrap();

        assert!(!cfg.join("dev-links.json").exists());
        assert!(cfg.join("dev/links.json").exists());
        assert!(!cfg.join("dev-build-fingerprints.json").exists());
        assert!(cfg.join("dev/build-fingerprints.json").exists());
    }

    #[test]
    fn migrate_dev_files_skips_when_target_exists() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::create_dir_all(cfg.join("dev")).unwrap();
        std::fs::write(cfg.join("dev-links.json"), r#"{"old": true}"#).unwrap();
        std::fs::write(cfg.join("dev/links.json"), r#"{"new": true}"#).unwrap();

        run_startup_cleanup(cfg).unwrap();

        let content = std::fs::read_to_string(cfg.join("dev/links.json")).unwrap();
        assert!(content.contains("new"), "should not overwrite existing");
    }

    #[test]
    fn migrate_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::write(cfg.join("dev-links.json"), "{}").unwrap();

        run_startup_cleanup(cfg).unwrap();
        run_startup_cleanup(cfg).unwrap();

        assert!(cfg.join("dev/links.json").exists());
    }

    #[test]
    fn clean_legacy_ephemeral_removes_old_files() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::write(cfg.join(".daemon-pids"), "123").unwrap();
        std::fs::write(cfg.join(".plugin-cache.json"), "{}").unwrap();

        run_startup_cleanup(cfg).unwrap();

        assert!(!cfg.join(".daemon-pids").exists());
        assert!(!cfg.join(".plugin-cache.json").exists());
    }

    #[test]
    fn clean_stale_staging_removes_orphan_dirs() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        let plugins = cfg.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();

        std::fs::create_dir_all(plugins.join(".foo.installing.123.456")).unwrap();
        std::fs::create_dir_all(plugins.join(".bar.updating.789.012")).unwrap();
        std::fs::create_dir_all(plugins.join(".baz.backup.111.222")).unwrap();
        std::fs::create_dir_all(plugins.join("real-plugin")).unwrap();

        run_startup_cleanup(cfg).unwrap();

        assert!(!plugins.join(".foo.installing.123.456").exists());
        assert!(!plugins.join(".bar.updating.789.012").exists());
        assert!(!plugins.join(".baz.backup.111.222").exists());
        assert!(plugins.join("real-plugin").exists());
    }

    #[test]
    fn migrate_dev_json_and_active_worktree() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::write(cfg.join("dev.json"), r#"{"search_paths":[]}"#).unwrap();
        std::fs::write(cfg.join("active-worktree.txt"), "feature-x").unwrap();

        run_startup_cleanup(cfg).unwrap();

        assert!(!cfg.join("dev.json").exists());
        assert!(cfg.join("dev/config.json").exists());
        assert!(!cfg.join("active-worktree.txt").exists());
        assert!(cfg.join("dev/active-worktree.txt").exists());

        let content = std::fs::read_to_string(cfg.join("dev/active-worktree.txt")).unwrap();
        assert_eq!(content, "feature-x");
    }

    #[test]
    fn is_stale_staging_dir_cases() {
        let cases = [
            (".foo.installing.123.456", true),
            (".bar.updating.789.012", true),
            (".baz.backup.111.222", true),
            ("real-plugin", false),
            (".hidden-but-not-staging", false),
            (".foo.installing", false),
        ];
        for (name, expected) in cases {
            assert_eq!(
                is_stale_staging_dir(name),
                expected,
                "is_stale_staging_dir({:?})",
                name
            );
        }
    }
}
