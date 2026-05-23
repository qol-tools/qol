use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

pub mod cloud;
mod fs_util;
mod journal;
mod lock;
pub mod portability;
pub mod sentinel;
pub mod transforms;
mod v3_15_to_v3_16;

pub use fs_util::archive_path;

pub const OLDEST_SUPPORTED: &str = "3.15.0";
const VERSION_FILE: &str = "version.txt";

pub enum Phase {
    PreFlight,
    PostAuth,
}

pub struct MigrationContext<'a> {
    pub config_dir: &'a Path,
    pub github_token: Option<&'a str>,
    pub http: Option<&'a reqwest::Client>,
}

pub trait FileMigration: Send + Sync {
    fn name(&self) -> &'static str;
    fn applies(&self, config_dir: &Path) -> Result<bool>;
    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport>;
}

#[async_trait::async_trait]
pub trait CloudMigration: Send + Sync {
    fn name(&self) -> &'static str;
    async fn applies(&self, ctx: &MigrationContext<'_>) -> Result<bool>;
    async fn migrate(
        &self,
        ctx: &MigrationContext<'_>,
        archive_dir: &Path,
    ) -> Result<MigrationReport>;
}

#[derive(Debug, Clone)]
pub struct MigrationReport {
    pub name: String,
    pub archived: Vec<PathBuf>,
}

pub struct PreFlightRegistry {
    entries: Vec<Box<dyn FileMigration>>,
}

impl PreFlightRegistry {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn current() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(v3_15_to_v3_16::V3_15ToV3_16));
        registry
    }

    pub fn register(&mut self, migration: Box<dyn FileMigration>) {
        self.entries.push(migration);
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|m| m.name()).collect()
    }
}

impl Default for PreFlightRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PostAuthRegistry {
    entries: Vec<Box<dyn CloudMigration>>,
}

impl PostAuthRegistry {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn current() -> Self {
        Self::new()
    }

    pub fn register(&mut self, migration: Box<dyn CloudMigration>) {
        self.entries.push(migration);
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|m| m.name()).collect()
    }
}

impl Default for PostAuthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run_pre_flight(config_dir: &Path) -> Result<Vec<MigrationReport>> {
    run_pre_flight_with(config_dir, &PreFlightRegistry::current())
}

pub fn run_pre_flight_with(
    config_dir: &Path,
    registry: &PreFlightRegistry,
) -> Result<Vec<MigrationReport>> {
    let _lock = lock::MigrationLock::acquire(config_dir)
        .context("acquiring migration lock for pre-flight")?;

    reject_if_below_oldest_supported(config_dir)?;

    let mut reports = Vec::new();
    for migration in &registry.entries {
        let name = migration.name();
        if journal::is_done(config_dir, name) {
            log::info!("[qol-migrations] skipping {name}: already journaled");
            continue;
        }
        let applies = migration
            .applies(config_dir)
            .with_context(|| format!("checking applicability of {name}"))?;
        if !applies {
            continue;
        }
        let archive_dir = archive_path(config_dir, name)?;
        std::fs::create_dir_all(&archive_dir).context("creating archive dir")?;
        let report = migration
            .migrate(config_dir, &archive_dir)
            .with_context(|| format!("running migration {name}"))?;
        journal::write_done(config_dir, name)
            .with_context(|| format!("journaling completion of {name}"))?;
        log::info!(
            "[qol-migrations] applied {} (archived {} paths to {})",
            report.name,
            report.archived.len(),
            archive_dir.display()
        );
        reports.push(report);
    }

    write_current_version(config_dir, env!("CARGO_PKG_VERSION"))?;
    Ok(reports)
}

pub async fn run_post_auth(ctx: &MigrationContext<'_>) -> Result<Vec<MigrationReport>> {
    run_post_auth_with(ctx, &PostAuthRegistry::current()).await
}

