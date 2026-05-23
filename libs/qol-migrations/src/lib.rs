use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

mod fs_util;
mod v3_15_to_v3_16;

pub use fs_util::archive_path;

pub trait Migration: Send + Sync {
    fn name(&self) -> &'static str;
    fn applies(&self, config_dir: &Path) -> Result<bool>;
    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport>;
}

#[derive(Debug, Clone)]
pub struct MigrationReport {
    pub name: String,
    pub archived: Vec<PathBuf>,
}

pub struct Registry {
    entries: Vec<Box<dyn Migration>>,
}

impl Registry {
    pub fn current() -> Self {
        Self {
            entries: vec![Box::new(v3_15_to_v3_16::V3_15ToV3_16)],
        }
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|m| m.name()).collect()
    }
}

pub fn run_if_needed(config_dir: &Path) -> Result<Vec<MigrationReport>> {
    run_with_registry(config_dir, &Registry::current())
}

pub fn run_with_registry(config_dir: &Path, registry: &Registry) -> Result<Vec<MigrationReport>> {
    let mut reports = Vec::new();
    for migration in &registry.entries {
        let applies = migration
            .applies(config_dir)
            .with_context(|| format!("checking applicability of {}", migration.name()))?;
        if !applies {
            continue;
        }
        let archive_dir = archive_path(config_dir, migration.name())?;
        std::fs::create_dir_all(&archive_dir).context("creating archive dir")?;
        let report = migration
            .migrate(config_dir, &archive_dir)
            .with_context(|| format!("running migration {}", migration.name()))?;
        log::info!(
            "[qol-migrations] applied {} (archived {} paths to {})",
            report.name,
            report.archived.len(),
            archive_dir.display()
        );
        reports.push(report);
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    struct RecordingMigration {
        name: &'static str,
        applies_response: bool,
        calls: Mutex<Vec<PathBuf>>,
    }

    impl Migration for RecordingMigration {
        fn name(&self) -> &'static str {
            self.name
        }

        fn applies(&self, _config_dir: &Path) -> Result<bool> {
            Ok(self.applies_response)
        }

        fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
            self.calls.lock().unwrap().push(config_dir.to_path_buf());
            Ok(MigrationReport {
                name: self.name.to_string(),
                archived: vec![],
            })
        }
    }

    fn make_registry(entries: Vec<Box<dyn Migration>>) -> Registry {
        Registry { entries }
    }

    #[test]
    fn skips_migrations_that_do_not_apply() {
        let dir = tempfile::tempdir().unwrap();
        let migration = RecordingMigration {
            name: "skip-me",
            applies_response: false,
            calls: Mutex::new(vec![]),
        };
        let registry = make_registry(vec![Box::new(migration)]);
        let reports = run_with_registry(dir.path(), &registry).unwrap();
        assert!(reports.is_empty());
    }

    #[test]
    fn runs_migrations_that_apply_and_archives_into_dated_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let migration = RecordingMigration {
            name: "v0.0-to-v0.1",
            applies_response: true,
            calls: Mutex::new(vec![]),
        };
        let registry = make_registry(vec![Box::new(migration)]);
        let reports = run_with_registry(dir.path(), &registry).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "v0.0-to-v0.1");
        let archive = dir.path().join("archive");
        assert!(archive.exists(), "archive dir should be created");
        let entries: Vec<_> = std::fs::read_dir(&archive).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }
}
