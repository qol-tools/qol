use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use qol_dev_env::ReportKind;
use serde_json::{json, Value};

use super::probes::Verification;
use super::storage::StagedImage;
use super::ImageImportPlan;
use crate::commands::emu::machine::LifecycleCleanupProof;

pub(super) fn write_initial_report(plan: &ImageImportPlan) -> Result<()> {
    let report = json!({
        "name": "qol-env-image-import",
        "kind": "image-import",
        "run_id": plan.run_id,
        "started_at_unix_ms": plan.started_at_unix_ms,
        "status": "preparing",
        "owner": owner(plan, "preparing"),
        "environment": {
            "id": plan.environment.id,
            "name": plan.environment.name,
            "backend": plan.environment.backend,
        },
        "launch": {
            "display": "none",
            "network": "none",
            "memory_mb": plan.environment.boot.memory_mb,
            "cpus": plan.environment.boot.cpus,
            "guest_adapter": plan.guest_adapter.as_str(),
            "guest_image_revision": plan.guest_revision,
        },
        "workflow": workflow_json(
            plan,
            &Verification {
                verdict: "pending",
                probes: Vec::new(),
                error: None,
            },
            None,
            "pending",
            None,
        ),
        "artifacts": {
            "run_dir": plan.report_path.parent(),
            "report": plan.report_path,
            "conversion": plan.report_path.parent().map(|run_dir| run_dir.join("conversion.json")),
            "image_import_config": plan.config_path,
        },
        "teardown": null,
        "next": [format!("Cancel with `qol env cancel {}`.", plan.run_id)],
    });
    write_report(&plan.report_path, &report)
}

pub(super) fn terminalize_report(
    plan: &ImageImportPlan,
    status: &str,
    workflow: Value,
    error: Option<&str>,
    staging_removed: bool,
    readonly: bool,
    cleanup: &LifecycleCleanupProof,
) -> Result<()> {
    let checked = qol_dev_env::read_report_checked(
        &plan.report_path,
        &plan.run_id,
        &ReportKind::ImageImport,
    )?
    .context("image-import report disappeared before terminal commit")?;
    if checked.status.is_terminal() {
        bail!("image-import report is already terminal");
    }
    let mut report = checked.document().clone();
    let existing_qemu_exit = report["teardown"]["qemu_exit_verified"].as_bool();
    let existing_tree_exit = report["teardown"]["tree_exit_verified"].as_bool();
    let qemu_exit_verified = existing_qemu_exit
        .map(|verified| verified && cleanup.qemu_exit_verified)
        .unwrap_or(cleanup.qemu_exit_verified);
    let tree_exit_verified = existing_tree_exit
        .map(|verified| verified && cleanup.tree_exit_verified)
        .unwrap_or(cleanup.tree_exit_verified);
    let cleanup_complete =
        staging_removed && cleanup.artifacts_removed && qemu_exit_verified && tree_exit_verified;
    let status = if cleanup_complete {
        status
    } else {
        "cleanup-incomplete"
    };
    report["status"] = json!(status);
    report["finished_at_unix_ms"] = json!(qol_dev_env::unix_millis()?);
    report["owner"] = owner(plan, status);
    report["workflow"] = workflow;
    report["teardown"]["status"] = json!(if cleanup_complete {
        "complete"
    } else {
        "incomplete"
    });
    let qemu_started = report["teardown"]["qemu_started"]
        .as_bool()
        .unwrap_or(false)
        || cleanup.qemu_started;
    report["teardown"]["qemu_started"] = json!(qemu_started);
    report["teardown"]["qemu_exit_verified"] = json!(qemu_exit_verified);
    report["teardown"]["tree_exit_verified"] = json!(tree_exit_verified);
    report["teardown"]["staging_removed"] = json!(staging_removed);
    if let Some(cleanup_error) = &cleanup.error {
        report["teardown"]["error"] = json!(cleanup_error);
    }
    report["next"] = if status == "pass" {
        json!([format!(
            "Run a sandbox with `qol env up {}`.",
            plan.environment.id
        )])
    } else {
        json!([format!("Inspect {}.", plan.report_path.display())])
    };
    if let Some(error) = error {
        report["error"] = json!(error);
    }
    write_report(&plan.report_path, &report)?;
    if readonly && cleanup_complete {
        set_readonly(&plan.report_path)?;
    }
    Ok(())
}

pub(super) fn workflow_json(
    plan: &ImageImportPlan,
    verification: &Verification,
    staged: Option<&StagedImage>,
    promotion_status: &str,
    promotion: Option<&Value>,
) -> Value {
    let image_path = staged.map(|staged| staged.image_path.as_path());
    let staging_path = staged.map(|staged| staged.path.clone()).or_else(|| {
        plan.report_path
            .parent()
            .map(|run_dir| run_dir.join("source.qcow2"))
    });
    let promotion = promotion.cloned().unwrap_or_else(|| {
        json!({
            "status": promotion_status,
            "image_path": image_path,
        })
    });
    let mut workflow = json!({
        "id": "image-import-verification",
        "verdict": verification.verdict,
        "adapter": plan.guest_adapter.as_str(),
        "source": {
            "path": plan.source,
            "sha256": staged.map(|staged| staged.sha256.as_str()),
            "size_bytes": staged
                .map(|staged| staged.size_bytes)
                .unwrap_or(plan.source_stamp.size_bytes),
            "format": "qcow2",
            "virtual_size": staged
                .map(|staged| staged.virtual_size)
                .unwrap_or(plan.source_virtual_size),
        },
        "staging": {
            "path": staging_path,
        },
        "probes": verification.probes,
        "promotion": promotion,
    });
    if let Some(error) = &verification.error {
        workflow["error"] = json!(error);
    }
    workflow
}

