use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

use crate::{FileMigration, MigrationReport};

pub struct V3_16ToV3_17DeviceToOs {
    target_os: &'static str,
}

const FILES: &[&str] = &["shortcuts.json", "task-runner.json"];

impl V3_16ToV3_17DeviceToOs {
    pub fn new_for_os(target_os: &'static str) -> Self {
        Self { target_os }
    }

    pub fn default_for_production() -> Self {
        Self::new_for_os(current_os_subdir())
    }
}

fn current_os_subdir() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

fn legacy_sidecar_path(src: &Path) -> PathBuf {
    let mut name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".legacy");
    src.with_file_name(name)
}

fn list_profile_dirs(profile_dir: &Path) -> Result<Vec<PathBuf>> {
    if !profile_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(profile_dir)
        .with_context(|| format!("reading {}", profile_dir.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("manifest.json").is_file() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

impl FileMigration for V3_16ToV3_17DeviceToOs {
    fn name(&self) -> &'static str {
        "v3.16-to-v3.17-device-to-os"
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        let profile = config_dir.join("profile");
        for root in list_profile_dirs(&profile)? {
            for file in FILES {
                if root.join("device").join(file).is_file() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
        let profile = config_dir.join("profile");
        let mut touched = Vec::new();

        for root in list_profile_dirs(&profile)? {
            for filename in FILES {
                let src = root.join("device").join(filename);
                if !src.exists() {
                    continue;
                }
                if !src.is_file() {
                    log::warn!(
                        "[v3.16-to-v3.17] skipping {} (not a regular file)",
                        src.display()
                    );
                    continue;
                }

                let os_dir = root.join("os").join(self.target_os);
                std::fs::create_dir_all(&os_dir)
                    .with_context(|| format!("creating {}", os_dir.display()))?;
                let dst = os_dir.join(filename);

                if dst.exists() {
                    if !dst.is_file() {
                        return Err(anyhow!(
                            "destination path exists but is not a file: {}",
                            dst.display()
                        ));
                    }
                    let bak = legacy_sidecar_path(&src);
                    if bak.exists() {
                        std::fs::remove_file(&bak).with_context(|| {
                            format!("clearing prior sidecar {}", bak.display())
                        })?;
                    }
                    std::fs::rename(&src, &bak).with_context(|| {
                        format!(
                            "destination {} already exists; archiving legacy {} to {}",
                            dst.display(),
                            src.display(),
                            bak.display()
                        )
                    })?;
                    log::warn!(
                        "[v3.16-to-v3.17] {} already exists; preserved legacy source at {}",
                        dst.display(),
                        bak.display()
                    );
                } else {
                    std::fs::rename(&src, &dst).with_context(|| {
                        format!("renaming {} to {}", src.display(), dst.display())
                    })?;
                    touched.push(dst);
                }
            }
        }

        Ok(MigrationReport {
            name: self.name().to_string(),
            archived: touched,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OS_MAC: &str = "macos";
    const OS_LINUX: &str = "linux";

    fn migration(target_os: &'static str) -> V3_16ToV3_17DeviceToOs {
        V3_16ToV3_17DeviceToOs::new_for_os(target_os)
    }

    fn empty_archive(dir: &Path) -> PathBuf {
        let p = dir.join("archive").join("v3.16-to-v3.17-test");
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn setup_profile(config_dir: &Path, name: &str) -> PathBuf {
        let root = config_dir.join("profile").join(name);
        write(&root.join("manifest.json"), b"{\"version\":2}");
        root
    }

    #[test]
    fn applies_returns_false_when_profile_dir_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn applies_returns_false_when_no_profiles_exist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("profile")).unwrap();
        assert!(!migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn applies_returns_false_when_only_device_sync_present() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("sync").join("state.json"), b"{}");
        write(&root.join("device").join("sync").join("toggles.json"), b"{}");
        assert!(!migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn applies_returns_true_when_device_shortcuts_present() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{}");
        assert!(migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn applies_returns_true_when_device_task_runner_present() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("task-runner.json"), b"{}");
        assert!(migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn applies_returns_true_when_only_one_of_several_profiles_has_legacy() {
        let dir = tempfile::tempdir().unwrap();
        setup_profile(dir.path(), "default");
        let work = setup_profile(dir.path(), "work");
        write(&work.join("device").join("shortcuts.json"), b"{}");
        assert!(migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn applies_ignores_dirs_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let bogus = dir.path().join("profile").join("not-a-profile");
        write(&bogus.join("device").join("shortcuts.json"), b"{}");
        assert!(!migration(OS_MAC).applies(dir.path()).unwrap());
    }

    #[test]
    fn migrate_moves_device_shortcuts_to_os_slot_for_current_os() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root.join("device").join("shortcuts.json");
        write(&src, br#"{"shortcuts":[{"key":"a"}]}"#);

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(!src.exists(), "src must be moved");
        let dst = root.join("os").join(OS_MAC).join("shortcuts.json");
        assert!(dst.is_file(), "lands under os/<target_os>/");
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            br#"{"shortcuts":[{"key":"a"}]}"#,
            "content preserved verbatim"
        );
    }

    #[test]
    fn migrate_moves_task_runner_with_same_semantics_as_shortcuts() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(
            &root.join("device").join("task-runner.json"),
            br#"{"actions":{"build":{"command":"cargo build","timeout":120}}}"#,
        );

        migration(OS_LINUX)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(!root.join("device").join("task-runner.json").exists());
        assert!(root.join("os").join(OS_LINUX).join("task-runner.json").is_file());
    }

    #[test]
    fn migrate_handles_both_files_in_one_profile() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{\"a\":1}");
        write(&root.join("device").join("task-runner.json"), b"{\"b\":2}");

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        for filename in FILES {
            assert!(root.join("os").join(OS_MAC).join(filename).is_file());
            assert!(!root.join("device").join(filename).exists());
        }
    }

    #[test]
    fn migrate_handles_only_shortcuts_without_task_runner() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{}");

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(root.join("os").join(OS_MAC).join("shortcuts.json").is_file());
        assert!(!root.join("os").join(OS_MAC).join("task-runner.json").exists());
    }

    #[test]
    fn migrate_handles_multiple_profiles_independently() {
        let dir = tempfile::tempdir().unwrap();
        let default = setup_profile(dir.path(), "default");
        let work = setup_profile(dir.path(), "work");
        write(&default.join("device").join("shortcuts.json"), b"\"d\"");
        write(&work.join("device").join("shortcuts.json"), b"\"w\"");

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert_eq!(
            std::fs::read(default.join("os").join(OS_MAC).join("shortcuts.json")).unwrap(),
            b"\"d\""
        );
        assert_eq!(
            std::fs::read(work.join("os").join(OS_MAC).join("shortcuts.json")).unwrap(),
            b"\"w\""
        );
    }

    #[test]
    fn migrate_creates_os_dir_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{}");
        assert!(!root.join("os").exists(), "precondition: no os/ dir");

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(root.join("os").join(OS_MAC).is_dir());
    }

    #[test]
    fn migrate_does_not_touch_other_os_slot() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"\"mac\"");
        write(
            &root.join("os").join(OS_LINUX).join("shortcuts.json"),
            b"\"linux-from-other-machine\"",
        );

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert_eq!(
            std::fs::read(root.join("os").join(OS_MAC).join("shortcuts.json")).unwrap(),
            b"\"mac\"",
            "this OS's slot got our migrated data"
        );
        assert_eq!(
            std::fs::read(root.join("os").join(OS_LINUX).join("shortcuts.json")).unwrap(),
            b"\"linux-from-other-machine\"",
            "the other OS's slot came down via sync; must not be touched"
        );
    }

    #[test]
    fn migrate_archives_src_to_sidecar_when_destination_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root.join("device").join("shortcuts.json");
        write(&src, b"\"src-has-real-user-data\"");
        write(
            &root.join("os").join(OS_MAC).join("shortcuts.json"),
            b"\"already-there\"",
        );

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(!src.exists(), "src must be moved out of the legacy path");
        let bak = root.join("device").join("shortcuts.json.legacy");
        assert!(
            bak.is_file(),
            "src must be preserved as a .legacy sidecar, never deleted - the user may need it"
        );
        assert_eq!(
            std::fs::read(&bak).unwrap(),
            b"\"src-has-real-user-data\"",
            "legacy sidecar must hold the original src bytes verbatim"
        );
        assert_eq!(
            std::fs::read(root.join("os").join(OS_MAC).join("shortcuts.json")).unwrap(),
            b"\"already-there\"",
            "existing OS-slot file wins; migration must not clobber"
        );
    }

    #[test]
    fn migrate_overwrites_an_existing_legacy_sidecar_rather_than_erroring() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        let src = root.join("device").join("shortcuts.json");
        write(&src, b"\"newer-src\"");
        write(
            &root.join("os").join(OS_MAC).join("shortcuts.json"),
            b"\"dst\"",
        );
        write(
            &root.join("device").join("shortcuts.json.legacy"),
            b"\"older-sidecar-from-prior-attempt\"",
        );

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        let bak = root.join("device").join("shortcuts.json.legacy");
        assert_eq!(
            std::fs::read(&bak).unwrap(),
            b"\"newer-src\"",
            "newer src replaces stale sidecar so this migration stays idempotent on retry"
        );
    }

    #[test]
    fn migrate_errors_when_destination_path_is_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{}");
        std::fs::create_dir_all(root.join("os").join(OS_MAC).join("shortcuts.json")).unwrap();

        let err = migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("not a file"),
            "expected diagnostic about dest type, got: {msg}"
        );
    }

    #[test]
    fn migrate_preserves_malformed_json_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"not even json {");

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert_eq!(
            std::fs::read(root.join("os").join(OS_MAC).join("shortcuts.json")).unwrap(),
            b"not even json {",
        );
    }

    #[test]
    fn migrate_leaves_device_sync_subtree_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{}");
        write(&root.join("device").join("sync").join("state.json"), b"{\"x\":1}");
        write(&root.join("device").join("sync").join("toggles.json"), b"{\"y\":2}");

        migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(root.join("device").join("sync").join("state.json").is_file());
        assert!(root.join("device").join("sync").join("toggles.json").is_file());
        assert_eq!(
            std::fs::read(root.join("device").join("sync").join("state.json")).unwrap(),
            b"{\"x\":1}",
            "device/sync content untouched - it is genuinely per-machine"
        );
    }

    #[test]
    fn migrate_idempotent_when_run_twice_back_to_back() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{\"v\":1}");

        let m = migration(OS_MAC);
        m.migrate(dir.path(), &empty_archive(dir.path())).unwrap();
        let first = std::fs::read(root.join("os").join(OS_MAC).join("shortcuts.json")).unwrap();

        m.migrate(dir.path(), &empty_archive(dir.path())).unwrap();
        let second = std::fs::read(root.join("os").join(OS_MAC).join("shortcuts.json")).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn migrate_writes_to_macos_slot_when_target_is_macos() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"\"m\"");
        migration("macos")
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();
        assert!(root.join("os").join("macos").join("shortcuts.json").is_file());
    }

    #[test]
    fn migrate_writes_to_linux_slot_when_target_is_linux() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"\"l\"");
        migration("linux")
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();
        assert!(root.join("os").join("linux").join("shortcuts.json").is_file());
    }

    #[test]
    fn migrate_writes_to_windows_slot_when_target_is_windows() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"\"w\"");
        migration("windows")
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();
        assert!(root.join("os").join("windows").join("shortcuts.json").is_file());
    }

    #[test]
    fn migrate_report_lists_only_freshly_moved_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{}");
        write(&root.join("device").join("task-runner.json"), b"{}");

        let report = migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert_eq!(report.archived.len(), 2);
        let names: Vec<String> = report
            .archived
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().any(|n| n == "shortcuts.json"));
        assert!(names.iter().any(|n| n == "task-runner.json"));
    }

    #[test]
    fn migrate_report_is_empty_when_destination_already_exists_for_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = setup_profile(dir.path(), "default");
        write(&root.join("device").join("shortcuts.json"), b"{}");
        write(&root.join("os").join(OS_MAC).join("shortcuts.json"), b"{}");

        let report = migration(OS_MAC)
            .migrate(dir.path(), &empty_archive(dir.path()))
            .unwrap();

        assert!(report.archived.is_empty(), "no new files; just src cleanup");
    }
}
