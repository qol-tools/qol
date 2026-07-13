use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use super::mimeapps;
use super::model::{
    ActionResult, ArtifactId, ArtifactSpec, Operation, Options, OwnershipProof, PlanItem,
    PlanTarget, PreservedReport, ProcessTargets, ReportStatus, TargetState, UninstallContext,
    UninstallPlan, UninstallReport, REPORT_SCHEMA_VERSION,
};

pub(super) fn build(
    context: UninstallContext,
    plugins: Vec<crate::plugins::daemon_tracker::ManagedProcess>,
    options: Options,
) -> UninstallPlan {
    let mut warnings = Vec::new();
    let mut preserved = inspect_preserved(&context, options, &mut warnings);
    let mut artifact_items = inspect_artifacts(&context.artifacts, &mut warnings);
    preserve_shell_hooks(&mut artifact_items, &mut preserved, options, &context);
    let processes = process_item(&artifact_items, plugins);
    let refresh = refresh_item(&artifact_items, &context);
    let mut items = vec![processes];
    items.extend(artifact_items);
    items.push(refresh);
    if options.purge_data {
        items.extend(inspect_artifacts(&context.purge_artifacts, &mut warnings));
    }
    UninstallPlan {
        platform: context.platform,
        purge_data: options.purge_data,
        items,
        preserved,
        warnings,
    }
}

pub(super) fn planned_report(plan: &UninstallPlan) -> UninstallReport {
    let actions = plan
        .items
        .iter()
        .map(|item| item.report(planned_result(item.state), None))
        .collect();
    UninstallReport {
        schema_version: REPORT_SCHEMA_VERSION,
        platform: plan.platform.to_string(),
        dry_run: true,
        purge_data: plan.purge_data,
        status: ReportStatus::Planned,
        actions,
        preserved: plan.preserved.clone(),
        warnings: plan.warnings.clone(),
    }
}

fn inspect_artifacts(specs: &[ArtifactSpec], warnings: &mut Vec<String>) -> Vec<PlanItem> {
    specs
        .iter()
        .map(|spec| {
            let (state, warning) = inspect_artifact(spec);
            warnings.extend(warning);
            PlanItem {
                id: spec.id,
                operation: spec.operation,
                target: PlanTarget::Path(spec.path.clone()),
                state,
                depends_on: spec.depends_on.clone(),
            }
        })
        .collect()
}

fn inspect_preserved(
    context: &UninstallContext,
    options: Options,
    warnings: &mut Vec<String>,
) -> Vec<PreservedReport> {
    if options.purge_data {
        return Vec::new();
    }
    context
        .preserved
        .iter()
        .map(|spec| {
            let (state, warning) = inspect_preserved_path(&spec.path);
            warnings.extend(warning);
            PreservedReport {
                id: spec.id,
                path: spec.path.clone(),
                state,
                reason: spec.reason.to_string(),
            }
        })
        .collect()
}

fn preserve_shell_hooks(
    items: &mut Vec<PlanItem>,
    preserved: &mut Vec<PreservedReport>,
    options: Options,
    context: &UninstallContext,
) {
    if !options.skip_shell_hook {
        return;
    }
    let shell_ids = [ArtifactId::ShellHookBash, ArtifactId::ShellHookZsh];
    for artifact in &context.artifacts {
        if !shell_ids.contains(&artifact.id) {
            continue;
        }
        let Some(item) = items.iter().find(|item| item.id == artifact.id) else {
            continue;
        };
        preserved.push(PreservedReport {
            id: artifact.id,
            path: artifact.path.clone(),
            state: item.state,
            reason: "preserved by --skip-shell-hook".to_string(),
        });
    }
    items.retain(|item| !shell_ids.contains(&item.id));
}

fn process_item(
    items: &[PlanItem],
    plugins: Vec<crate::plugins::daemon_tracker::ManagedProcess>,
) -> PlanItem {
    let installed_binary = items
        .iter()
        .find(|item| item.id == ArtifactId::Binary && item.state == TargetState::Present)
        .and_then(|item| match &item.target {
            PlanTarget::Path(path) => Some(path.clone()),
            PlanTarget::Processes(_) => None,
        });
    let state = if installed_binary.is_some() || !plugins.is_empty() {
        TargetState::Present
    } else {
        TargetState::Absent
    };
    PlanItem {
        id: ArtifactId::StopProcesses,
        operation: Operation::StopProcesses,
        target: PlanTarget::Processes(ProcessTargets {
            installed_binary,
            plugins,
        }),
        state,
        depends_on: Vec::new(),
    }
}

fn refresh_item(items: &[PlanItem], context: &UninstallContext) -> PlanItem {
    let state = if items.iter().any(needs_desktop_refresh) {
        TargetState::Present
    } else {
        TargetState::Absent
    };
    PlanItem {
        id: ArtifactId::RefreshDesktopCaches,
        operation: Operation::RefreshDesktopCaches,
        target: PlanTarget::Path(context.refresh_root.clone()),
        state,
        depends_on: Vec::new(),
    }
}

