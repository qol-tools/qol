mod platform;

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const APP_NAME: &str = "qol-tray";
const INSTALL_ID_MARKER_FILE: &str = "qol-tray.install-id";
const ACTIVE_INSTALL_ID_FILE: &str = "active-install-id";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeStatus {
    Ok,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub id: &'static str,
    pub status: OutcomeStatus,
    pub message: String,
    pub fix_available: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
}

impl Report {
    pub fn count_ok(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o.status, OutcomeStatus::Ok))
            .count()
    }

    pub fn count_warn(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o.status, OutcomeStatus::Warn))
            .count()
    }

    pub fn count_error(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o.status, OutcomeStatus::Error))
            .count()
    }

    pub fn has_warnings(&self) -> bool {
        self.count_warn() > 0
    }

    pub fn has_errors(&self) -> bool {
        self.count_error() > 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct FixReport {
    pub before: Report,
    pub after: Report,
    pub attempted: usize,
    pub applied: usize,
    pub failures: Vec<String>,
}

struct Diagnosis {
    outcome: Outcome,
    fix: Option<FixAction>,
}

enum FixAction {
    SetActiveInstallId(String),
    WriteInstallMarker {
        marker_path: PathBuf,
        install_id: String,
    },
    WriteAutostartEntry {
        binary_path: PathBuf,
    },
    EnsurePluginsDir {
        path: PathBuf,
    },
}

pub fn check() -> Report {
    let diagnoses = collect_diagnoses();
    Report {
        outcomes: diagnoses.into_iter().map(|d| d.outcome).collect(),
    }
}

pub fn fix_safe() -> FixReport {
    let diagnoses = collect_diagnoses();
    let before = Report {
        outcomes: diagnoses.iter().map(|d| d.outcome.clone()).collect(),
    };

    let mut attempted = 0usize;
    let mut applied = 0usize;
    let mut failures = Vec::new();

    for diagnosis in diagnoses {
        let Some(action) = diagnosis.fix else {
            continue;
        };
        attempted += 1;
        if let Err(e) = apply_fix(&action) {
            failures.push(format!("{}: {}", diagnosis.outcome.id, e));
            continue;
        }
        applied += 1;
    }

    let after = check();
    FixReport {
        before,
        after,
        attempted,
        applied,
        failures,
    }
}

pub fn auto_fix_startup() -> FixReport {
    let report = fix_safe();

    if report.attempted > 0 {
        log::info!(
            "doctor startup fixes attempted={}, applied={}",
            report.attempted,
            report.applied
        );
    }

    for failure in &report.failures {
        log::warn!("doctor startup fix failed: {}", failure);
    }

    for outcome in &report.after.outcomes {
        if matches!(outcome.status, OutcomeStatus::Ok) {
            continue;
        }
        log::warn!("doctor {}: {}", outcome.id, outcome.message);
    }

    report
}

fn collect_diagnoses() -> Vec<Diagnosis> {
    vec![
        check_install_identity(),
        check_autostart_target(),
        check_plugins_dir(),
    ]
}

fn check_install_identity() -> Diagnosis {
    let id = "install_identity";
    let current_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            return error_outcome(id, format!("failed to resolve current executable: {}", e));
        }
    };

    let marker_path = match current_exe.parent() {
        Some(parent) => parent.join(INSTALL_ID_MARKER_FILE),
        None => {
            return error_outcome(
                id,
                format!(
                    "current executable has no parent directory: {}",
                    current_exe.display()
                ),
            );
        }
    };
    let marker_id = read_install_id_file(&marker_path);

    let active_path = match active_install_id_path() {
        Ok(path) => path,
        Err(e) => {
            return error_outcome(
                id,
                format!("failed to resolve active install id path: {}", e),
            )
        }
    };
    let active_id = read_install_id_file(&active_path);

    match (marker_id, active_id) {
        (Some(marker), Some(active)) if marker == active => ok_outcome(
            id,
            format!("marker and active install id are aligned ({})", marker),
        ),
        (Some(marker), Some(active)) => warn_outcome(
            id,
            format!(
                "marker install id ({}) differs from active install id ({})",
                marker, active
            ),
            Some(FixAction::SetActiveInstallId(marker)),
        ),
        (Some(marker), None) => warn_outcome(
            id,
            format!("active install id is missing; marker has {}", marker),
            Some(FixAction::SetActiveInstallId(marker)),
        ),
        (None, Some(active)) => warn_outcome(
            id,
            format!(
                "install marker missing near executable; active install id is {}",
                active
            ),
            Some(FixAction::WriteInstallMarker {
                marker_path,
                install_id: active,
            }),
        ),
        (None, None) => warn_outcome(
            id,
            "no install marker or active install id found".to_string(),
            None,
        ),
    }
}