pub async fn run_post_auth_with(
    ctx: &MigrationContext<'_>,
    registry: &PostAuthRegistry,
) -> Result<Vec<MigrationReport>> {
    let _lock = lock::MigrationLock::acquire(ctx.config_dir)
        .context("acquiring migration lock for post-auth")?;

    reject_if_below_oldest_supported(ctx.config_dir)?;

    let mut reports = Vec::new();
    for migration in &registry.entries {
        let name = migration.name();
        if journal::is_done(ctx.config_dir, name) {
            log::info!("[qol-migrations] skipping {name}: already journaled");
            continue;
        }
        let applies = migration
            .applies(ctx)
            .await
            .with_context(|| format!("checking applicability of {name}"))?;
        if !applies {
            continue;
        }
        let archive_dir = archive_path(ctx.config_dir, name)?;
        std::fs::create_dir_all(&archive_dir).context("creating archive dir")?;
        let report = migration
            .migrate(ctx, &archive_dir)
            .await
            .with_context(|| format!("running migration {name}"))?;
        journal::write_done(ctx.config_dir, name)
            .with_context(|| format!("journaling completion of {name}"))?;
        log::info!(
            "[qol-migrations] applied {} (archived {} paths to {})",
            report.name,
            report.archived.len(),
            archive_dir.display()
        );
        reports.push(report);
    }

    write_current_version(ctx.config_dir, env!("CARGO_PKG_VERSION"))?;
    Ok(reports)
}

fn version_path(config_dir: &Path) -> PathBuf {
    config_dir.join(VERSION_FILE)
}