fn needs_desktop_refresh(item: &PlanItem) -> bool {
    item.state == TargetState::Present
        && matches!(
            item.id,
            ArtifactId::DesktopEntry
                | ArtifactId::Icon64
                | ArtifactId::Icon128
                | ArtifactId::Icon256
        )
}

fn planned_result(state: TargetState) -> ActionResult {
    match state {
        TargetState::Present => ActionResult::Planned,
        TargetState::Absent => ActionResult::AlreadyAbsent,
        TargetState::Unowned => ActionResult::SkippedUnowned,
    }
}

fn inspect_artifact(spec: &ArtifactSpec) -> (TargetState, Option<String>) {
    let kind = match path_kind(&spec.path) {
        Ok(kind) => kind,
        Err(error) => return unowned(spec, &error.to_string()),
    };
    if kind == PathKind::Missing {
        return (TargetState::Absent, None);
    }
    let state = match &spec.ownership {
        OwnershipProof::AnyFile => file_like_state(kind),
        OwnershipProof::AnyDirectory => directory_state(kind),
        OwnershipProof::BinaryWithMarker(marker) => binary_state(kind, marker),
        OwnershipProof::ExactBytes(expected) => exact_bytes_state(kind, &spec.path, expected),
        OwnershipProof::MimeAssociation => mime_state(kind, &spec.path),
        OwnershipProof::ShellHook => shell_hook_state(kind, &spec.path),
        OwnershipProof::TextMarkers(markers) => text_markers_state(kind, &spec.path, markers),
        OwnershipProof::ValidInstallId => valid_install_id_state(kind, &spec.path),
    };
    match state {
        Ok(state) => (state, None),
        Err(error) => unowned(spec, &error),
    }
}

fn inspect_preserved_path(path: &Path) -> (TargetState, Option<String>) {
    match path_kind(path) {
        Ok(PathKind::Missing) => (TargetState::Absent, None),
        Ok(_) => (TargetState::Present, None),
        Err(error) => (
            TargetState::Unowned,
            Some(format!(
                "Could not inspect preserved path {}: {error}",
                path.display()
            )),
        ),
    }
}

fn unowned(spec: &ArtifactSpec, detail: &str) -> (TargetState, Option<String>) {
    (
        TargetState::Unowned,
        Some(format!(
            "{} was left untouched because ownership could not be proven at {}: {detail}",
            spec.id.label(),
            spec.path.display()
        )),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathKind {
    Missing,
    File,
    Directory,
    Symlink,
    Other,
}

fn path_kind(path: &Path) -> std::io::Result<PathKind> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(PathKind::Missing),
        Err(error) => return Err(error),
    };
    let kind = metadata.file_type();
    if kind.is_file() {
        return Ok(PathKind::File);
    }
    if kind.is_dir() {
        return Ok(PathKind::Directory);
    }
    if kind.is_symlink() {
        return Ok(PathKind::Symlink);
    }
    Ok(PathKind::Other)
}

fn file_like_state(kind: PathKind) -> Result<TargetState, String> {
    match kind {
        PathKind::File => Ok(TargetState::Present),
        _ => Err(format!("expected a regular file, found {kind:?}")),
    }
}

fn directory_state(kind: PathKind) -> Result<TargetState, String> {
    match kind {
        PathKind::Directory => Ok(TargetState::Present),
        _ => Err(format!("expected a directory, found {kind:?}")),
    }
}

fn binary_state(kind: PathKind, marker: &Path) -> Result<TargetState, String> {
    file_like_state(kind)?;
    if marker_has_valid_install_id(marker)? {
        return Ok(TargetState::Present);
    }
    Err(format!(
        "valid ownership marker missing at {}",
        marker.display()
    ))
}

fn exact_bytes_state(kind: PathKind, path: &Path, expected: &[u8]) -> Result<TargetState, String> {
    file_like_state(kind)?;
    let actual = fs::read(path).map_err(|error| error.to_string())?;
    if actual == expected {
        return Ok(TargetState::Present);
    }
    Err("file content differs from the installer-owned artifact".to_string())
}

fn mime_state(kind: PathKind, path: &Path) -> Result<TargetState, String> {
    file_like_state(kind)?;
    let present = mimeapps::contains_qol_association(path).map_err(|error| error.to_string())?;
    Ok(if present {
        TargetState::Present
    } else {
        TargetState::Absent
    })
}

fn shell_hook_state(kind: PathKind, path: &Path) -> Result<TargetState, String> {
    file_like_state(kind)?;
    let present = super::super::shell_hook::contains_managed_block(path)
        .map_err(|error| error.to_string())?;
    Ok(if present {
        TargetState::Present
    } else {
        TargetState::Absent
    })
}

