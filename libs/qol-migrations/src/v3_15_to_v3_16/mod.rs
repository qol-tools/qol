use crate::fs_util::move_into_archive;
use crate::{Migration, MigrationReport};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(crate) struct V3_15ToV3_16;

const TOP_LEGACY_FILES: &[&str] = &["manifest.json", "plugins.lock.json"];
const TOP_LEGACY_DIRS: &[&str] = &["plugin-configs"];

struct InteriorMove {
    from_relative: &'static str,
    to_relative_template: &'static str,
}

const INTERIOR_MOVES: &[InteriorMove] = &[
    InteriorMove {
        from_relative: "core/hotkeys.json",
        to_relative_template: "os/{os}/hotkeys.json",
    },
    InteriorMove {
        from_relative: "core/shortcuts.json",
        to_relative_template: "device/shortcuts.json",
    },
    InteriorMove {
        from_relative: "core/task-runner.json",
        to_relative_template: "device/task-runner.json",
    },
    InteriorMove {
        from_relative: "plugin-configs",
        to_relative_template: "core/plugin-configs",
    },
    InteriorMove {
        from_relative: "plugins.lock.json",
        to_relative_template: "core/plugins.lock.json",
    },
];

fn current_os_subdir() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

fn resolve_template(template: &str) -> String {
    template.replace("{os}", current_os_subdir())
}

fn is_empty_dir(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    Ok(std::fs::read_dir(path)
        .with_context(|| format!("reading {}", path.display()))?
        .next()
        .is_none())
}