fn check_autostart_target() -> Diagnosis {
    let id = "autostart_target";
    let current_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            return error_outcome(id, format!("failed to resolve current executable: {}", e));
        }
    };

    let autostart_path = match crate::installer::autostart_path() {
        Ok(path) => path,
        Err(e) => return error_outcome(id, format!("failed to resolve autostart path: {}", e)),
    };

    let target = match platform::read_autostart_target() {
        Ok(target) => target,
        Err(e) => {
            return error_outcome(
                id,
                format!(
                    "failed to read autostart target from {}: {}",
                    autostart_path.display(),
                    e
                ),
            )
        }
    };

    match target {
        None => warn_outcome(
            id,
            format!("autostart entry missing at {}", autostart_path.display()),
            Some(FixAction::WriteAutostartEntry {
                binary_path: current_exe,
            }),
        ),
        Some(target_path) => {
            let expected = canonical_or_original(&current_exe);
            let actual = canonical_or_original(&target_path);
            if expected == actual {
                return ok_outcome(
                    id,
                    format!(
                        "autostart target matches current binary ({})",
                        actual.display()
                    ),
                );
            }
            warn_outcome(
                id,
                format!(
                    "autostart target points to {} instead of {}",
                    target_path.display(),
                    current_exe.display()
                ),
                Some(FixAction::WriteAutostartEntry {
                    binary_path: current_exe,
                }),
            )
        }
    }
}

fn check_plugins_dir() -> Diagnosis {
    let id = "plugins_dir";
    let plugins_dir = match crate::paths::plugins_dir() {
        Ok(path) => path,
        Err(e) => return error_outcome(id, format!("failed to resolve plugins directory: {}", e)),
    };

    if plugins_dir.exists() && plugins_dir.is_dir() {
        return ok_outcome(
            id,
            format!("plugins directory exists ({})", plugins_dir.display()),
        );
    }

    if plugins_dir.exists() {
        return error_outcome(
            id,
            format!(
                "plugins path exists but is not a directory ({})",
                plugins_dir.display()
            ),
        );
    }

    warn_outcome(
        id,
        format!("plugins directory missing ({})", plugins_dir.display()),
        Some(FixAction::EnsurePluginsDir { path: plugins_dir }),
    )
}

fn apply_fix(action: &FixAction) -> Result<()> {
    match action {
        FixAction::SetActiveInstallId(install_id) => {
            crate::paths::set_active_install_id(install_id)
        }
        FixAction::WriteInstallMarker {
            marker_path,
            install_id,
        } => write_install_id_file(marker_path, install_id),
        FixAction::WriteAutostartEntry { binary_path } => {
            crate::installer::write_autostart_entry(binary_path)
        }
        FixAction::EnsurePluginsDir { path } => {
            fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
        }
    }
}

fn ok_outcome(id: &'static str, message: String) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Ok,
            message,
            fix_available: false,
        },
        fix: None,
    }
}

fn warn_outcome(id: &'static str, message: String, fix: Option<FixAction>) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Warn,
            message,
            fix_available: fix.is_some(),
        },
        fix,
    }
}

fn error_outcome(id: &'static str, message: String) -> Diagnosis {
    Diagnosis {
        outcome: Outcome {
            id,
            status: OutcomeStatus::Error,
            message,
            fix_available: false,
        },
        fix: None,
    }
}

fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn read_install_id_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if valid_install_id(trimmed) {
        return Some(trimmed.to_string());
    }
    None
}

fn write_install_id_file(path: &Path, install_id: &str) -> Result<()> {
    if !valid_install_id(install_id) {
        anyhow::bail!("invalid install id");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    fs::write(path, format!("{}\n", install_id))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn active_install_id_path() -> Result<PathBuf> {
    let base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .context("could not determine local data directory")?;
    Ok(base.join(APP_NAME).join(ACTIVE_INSTALL_ID_FILE))
}

fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