fn text_markers_state(
    kind: PathKind,
    path: &Path,
    markers: &[&str],
) -> Result<TargetState, String> {
    file_like_state(kind)?;
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if markers
        .iter()
        .all(|marker| content.lines().any(|line| line == *marker))
    {
        return Ok(TargetState::Present);
    }
    Err("file does not match the installer-owned signature".to_string())
}

fn valid_install_id_state(kind: PathKind, path: &Path) -> Result<TargetState, String> {
    file_like_state(kind)?;
    if marker_has_valid_install_id(path)? {
        return Ok(TargetState::Present);
    }
    Err("marker does not contain a valid install id".to_string())
}

fn marker_has_valid_install_id(path: &Path) -> Result<bool, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    Ok(qol_config::valid_install_id(raw.trim()))
}

#[cfg(test)]
mod tests {
    use super::super::model::{ArtifactSpec, OwnershipProof, PreserveSpec};
    use super::*;
    use std::path::PathBuf;

    fn context(root: &Path) -> UninstallContext {
        let binary = root.join("bin/qol-tray");
        let marker = root.join("bin/qol-tray.install-id");
        UninstallContext {
            platform: "linux",
            artifacts: vec![
                ArtifactSpec {
                    id: ArtifactId::Binary,
                    operation: Operation::RemoveFile,
                    path: binary,
                    ownership: OwnershipProof::BinaryWithMarker(marker.clone()),
                    depends_on: Vec::new(),
                },
                ArtifactSpec {
                    id: ArtifactId::InstallMarker,
                    operation: Operation::RemoveFile,
                    path: marker,
                    ownership: OwnershipProof::ValidInstallId,
                    depends_on: vec![ArtifactId::Binary],
                },
            ],
            purge_artifacts: vec![
                ArtifactSpec {
                    id: ArtifactId::ConfigDirectory,
                    operation: Operation::RemoveDirectory,
                    path: root.join("config"),
                    ownership: OwnershipProof::AnyDirectory,
                    depends_on: Vec::new(),
                },
                ArtifactSpec {
                    id: ArtifactId::DataDirectory,
                    operation: Operation::RemoveDirectory,
                    path: root.join("data"),
                    ownership: OwnershipProof::AnyDirectory,
                    depends_on: Vec::new(),
                },
            ],
            preserved: vec![
                PreserveSpec {
                    id: ArtifactId::ConfigDirectory,
                    path: root.join("config"),
                    reason: "user config",
                },
                PreserveSpec {
                    id: ArtifactId::DataDirectory,
                    path: root.join("data"),
                    reason: "user data",
                },
            ],
            refresh_root: PathBuf::from("/data"),
        }
    }

    #[test]
    fn owned_binary_is_planned_and_marker_depends_on_its_removal() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("bin")).unwrap();
        fs::write(tmp.path().join("bin/qol-tray"), "binary").unwrap();
        fs::write(tmp.path().join("bin/qol-tray.install-id"), "install-123\n").unwrap();

        let plan = build(context(tmp.path()), Vec::new(), Options::default());
        let binary = plan
            .items
            .iter()
            .find(|item| item.id == ArtifactId::Binary)
            .unwrap();
        let marker = plan
            .items
            .iter()
            .find(|item| item.id == ArtifactId::InstallMarker)
            .unwrap();

        assert_eq!(binary.state, TargetState::Present);
        assert_eq!(marker.state, TargetState::Present);
        assert_eq!(marker.depends_on, vec![ArtifactId::Binary]);
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn binary_without_marker_is_reported_unowned_and_left_untouched() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("bin")).unwrap();
        fs::write(tmp.path().join("bin/qol-tray"), "binary").unwrap();

        let plan = build(context(tmp.path()), Vec::new(), Options::default());
        let report = planned_report(&plan);
        let binary = report
            .actions
            .iter()
            .find(|action| action.id == ArtifactId::Binary)
            .unwrap();

        assert_eq!(binary.state, TargetState::Unowned);
        assert_eq!(binary.result, ActionResult::SkippedUnowned);
        assert_eq!(plan.warnings.len(), 1);
    }

    #[test]
    fn default_preserves_user_roots_while_purge_plans_their_removal() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("config/profile")).unwrap();
        fs::create_dir_all(tmp.path().join("data/plugins")).unwrap();

        let preserved = build(context(tmp.path()), Vec::new(), Options::default());
        let purged = build(
            context(tmp.path()),
            Vec::new(),
            Options {
                purge_data: true,
                ..Options::default()
            },
        );

        assert_eq!(preserved.preserved.len(), 2);
        assert!(preserved.items.iter().all(|item| !matches!(
            item.id,
            ArtifactId::ConfigDirectory | ArtifactId::DataDirectory
        )));
        assert!(purged.preserved.is_empty());
        assert!(purged
            .items
            .iter()
            .any(|item| item.id == ArtifactId::ConfigDirectory));
        assert!(purged
            .items
            .iter()
            .any(|item| item.id == ArtifactId::DataDirectory));
    }
}
