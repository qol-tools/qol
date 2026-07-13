use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use super::mimeapps;
use super::model::{
    ActionReport, ActionResult, ArtifactId, Operation, PlanItem, PlanTarget, ReportStatus,
    TargetState, UninstallContext, UninstallPlan, UninstallReport, REPORT_SCHEMA_VERSION,
};
use super::platform::PlatformOps;

pub(super) fn execute<P: PlatformOps>(
    platform: &P,
    context: &UninstallContext,
    plan: &UninstallPlan,
) -> UninstallReport {
    let mut results = HashMap::new();
    let mut actions = Vec::new();
    for item in &plan.items {
        let report = execute_item_with_dependencies(platform, context, item, &results);
        results.insert(item.id, report.result);
        actions.push(report);
    }
    let status = if actions.iter().any(|action| action.result.is_incomplete()) {
        ReportStatus::Partial
    } else {
        ReportStatus::Complete
    };
    UninstallReport {
        schema_version: REPORT_SCHEMA_VERSION,
        platform: plan.platform.to_string(),
        dry_run: false,
        purge_data: plan.purge_data,
        status,
        actions,
        preserved: plan.preserved.clone(),
        warnings: plan.warnings.clone(),
    }
}

fn execute_item_with_dependencies<P: PlatformOps>(
    platform: &P,
    context: &UninstallContext,
    item: &PlanItem,
    results: &HashMap<ArtifactId, ActionResult>,
) -> ActionReport {
    if dependency_incomplete(item, results) {
        return item.report(ActionResult::SkippedDependency, None);
    }
    match execute_item(platform, context, item) {
        Ok(result) => item.report(result, None),
        Err(error) => item.report(ActionResult::Failed, Some(format!("{error:#}"))),
    }
}

fn dependency_incomplete(item: &PlanItem, results: &HashMap<ArtifactId, ActionResult>) -> bool {
    item.depends_on.iter().any(|dependency| {
        results
            .get(dependency)
            .is_none_or(|result| result.is_incomplete())
    })
}

fn execute_item<P: PlatformOps>(
    platform: &P,
    context: &UninstallContext,
    item: &PlanItem,
) -> Result<ActionResult> {
    match item.state {
        TargetState::Absent => return Ok(ActionResult::AlreadyAbsent),
        TargetState::Unowned => return Ok(ActionResult::SkippedUnowned),
        TargetState::Present => {}
    }
    match item.operation {
        Operation::StopProcesses => {
            let PlanTarget::Processes(targets) = &item.target else {
                return Err(anyhow!("process action has a path target"));
            };
            platform.stop_processes(targets)?;
            Ok(ActionResult::Stopped)
        }
        Operation::RemoveFile => {
            remove_file(path_target(item)?)?;
            Ok(ActionResult::Removed)
        }
        Operation::RemoveDirectory => {
            remove_directory(path_target(item)?)?;
            Ok(ActionResult::Removed)
        }
        Operation::EditShellHook => {
            super::super::shell_hook::uninstall_managed_block(path_target(item)?)?;
            Ok(ActionResult::Updated)
        }
        Operation::EditMimeAssociation => {
            mimeapps::remove_qol_association(path_target(item)?)?;
            Ok(ActionResult::Updated)
        }
        Operation::RefreshDesktopCaches => {
            platform.refresh_desktop_caches(context)?;
            Ok(ActionResult::Updated)
        }
    }
}

fn path_target(item: &PlanItem) -> Result<&Path> {
    match &item.target {
        PlanTarget::Path(path) => Ok(path),
        PlanTarget::Processes(_) => Err(anyhow!("path action has a process target")),
    }
}

fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Failed to remove {}", path.display())),
    }
}