pub(super) fn refuse_existing_run(run_dir: &Path, report_path: &Path) -> Result<()> {
    if fs::symlink_metadata(run_dir).is_ok() || fs::symlink_metadata(report_path).is_ok() {
        bail!(
            "image-import run already exists at {}; choose another --run-id",
            run_dir.display()
        );
    }
    Ok(())
}

fn owner(plan: &ImageImportPlan, state: &str) -> Value {
    json!({
        "pid": std::process::id(),
        "process_identity": qol_process::process_identity(std::process::id()).ok(),
        "state": state,
        "worktree": plan.worktree,
        "task": "image-import-verification",
    })
}

fn write_report(path: &Path, report: &Value) -> Result<()> {
    let content =
        serde_json::to_vec_pretty(report).context("failed to encode image-import report")?;
    let mut terminated = content;
    terminated.push(b'\n');
    qol_fs::atomic_write_durable(path, &terminated)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn set_readonly(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to make {} read-only", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::commands::emu::image_import::tests::plan_fixture;

    #[test]
    fn initial_and_terminal_reports_keep_one_exact_typed_identity() {
        let root = tempfile::tempdir().unwrap();
        let report_path = root.path().join("imports/image-import-test/report.json");
        fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        let mut plan = plan_fixture(root.path());
        plan.report_path = report_path.clone();
        plan.worktree = root.path().to_path_buf();
        write_initial_report(&plan).unwrap();
        let preparing =
            qol_dev_env::read_report_checked(&report_path, &plan.run_id, &ReportKind::ImageImport)
                .unwrap()
                .unwrap();
        assert_eq!(preparing.status, qol_dev_env::ReportStatus::Preparing);
        terminalize_report(
            &plan,
            "failed",
            workflow_json(
                &plan,
                &Verification::failed(Vec::new(), "fixture"),
                None,
                "not-published",
                None,
            ),
            Some("fixture"),
            true,
            false,
            &LifecycleCleanupProof::not_started(true),
        )
        .unwrap();
        let terminal =
            qol_dev_env::read_report_checked(&report_path, &plan.run_id, &ReportKind::ImageImport)
                .unwrap()
                .unwrap();
        assert_eq!(terminal.status, qol_dev_env::ReportStatus::Failed);
        assert!(terminal.cleanup.is_complete());
        assert_eq!(terminal.owner.worktree, Some(PathBuf::from(root.path())));
    }

    #[test]
    fn unresolved_pre_vm_process_tree_keeps_report_recoverable_and_lease_blocking() {
        let root = tempfile::tempdir().unwrap();
        let report_path = root.path().join("imports/image-import-test/report.json");
        fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        let mut plan = plan_fixture(root.path());
        plan.report_path = report_path.clone();
        write_initial_report(&plan).unwrap();

        terminalize_report(
            &plan,
            "cleanup-incomplete",
            workflow_json(
                &plan,
                &Verification::cancelled(Vec::new()),
                None,
                "not-published",
                None,
            ),
            Some("fixture cleanup failure"),
            true,
            false,
            &LifecycleCleanupProof::not_started(false),
        )
        .unwrap();

        let terminal =
            qol_dev_env::read_report_checked(&report_path, &plan.run_id, &ReportKind::ImageImport)
                .unwrap()
                .unwrap();
        assert_eq!(
            terminal.status,
            qol_dev_env::ReportStatus::CleanupIncomplete
        );
        assert!(!terminal.cleanup.is_complete());
        assert_eq!(terminal.document()["teardown"]["tree_exit_verified"], false);
        assert!(!fs::metadata(report_path).unwrap().permissions().readonly());
    }

    #[test]
    fn unresolved_started_vm_never_becomes_synthetic_cleanup_proof() {
        let root = tempfile::tempdir().unwrap();
        let report_path = root.path().join("imports/image-import-test/report.json");
        fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        let mut plan = plan_fixture(root.path());
        plan.report_path = report_path.clone();
        write_initial_report(&plan).unwrap();
        let cleanup = LifecycleCleanupProof {
            qemu_started: true,
            qemu_exit_verified: false,
            tree_exit_verified: false,
            artifacts_removed: false,
            error: Some("injected VM cleanup failure".to_string()),
        };

        terminalize_report(
            &plan,
            "failed",
            workflow_json(
                &plan,
                &Verification::failed(Vec::new(), "boot failed"),
                None,
                "not-published",
                None,
            ),
            Some("boot failed"),
            true,
            true,
            &cleanup,
        )
        .unwrap();

        let terminal =
            qol_dev_env::read_report_checked(&report_path, &plan.run_id, &ReportKind::ImageImport)
                .unwrap()
                .unwrap();
        assert_eq!(
            terminal.status,
            qol_dev_env::ReportStatus::CleanupIncomplete
        );
        assert!(!terminal.cleanup.is_complete());
        assert_eq!(terminal.document()["teardown"]["qemu_started"], true);
        assert_eq!(terminal.document()["teardown"]["qemu_exit_verified"], false);
        assert_eq!(terminal.document()["teardown"]["tree_exit_verified"], false);
        assert!(!fs::metadata(report_path).unwrap().permissions().readonly());
    }
}
