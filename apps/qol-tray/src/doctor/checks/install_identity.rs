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
}

fn build_context() -> Result<Context> {
    let current_exe = runtime_prereqs::current_exe()?;
    let marker_path = marker_path_for(&current_exe)?;
    let active_path = active_install_id_path()?;

    Ok(Context {
        marker_id: read_install_id_file(&marker_path),
        active_id: read_install_id_file(&active_path),
        marker_path,
    })
}

fn diagnose(context: Context) -> Diagnosis {
    match (context.marker_id, context.active_id) {
        (Some(marker), Some(active)) if marker == active => ok_outcome(
            ID,
            format!("marker and active install id are aligned ({})", marker),
        ),
        (Some(marker), Some(active)) => warn_outcome(
            ID,
            format!(
                "marker install id ({}) differs from active install id ({})",
                marker, active
            ),
            Some(FixAction::SetActiveInstallId(marker)),
        ),
        (Some(marker), None) => warn_outcome(
            ID,
            format!("active install id is missing; marker has {}", marker),
            Some(FixAction::SetActiveInstallId(marker)),
        ),
        (None, Some(active)) => warn_outcome(
            ID,
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
            ID,
            "no install marker or active install id found".to_string(),
            None,
        ),
    }
}
