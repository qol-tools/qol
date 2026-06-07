use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use super::super::install_id::{active_install_id_path, marker_path_for, read_install_id_file};
use super::runtime_prereqs;
use anyhow::Result;
use std::path::PathBuf;

const ID: &str = "install_identity";

pub(super) struct InstallIdentityCheck;

impl DoctorCheck for InstallIdentityCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Install identity", CheckCategory::Install)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let context = match build_context() {
            Ok(context) => context,
            Err(error) => return CheckReport::error(error.to_string(), ID),
        };
        diagnose(context)
    }
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

fn diagnose_both(marker: String, active: String) -> CheckReport {
    if marker == active {
        return CheckReport::ok(format!(
            "marker and active install id are aligned ({})",
            marker
        ));
    }
    CheckReport::warn(
        format!(
            "marker install id ({}) differs from active install id ({})",
            marker, active
        ),
        ID,
        vec![FixAction::SetActiveInstallId(marker)],
    )
}

fn diagnose_marker_only(marker: String) -> CheckReport {
    CheckReport::warn(
        format!("active install id is missing; marker has {}", marker),
        ID,
        vec![FixAction::SetActiveInstallId(marker)],
    )
}

fn diagnose_active_only(context: &Context, active: String) -> CheckReport {
    if !context.marker_required {
        return CheckReport::ok(format!("active install id is present ({})", active));
    }

    CheckReport::warn(
        format!(
            "install marker missing near executable; active install id is {}",
            active
        ),
        ID,
        vec![FixAction::WriteInstallMarker {
            marker_path: context.marker_path.clone(),
            install_id: active,
        }],
    )
}

fn diagnose(context: Context) -> CheckReport {
    match (&context.marker_id, &context.active_id) {
        (Some(marker), Some(active)) => diagnose_both(marker.clone(), active.clone()),
        (Some(marker), None) => diagnose_marker_only(marker.clone()),
        (None, Some(active)) => diagnose_active_only(&context, active.clone()),
        (None, None) => CheckReport::warn(
            "no install marker or active install id found".to_string(),
            ID,
            Vec::new(),
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

        let report = diagnose(context);
        assert!(report.issues.is_empty(), "Ok report must have no issues");
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn active_install_id_without_marker_still_warns_when_marker_is_required() {
        let context = Context {
            marker_path: PathBuf::from("/tmp/qol-tray.install-id"),
            marker_id: None,
            active_id: Some("install-1".to_string()),
            marker_required: true,
        };

        let report = diagnose(context);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.fixes.len(), 1);
    }
}