fn read_installed_version(config_dir: &Path) -> Result<String> {
    let path = version_path(config_dir);
    if !path.exists() {
        return Ok(OLDEST_SUPPORTED.to_string());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(raw.trim().to_string())
}

fn write_current_version(config_dir: &Path, version: &str) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("ensuring config dir {}", config_dir.display()))?;
    let final_path = version_path(config_dir);
    let tmp_path = config_dir.join(format!("{VERSION_FILE}.tmp"));
    std::fs::write(&tmp_path, version)
        .with_context(|| format!("writing tmp {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "renaming {} -> {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    Ok(())
}

fn reject_if_below_oldest_supported(config_dir: &Path) -> Result<()> {
    let installed = read_installed_version(config_dir)?;
    if compare_semver(&installed, OLDEST_SUPPORTED) < 0 {
        return Err(anyhow!(
            "install version {installed} is older than the oldest supported version {OLDEST_SUPPORTED}; upgrade to {OLDEST_SUPPORTED} first"
        ));
    }
    Ok(())
}

fn parse_semver(v: &str) -> (u64, u64, u64) {
    let mut parts = v
        .split('.')
        .map(|p| p.split(|c: char| !c.is_ascii_digit()).next().unwrap_or(""))
        .map(|p| p.parse::<u64>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major, minor, patch)
}

fn compare_semver(a: &str, b: &str) -> i32 {
    let pa = parse_semver(a);
    let pb = parse_semver(b);
    match pa.cmp(&pb) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingFileMigration {
        name: &'static str,
        applies_response: bool,
        calls: Mutex<Vec<PathBuf>>,
    }

    impl FileMigration for RecordingFileMigration {
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

    struct RecordingCloudMigration {
        name: &'static str,
        applies_response: bool,
        order_log: std::sync::Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait::async_trait]
    impl CloudMigration for RecordingCloudMigration {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn applies(&self, _ctx: &MigrationContext<'_>) -> Result<bool> {
            Ok(self.applies_response)
        }

        async fn migrate(
            &self,
            _ctx: &MigrationContext<'_>,
            _archive_dir: &Path,
        ) -> Result<MigrationReport> {
            self.order_log.lock().unwrap().push(self.name);
            Ok(MigrationReport {
                name: self.name.to_string(),
                archived: vec![],
            })
        }
    }

    fn write_version(dir: &Path, v: &str) {
        std::fs::write(dir.join(VERSION_FILE), v).unwrap();
    }

    #[test]
    fn pre_flight_runs_registered_migrations_in_order_and_journals_each() {
        let dir = tempfile::tempdir().unwrap();
        let order_log = std::sync::Arc::new(Mutex::new(Vec::<&'static str>::new()));

        struct OrderedFm {
            name: &'static str,
            log: std::sync::Arc<Mutex<Vec<&'static str>>>,
        }
        impl FileMigration for OrderedFm {
            fn name(&self) -> &'static str {
                self.name
            }
            fn applies(&self, _: &Path) -> Result<bool> {
                Ok(true)
            }
            fn migrate(&self, _: &Path, _: &Path) -> Result<MigrationReport> {
                self.log.lock().unwrap().push(self.name);
                Ok(MigrationReport {
                    name: self.name.to_string(),
                    archived: vec![],
                })
            }
        }

        let mut registry = PreFlightRegistry::new();
        registry.register(Box::new(OrderedFm {
            name: "first",
            log: order_log.clone(),
        }));
        registry.register(Box::new(OrderedFm {
            name: "second",
            log: order_log.clone(),
        }));

        let reports = run_pre_flight_with(dir.path(), &registry).unwrap();

        assert_eq!(reports.len(), 2);
        assert_eq!(*order_log.lock().unwrap(), vec!["first", "second"]);
        assert!(journal::is_done(dir.path(), "first"));
        assert!(journal::is_done(dir.path(), "second"));
    }

    #[test]
    fn pre_flight_skips_already_journaled_migration() {
        let dir = tempfile::tempdir().unwrap();
        let migration = RecordingFileMigration {
            name: "already-done",
            applies_response: true,
            calls: Mutex::new(vec![]),
        };

        journal::write_done(dir.path(), "already-done").unwrap();

        let mut registry = PreFlightRegistry::new();
        registry.register(Box::new(migration));

        let reports = run_pre_flight_with(dir.path(), &registry).unwrap();

        assert!(reports.is_empty(), "journaled migration must be skipped");
    }

    #[test]
    fn pre_flight_skips_migration_that_does_not_apply() {
        let dir = tempfile::tempdir().unwrap();
        let migration = RecordingFileMigration {
            name: "skip-me",
            applies_response: false,
            calls: Mutex::new(vec![]),
        };
        let mut registry = PreFlightRegistry::new();
        registry.register(Box::new(migration));

        let reports = run_pre_flight_with(dir.path(), &registry).unwrap();

        assert!(reports.is_empty());
        assert!(!journal::is_done(dir.path(), "skip-me"));
    }

    #[test]
    fn pre_flight_rejects_install_older_than_oldest_supported() {
        let dir = tempfile::tempdir().unwrap();
        write_version(dir.path(), "3.14.9");

        let err = run_pre_flight_with(dir.path(), &PreFlightRegistry::new())
            .err()
            .expect("pre-flight should reject older install");

        let msg = format!("{err:#}");
        assert!(
            msg.contains("older than the oldest supported version"),
            "unexpected error: {msg}"
        );
        assert!(msg.contains("3.14.9"), "should mention installed: {msg}");
        assert!(msg.contains(OLDEST_SUPPORTED), "should mention oldest: {msg}");
    }

    #[test]
    fn pre_flight_writes_version_file_after_success() {
        let dir = tempfile::tempdir().unwrap();
        run_pre_flight_with(dir.path(), &PreFlightRegistry::new()).unwrap();
        let written = std::fs::read_to_string(dir.path().join(VERSION_FILE)).unwrap();
        assert_eq!(written, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn pre_flight_accepts_missing_version_file_as_oldest_supported() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!dir.path().join(VERSION_FILE).exists());
        run_pre_flight_with(dir.path(), &PreFlightRegistry::new()).unwrap();
    }

    #[tokio::test]
    async fn post_auth_runs_registered_cloud_migrations_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let order_log = std::sync::Arc::new(Mutex::new(Vec::<&'static str>::new()));

        let mut registry = PostAuthRegistry::new();
        registry.register(Box::new(RecordingCloudMigration {
            name: "cloud-a",
            applies_response: true,
            order_log: order_log.clone(),
        }));
        registry.register(Box::new(RecordingCloudMigration {
            name: "cloud-b",
            applies_response: true,
            order_log: order_log.clone(),
        }));

        let ctx = MigrationContext {
            config_dir: dir.path(),
            github_token: None,
            http: None,
        };

        let reports = run_post_auth_with(&ctx, &registry).await.unwrap();

        assert_eq!(reports.len(), 2);
        assert_eq!(*order_log.lock().unwrap(), vec!["cloud-a", "cloud-b"]);
        assert!(journal::is_done(dir.path(), "cloud-a"));
        assert!(journal::is_done(dir.path(), "cloud-b"));
    }

    #[test]
    fn semver_compare_orders_correctly() {
        let cases = [
            ("3.15.0", "3.15.0", 0),
            ("3.14.9", "3.15.0", -1),
            ("3.15.1", "3.15.0", 1),
            ("4.0.0", "3.99.99", 1),
            ("3.15", "3.15.0", 0),
        ];
        for (a, b, expected) in cases {
            assert_eq!(
                compare_semver(a, b),
                expected,
                "compare_semver({a}, {b})"
            );
        }
    }
}
