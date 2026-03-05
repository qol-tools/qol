use super::diagnosis::{error_outcome, ok_outcome, warn_outcome, Diagnosis, FixAction};
use super::install_id::{
    active_install_id_path, canonical_or_original, marker_path_for, read_install_id_file,
};
use super::platform;
use anyhow::{Context, Result};
use std::path::PathBuf;

pub(super) fn collect_diagnoses() -> Vec<Diagnosis> {
    vec![
        check_install_identity(),
        check_autostart_target(),
        check_plugins_dir(),
    ]
}

struct InstallIdentityContext {
    marker_path: PathBuf,
    marker_id: Option<String>,
    active_id: Option<String>,
}

struct AutostartTargetContext {
    current_exe: PathBuf,
    autostart_path: PathBuf,
    target: Option<PathBuf>,
}

fn check_install_identity() -> Diagnosis {
    let id = "install_identity";
    let context = match install_identity_context() {
        Ok(context) => context,
        Err(error) => return error_outcome(id, error.to_string()),
    };
    install_identity_diagnosis(id, context)
}

fn check_autostart_target() -> Diagnosis {
    let id = "autostart_target";
    let context = match autostart_target_context() {
        Ok(context) => context,
        Err(error) => return error_outcome(id, error.to_string()),
    };
    autostart_target_diagnosis(id, context)
}

fn check_plugins_dir() -> Diagnosis {
    let id = "plugins_dir";
    let plugins_dir = match crate::paths::plugins_dir() {
        Ok(path) => path,
        Err(error) => return error_outcome(id, plugins_dir_error(error)),
    };
    plugins_dir_diagnosis(id, plugins_dir)
}

fn install_identity_context() -> Result<InstallIdentityContext> {
    let current_exe = current_exe()?;
    let marker_path = marker_path_for(&current_exe)?;
    let active_path = active_install_id_path()?;
    Ok(InstallIdentityContext {
        marker_id: read_install_id_file(&marker_path),
        active_id: read_install_id_file(&active_path),
        marker_path,
    })
}

fn autostart_target_context() -> Result<AutostartTargetContext> {
    let current_exe = current_exe()?;
    let autostart_path = crate::installer::autostart_path()?;
    let target = platform::read_autostart_target().with_context(|| {
        format!(
            "failed to read autostart target from {}",
            autostart_path.display()
        )
    })?;
    Ok(AutostartTargetContext {
        current_exe,
        autostart_path,
        target,
    })
}

fn install_identity_diagnosis(id: &'static str, context: InstallIdentityContext) -> Diagnosis {
    match (context.marker_id, context.active_id) {
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
                marker_path: context.marker_path,
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

fn autostart_target_diagnosis(id: &'static str, context: AutostartTargetContext) -> Diagnosis {
    let Some(target_path) = context.target else {
        return warn_outcome(
            id,
            format!(
                "autostart entry missing at {}",
                context.autostart_path.display()
            ),
            Some(FixAction::WriteAutostartEntry {
                binary_path: context.current_exe,
            }),
        );
    };

    let expected = canonical_or_original(&context.current_exe);
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
            context.current_exe.display()
        ),
        Some(FixAction::WriteAutostartEntry {
            binary_path: context.current_exe,
        }),
    )
}

fn plugins_dir_diagnosis(id: &'static str, plugins_dir: PathBuf) -> Diagnosis {
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

fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("failed to resolve current executable")
}

fn plugins_dir_error(error: anyhow::Error) -> String {
    format!("failed to resolve plugins directory: {}", error)
}
