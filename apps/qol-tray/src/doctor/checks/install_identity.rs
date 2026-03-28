use super::super::diagnosis::{error_outcome, ok_outcome, warn_outcome, Diagnosis, FixAction};
use super::super::install_id::{active_install_id_path, marker_path_for, read_install_id_file};
use super::runtime_prereqs;
use anyhow::Result;
use std::path::PathBuf;

const ID: &str = "install_identity";

pub(super) fn check() -> Diagnosis {
    let context = match build_context() {
        Ok(context) => context,
        Err(error) => return error_outcome(ID, error.to_string()),
    };

    diagnose(context)
}

struct Context {
    marker_path: PathBuf,
    marker_id: Option<String>,
    active_id: Option<String>,
    marker_required: bool,
}

fn build_context() -> Result<Context> {
    let current_exe = runtime_prereqs::current_exe()?;
    let marker_path = marker_path_for(&current_exe)?;
    let active_path = active_install_id_path()?;

    Ok(Context {
        marker_id: read_install_id_file(&marker_path),
        active_id: read_install_id_file(&active_path),
        marker_path,
        marker_required: super::super::platform::install_marker_required(&current_exe),
    })
}

fn diagnose_both(marker: String, active: String) -> Diagnosis {
    if marker == active {
        return ok_outcome(
            ID,
            format!("marker and active install id are aligned ({})", marker),
        );
    }
    warn_outcome(
        ID,
        format!(
            "marker install id ({}) differs from active install id ({})",
            marker, active
        ),
        Some(FixAction::SetActiveInstallId(marker)),
    )
}

fn diagnose_marker_only(marker: String) -> Diagnosis {
    warn_outcome(
        ID,
        format!("active install id is missing; marker has {}", marker),
        Some(FixAction::SetActiveInstallId(marker)),
    )
}

fn diagnose_active_only(context: &Context, active: String) -> Diagnosis {
    if !context.marker_required {
        return ok_outcome(
            ID,
            format!("active install id is present ({})", active),
        );
    }

    warn_outcome(
        ID,
        format!(
            "install marker missing near executable; active install id is {}",
            active
        ),
        Some(FixAction::WriteInstallMarker {
            marker_path: context.marker_path.clone(),
            install_id: active,
        }),
    )
}

fn diagnose(context: Context) -> Diagnosis {
    match (&context.marker_id, &context.active_id) {
        (Some(marker), Some(active)) => diagnose_both(marker.clone(), active.clone()),
        (Some(marker), None) => diagnose_marker_only(marker.clone()),
        (None, Some(active)) => diagnose_active_only(&context, active.clone()),
        (None, None) => warn_outcome(
            ID,
            "no install marker or active install id found".to_string(),
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_install_id_is_ok_without_marker_for_app_bundle_installs() {
        let context = Context {
            marker_path: PathBuf::from("/tmp/qol-tray.install-id"),
            marker_id: None,
            active_id: Some("install-1".to_string()),
            marker_required: false,
        };

        let diagnosis = diagnose(context);
        assert_eq!(diagnosis.outcome.status, super::super::OutcomeStatus::Ok);
        assert!(!diagnosis.outcome.fix_available);
    }

    #[test]
    fn active_install_id_without_marker_still_warns_when_marker_is_required() {
        let context = Context {
            marker_path: PathBuf::from("/tmp/qol-tray.install-id"),
            marker_id: None,
            active_id: Some("install-1".to_string()),
            marker_required: true,
        };

        let diagnosis = diagnose(context);
        assert_eq!(diagnosis.outcome.status, super::super::OutcomeStatus::Warn);
        assert!(diagnosis.outcome.fix_available);
    }
}