fn list_profile_dirs(profile_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(profile_dir)
        .with_context(|| format!("reading {}", profile_dir.display()))?;
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("manifest.json").is_file() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

impl Migration for V3_15ToV3_16 {
    fn name(&self) -> &'static str {
        "v3.15-to-v3.16"
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        let profile = config_dir.join("profile");
        if !profile.join("registry.json").exists() {
            return Ok(false);
        }
        let has_top_legacy = TOP_LEGACY_FILES
            .iter()
            .map(|name| profile.join(name))
            .any(|p| p.is_file())
            || TOP_LEGACY_DIRS
                .iter()
                .map(|name| profile.join(name))
                .any(|p| p.is_dir());
        if has_top_legacy {
            return Ok(true);
        }
        for profile_root in list_profile_dirs(&profile)? {
            for mv in INTERIOR_MOVES {
                if profile_root.join(mv.from_relative).exists() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport> {
        let profile = config_dir.join("profile");
        let mut archived = Vec::new();

        for name in TOP_LEGACY_FILES {
            let path = profile.join(name);
            if path.is_file() {
                archived.push(move_into_archive(&path, archive_dir)?);
            }
        }
        for name in TOP_LEGACY_DIRS {
            let path = profile.join(name);
            if path.is_dir() {
                archived.push(move_into_archive(&path, archive_dir)?);
            }
        }

        for profile_root in list_profile_dirs(&profile)? {
            let profile_name = profile_root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            for mv in INTERIOR_MOVES {
                let from = profile_root.join(mv.from_relative);
                if !from.exists() {
                    continue;
                }
                let to = profile_root.join(resolve_template(mv.to_relative_template));
                if to.exists() && !is_empty_dir(&to)? {
                    let archived_path = archive_dir.join(&profile_name).join(mv.from_relative);
                    if let Some(parent) = archived_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    move_into_archive(&from, archived_path.parent().unwrap())?;
                    archived.push(archived_path);
                    continue;
                }
                if to.exists() {
                    std::fs::remove_dir(&to)
                        .with_context(|| format!("removing empty stub {}", to.display()))?;
                }
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                std::fs::rename(&from, &to)
                    .with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
                archived.push(to);
            }
        }

        Ok(MigrationReport {
            name: self.name().to_string(),
            archived,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn applies_when_registry_and_legacy_files_coexist() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        write(&profile.join("registry.json"), b"{}");
        write(&profile.join("manifest.json"), b"{}");

        assert!(V3_15ToV3_16.applies(dir.path()).unwrap());
    }

    #[test]
    fn does_not_apply_without_registry() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        write(&profile.join("manifest.json"), b"{}");

        assert!(!V3_15ToV3_16.applies(dir.path()).unwrap());
    }

    #[test]
    fn does_not_apply_when_registry_exists_alone() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        write(&profile.join("registry.json"), b"{}");

        assert!(!V3_15ToV3_16.applies(dir.path()).unwrap());
    }

    #[test]
    fn migrate_archives_all_legacy_targets() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        let archive_dir = dir.path().join("archive").join("test");
        std::fs::create_dir_all(&archive_dir).unwrap();

        write(&profile.join("registry.json"), b"{}");
        write(&profile.join("manifest.json"), b"{\"version\":1}");
        write(&profile.join("plugins.lock.json"), b"{}");
        write(&profile.join("plugin-configs").join("foo.json"), b"{}");

        let report = V3_15ToV3_16.migrate(dir.path(), &archive_dir).unwrap();

        assert_eq!(report.archived.len(), 3);
        assert!(!profile.join("manifest.json").exists());
        assert!(!profile.join("plugins.lock.json").exists());
        assert!(!profile.join("plugin-configs").exists());
        assert!(profile.join("registry.json").exists(), "registry must survive");
        assert!(archive_dir.join("manifest.json").exists());
        assert!(archive_dir.join("plugins.lock.json").exists());
        assert!(archive_dir.join("plugin-configs").join("foo.json").exists());
    }

    #[test]
    fn applies_when_interior_files_at_legacy_paths() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        write(&profile.join("registry.json"), b"{}");
        let default = profile.join("default");
        write(&default.join("manifest.json"), b"{\"version\":1}");
        write(&default.join("core").join("hotkeys.json"), b"{}");

        assert!(V3_15ToV3_16.applies(dir.path()).unwrap());
    }

    #[test]
    fn does_not_apply_when_interior_files_already_at_new_paths() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        write(&profile.join("registry.json"), b"{}");
        let default = profile.join("default");
        write(&default.join("manifest.json"), b"{\"version\":1}");
        write(
            &default.join("os").join(current_os_subdir()).join("hotkeys.json"),
            b"{}",
        );
        write(&default.join("device").join("shortcuts.json"), b"{}");
        write(&default.join("core").join("plugin-configs").join("foo.json"), b"{}");

        assert!(!V3_15ToV3_16.applies(dir.path()).unwrap());
    }

    #[test]
    fn migrate_relocates_interior_files_to_new_split() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        let archive_dir = dir.path().join("archive").join("test");
        std::fs::create_dir_all(&archive_dir).unwrap();

        write(&profile.join("registry.json"), b"{}");
        let default = profile.join("default");
        write(&default.join("manifest.json"), b"{\"version\":1}");
        write(&default.join("core").join("hotkeys.json"), b"{\"hotkeys\":[]}");
        write(&default.join("core").join("shortcuts.json"), b"{\"shortcuts\":[]}");
        write(&default.join("core").join("task-runner.json"), b"{\"actions\":{}}");
        write(&default.join("plugin-configs").join("foo.json"), b"{}");

        V3_15ToV3_16.migrate(dir.path(), &archive_dir).unwrap();

        let os_dir = default.join("os").join(current_os_subdir());
        assert!(os_dir.join("hotkeys.json").is_file(), "hotkeys at os/<os>");
        assert!(default.join("device").join("shortcuts.json").is_file());
        assert!(default.join("device").join("task-runner.json").is_file());
        assert!(default.join("core").join("plugin-configs").join("foo.json").is_file());
        assert!(!default.join("core").join("hotkeys.json").exists(), "old hotkeys removed");
        assert!(!default.join("core").join("shortcuts.json").exists());
        assert!(!default.join("core").join("task-runner.json").exists());
        assert!(!default.join("plugin-configs").exists(), "old plugin-configs dir removed");
    }

    #[test]
    fn migrate_handles_multiple_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        let archive_dir = dir.path().join("archive").join("test");
        std::fs::create_dir_all(&archive_dir).unwrap();

        write(&profile.join("registry.json"), b"{}");
        for name in ["default", "work"] {
            let p = profile.join(name);
            write(&p.join("manifest.json"), b"{\"version\":1}");
            write(&p.join("core").join("hotkeys.json"), b"{\"hotkeys\":[]}");
        }

        V3_15ToV3_16.migrate(dir.path(), &archive_dir).unwrap();

        for name in ["default", "work"] {
            let p = profile.join(name);
            assert!(
                p.join("os").join(current_os_subdir()).join("hotkeys.json").is_file(),
                "{name}: hotkeys at os/<os>"
            );
            assert!(!p.join("core").join("hotkeys.json").exists(), "{name}: old removed");
        }
    }

    #[test]
    fn migrate_replaces_empty_destination_stub() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        let archive_dir = dir.path().join("archive").join("test");
        std::fs::create_dir_all(&archive_dir).unwrap();

        write(&profile.join("registry.json"), b"{}");
        let default = profile.join("default");
        write(&default.join("manifest.json"), b"{\"version\":1}");
        write(&default.join("plugin-configs").join("foo.json"), b"{\"real\":true}");
        std::fs::create_dir_all(default.join("core").join("plugin-configs")).unwrap();

        V3_15ToV3_16.migrate(dir.path(), &archive_dir).unwrap();

        let restored = default.join("core").join("plugin-configs").join("foo.json");
        assert!(restored.is_file(), "real config restored at new path");
        let content = std::fs::read_to_string(&restored).unwrap();
        assert_eq!(content, "{\"real\":true}", "real content preserved");
        assert!(!default.join("plugin-configs").exists(), "old dir removed");
    }

    #[test]
    fn migrate_archives_interior_file_when_destination_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        let archive_dir = dir.path().join("archive").join("test");
        std::fs::create_dir_all(&archive_dir).unwrap();

        write(&profile.join("registry.json"), b"{}");
        let default = profile.join("default");
        write(&default.join("manifest.json"), b"{\"version\":1}");
        write(&default.join("core").join("hotkeys.json"), b"old");
        write(
            &default.join("os").join(current_os_subdir()).join("hotkeys.json"),
            b"new",
        );

        V3_15ToV3_16.migrate(dir.path(), &archive_dir).unwrap();

        let kept = std::fs::read_to_string(
            default.join("os").join(current_os_subdir()).join("hotkeys.json"),
        )
        .unwrap();
        assert_eq!(kept, "new", "destination wins on conflict");
        assert!(!default.join("core").join("hotkeys.json").exists());
        assert!(archive_dir.join("default").join("core").join("hotkeys.json").is_file());
    }
}