fn remove_directory(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Failed to remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{
        ArtifactSpec, Operation, Options, OwnershipProof, PreserveSpec, ProcessTargets,
        UninstallContext,
    };
    use super::super::planner;
    use super::*;
    use std::cell::Cell;
    use std::path::PathBuf;

    struct FakePlatform {
        context: UninstallContext,
        stopped: Cell<usize>,
        refreshed: Cell<usize>,
    }

    impl PlatformOps for FakePlatform {
        fn context(&self) -> Result<UninstallContext> {
            Ok(self.context.clone())
        }

        fn managed_processes(&self) -> Vec<crate::plugins::daemon_tracker::ManagedProcess> {
            Vec::new()
        }

        fn stop_processes(&self, _targets: &ProcessTargets) -> Result<()> {
            self.stopped.set(self.stopped.get() + 1);
            Ok(())
        }

        fn refresh_desktop_caches(&self, _context: &UninstallContext) -> Result<()> {
            self.refreshed.set(self.refreshed.get() + 1);
            Ok(())
        }
    }

    fn context(root: &Path) -> UninstallContext {
        let binary = root.join("bin/qol-tray");
        let marker = root.join("bin/qol-tray.install-id");
        let config = root.join("config");
        let data = root.join("data");
        UninstallContext {
            platform: "linux",
            artifacts: vec![
                ArtifactSpec {
                    id: ArtifactId::MimeDefault,
                    operation: Operation::EditMimeAssociation,
                    path: root.join("mimeapps.list"),
                    ownership: OwnershipProof::MimeAssociation,
                    depends_on: Vec::new(),
                },
                ArtifactSpec {
                    id: ArtifactId::RuntimeDirectory,
                    operation: Operation::RemoveDirectory,
                    path: root.join("runtime"),
                    ownership: OwnershipProof::AnyDirectory,
                    depends_on: Vec::new(),
                },
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
            purge_artifacts: Vec::new(),
            preserved: vec![
                PreserveSpec {
                    id: ArtifactId::ConfigDirectory,
                    path: config,
                    reason: "user config",
                },
                PreserveSpec {
                    id: ArtifactId::DataDirectory,
                    path: data,
                    reason: "user data",
                },
            ],
            refresh_root: PathBuf::from("/data"),
        }
    }

    fn seed(root: &Path) {
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("runtime")).unwrap();
        fs::create_dir_all(root.join("config/profile")).unwrap();
        fs::create_dir_all(root.join("data/plugins")).unwrap();
        fs::write(root.join("bin/qol-tray"), "binary").unwrap();
        fs::write(root.join("bin/qol-tray.install-id"), "install-123\n").unwrap();
        fs::write(
            root.join("mimeapps.list"),
            "[Default Applications]\nx-scheme-handler/qol=qol-tray.desktop;other.desktop;\n",
        )
        .unwrap();
    }

    #[test]
    fn execution_removes_owned_artifacts_preserves_data_and_retries_cleanly() {
        let tmp = tempfile::TempDir::new().unwrap();
        seed(tmp.path());
        let context = context(tmp.path());
        let platform = FakePlatform {
            context: context.clone(),
            stopped: Cell::new(0),
            refreshed: Cell::new(0),
        };

        let first_plan = planner::build(context.clone(), Vec::new(), Options::default());
        let first = execute(&platform, &context, &first_plan);
        let second_plan = planner::build(context.clone(), Vec::new(), Options::default());
        let second = execute(&platform, &context, &second_plan);

        assert_eq!(first.status, ReportStatus::Complete);
        assert_eq!(second.status, ReportStatus::Complete);
        assert!(!tmp.path().join("bin/qol-tray").exists());
        assert!(!tmp.path().join("bin/qol-tray.install-id").exists());
        assert!(!tmp.path().join("runtime").exists());
        assert!(tmp.path().join("config/profile").is_dir());
        assert!(tmp.path().join("data/plugins").is_dir());
        let mime = fs::read_to_string(tmp.path().join("mimeapps.list")).unwrap();
        assert_eq!(
            mime,
            "[Default Applications]\nx-scheme-handler/qol=other.desktop;\n"
        );
        assert_eq!(platform.stopped.get(), 1);
    }

    #[test]
    fn marker_is_preserved_when_binary_ownership_is_unproven() {
        let tmp = tempfile::TempDir::new().unwrap();
        seed(tmp.path());
        fs::write(tmp.path().join("bin/qol-tray.install-id"), "invalid id").unwrap();
        let context = context(tmp.path());
        let platform = FakePlatform {
            context: context.clone(),
            stopped: Cell::new(0),
            refreshed: Cell::new(0),
        };

        let plan = planner::build(context.clone(), Vec::new(), Options::default());
        let report = execute(&platform, &context, &plan);

        assert_eq!(report.status, ReportStatus::Partial);
        assert!(tmp.path().join("bin/qol-tray").is_file());
        assert!(tmp.path().join("bin/qol-tray.install-id").is_file());
        let marker = report
            .actions
            .iter()
            .find(|action| action.id == ArtifactId::InstallMarker)
            .unwrap();
        assert_eq!(marker.result, ActionResult::SkippedDependency);
    }
}
