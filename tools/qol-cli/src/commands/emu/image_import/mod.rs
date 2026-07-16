mod plan;
mod probes;
mod reconcile;
mod report;
mod storage;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use qol_dev_env::{
    EnvironmentDefinition, ReportKind, ReportStatus, VerifiedImageRegistration,
    VERIFIED_IMAGE_PROVENANCE,
};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::guest::GuestAdapter;
use super::launch::{DisplayMode, LaunchOptions};
use super::machine::LifecycleCleanupProof;
use super::{BackendImageKind, BackendSpec};
use plan::SourceStamp;
use probes::Verification;
use storage::StagedImage;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageImportRequest {
    pub(crate) environment_id: String,
    pub(crate) source: PathBuf,
    pub(crate) run_id: Option<String>,
    pub(crate) worktree: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageImportPlan {
    pub(crate) run_id: String,
    pub(crate) report_path: PathBuf,
    pub(crate) image_root: PathBuf,
    pub(crate) config_path: PathBuf,
    environment: EnvironmentDefinition,
    source: PathBuf,
    source_stamp: SourceStamp,
    source_virtual_size: u64,
    worktree: PathBuf,
    guest_adapter: GuestAdapter,
    guest_revision: String,
    backend: BackendSpec,
    started_at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageImportReceipt {
    pub(crate) run_id: String,
    pub(crate) image_path: PathBuf,
    pub(crate) report_path: PathBuf,
}

pub(super) struct ImportCancellation {
    signals: qol_process::CancellationToken,
    inbox: qol_dev_env::CancellationInbox,
}

impl ImportCancellation {
    fn install(run_id: &str) -> Result<Self> {
        Ok(Self {
            signals: qol_process::CancellationToken::install()
                .context("failed to install image-import cancellation handlers")?,
            inbox: qol_dev_env::CancellationInbox::for_run(run_id)?,
        })
    }

    pub(super) fn is_requested(&self) -> bool {
        self.signals.is_cancelled() || self.inbox.is_requested().unwrap_or(true)
    }

    fn check(&self) -> Result<()> {
        if self.is_requested() {
            bail!("image import cancelled");
        }
        Ok(())
    }
}

pub(crate) use plan::plan_image_import;
pub(crate) use reconcile::reconcile_leased_imports;

impl ImageImportPlan {
    pub(crate) fn fingerprint(&self) -> Result<String> {
        let source_modified = self.source_stamp.modified.map(|modified| {
            modified
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| ("after", duration.as_nanos().to_string()))
                .unwrap_or_else(|error| ("before", error.duration().as_nanos().to_string()))
        });
        let identity = json!({
            "schema": 1,
            "run_id": self.run_id,
            "report_path": self.report_path,
            "image_root": self.image_root,
            "config_path": self.config_path,
            "environment": {
                "id": self.environment.id,
                "name": self.environment.name,
                "family": self.environment.family,
                "backend": self.environment.backend,
                "image": {
                    "kind": self.environment.image.kind,
                    "base": self.environment.image.base,
                    "recommended_size_gb": self.environment.image.recommended_size_gb,
                    "arch": self.environment.image.arch,
                    "firmware": self.environment.image.firmware,
                },
                "boot": {
                    "memory_mb": self.environment.boot.memory_mb,
                    "cpus": self.environment.boot.cpus,
                    "display": self.environment.boot.display,
                },
                "mounts": { "workspace": self.environment.mounts.workspace },
                "capabilities": self.environment.capabilities,
                "source": self.environment.source,
            },
            "source": self.source,
            "source_stamp": {
                "size_bytes": self.source_stamp.size_bytes,
                "modified": source_modified,
            },
            "source_virtual_size": self.source_virtual_size,
            "worktree": self.worktree,
            "guest_adapter": self.guest_adapter.as_str(),
            "guest_revision": self.guest_revision,
            "backend": {
                "arch": self.backend.arch.as_str(),
                "firmware": self.backend.firmware.as_str(),
                "image_kind": self.backend.image_kind.as_str(),
                "acceleration": self.backend.acceleration.as_str(),
            },
        });
        let encoded = serde_json::to_vec(&identity)
            .context("failed to encode the immutable image-import plan")?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}

pub(crate) fn execute_image_import(
    plan: ImageImportPlan,
    verbose: bool,
) -> Result<ImageImportReceipt> {
    let cancellation = ImportCancellation::install(&plan.run_id)?;
    cancellation.check()?;
    let run_dir = plan
        .report_path
        .parent()
        .context("verification report has no run directory")?;
    report::refuse_existing_run(run_dir, &plan.report_path)?;
    crate::commands::env::reconcile_for_admission()?;
    crate::commands::flow::reconcile_all()?;
    crate::commands::dev_env::reconcile_resources()?;
    let profile = qol_dev_env::resources::profile(
        plan.environment.boot.memory_mb,
        u64::from(plan.environment.boot.cpus),
    )?;
    let (_, lease) = qol_dev_env::resources::reserve(
        &plan.run_id,
        &plan.report_path,
        qol_dev_env::resources::AdmissionRequest {
            concurrent_lanes: 1,
            profile,
            recommended_size_gb: plan.environment.image.recommended_size_gb,
            capacity: crate::commands::dev_env::host_capacity(&plan.image_root),
            force: false,
        },
    )?;
    match import_reserved(&plan, &cancellation, verbose) {
        Ok(receipt) => {
            lease
                .release()
                .context("failed to release the image-import resource lease")?;
            Ok(receipt)
        }
        Err(error) if plan.report_path.is_file() => {
            match qol_dev_env::read_report_checked(
                &plan.report_path,
                &plan.run_id,
                &ReportKind::ImageImport,
            ) {
                Ok(Some(report))
                    if report.cleanup.is_complete()
                        && matches!(
                            report.status,
                            ReportStatus::Failed | ReportStatus::Cancelled | ReportStatus::Skipped
                        ) =>
                {
                    lease.release().with_context(|| {
                        format!(
                            "{error:#}; failed to release the terminal image-import resource lease"
                        )
                    })?;
                }
                _ => lease.retain(),
            }
            Err(error)
        }
        Err(error) => {
            let cleanup = std::fs::remove_dir_all(run_dir).err();
            let rollback = lease.rollback_unpublished().err();
            Err(combine_errors(error, cleanup, rollback))
        }
    }
}

fn import_reserved(
    plan: &ImageImportPlan,
    cancellation: &ImportCancellation,
    verbose: bool,
) -> Result<ImageImportReceipt> {
    let run_dir = plan.report_path.parent().unwrap();
    std::fs::create_dir_all(run_dir.parent().unwrap())
        .with_context(|| format!("failed to create report root for {}", plan.run_id))?;
    std::fs::create_dir(run_dir)
        .with_context(|| format!("failed to create image-import run {}", run_dir.display()))?;
    report::write_initial_report(plan)?;
    let stage_path = run_dir.join("source.qcow2");
    let staged = match storage::stage_source(plan, &stage_path, cancellation, verbose) {
        Ok(staged) => staged,
        Err(failure) => {
            let status = if cancellation.is_requested() {
                "cancelled"
            } else {
                "failed"
            };
            return finish_without_vm(
                plan,
                &stage_path,
                None,
                status,
                failure.error,
                LifecycleCleanupProof::not_started(failure.process_tree_exit_verified),
            );
        }
    };
    if let Err(error) = cancellation.check() {
        return finish_without_vm(
            plan,
            &stage_path,
            Some(&staged),
            "cancelled",
            error,
            LifecycleCleanupProof::not_started(true),
        );
    }
    let mut launch = import_launch(plan, &staged.path)?;
    launch.worktree = Some(plan.worktree.clone());
    let vm = match super::boot_vm_with_external_admission(launch, "image-import", verbose) {
        Ok(vm) => vm,
        Err(failure) => {
            let error = match &failure.cleanup.error {
                Some(cleanup_error) => anyhow::anyhow!(
                    "{:#}; VM lifecycle cleanup failed: {cleanup_error}",
                    failure.error
                ),
                None => failure.error,
            };
            return finish_without_vm(
                plan,
                &stage_path,
                Some(&staged),
                "failed",
                error,
                failure.cleanup,
            );
        }
    };
    let verification = match probes::verify_guest(plan, &vm, cancellation) {
        Ok(verification) => verification,
        Err(_error) if cancellation.is_requested() => Verification::cancelled(Vec::new()),
        Err(error) => Verification::failed(Vec::new(), format!("{error:#}")),
    };
    let mut vm = vm;
    let active_workflow =
        report::workflow_json(plan, &verification, Some(&staged), "pending", None);
    let stopping_report = vm.write_stopping_report(Some(active_workflow.clone()), "image-import");
    let shutdown = super::shutdown_after_report_attempt(stopping_report, || {
        super::shutdown_vm(&mut vm).with_context(|| {
            format!(
                "failed to stop verified image-import VM `{}`; cleanup remains unresolved",
                plan.run_id
            )
        })
    });
    let (exit, stopping_report_error) = match shutdown {
        Ok(shutdown) => shutdown,
        Err(shutdown_error) => {
            return match verification.error.as_deref() {
                Some(verification_error) => {
                    Err(anyhow::anyhow!("{verification_error}; {shutdown_error:#}"))
                }
                None => Err(shutdown_error),
            }
        }
    };
    if let Err(error) = super::finish_image_import_vm(vm, exit, active_workflow) {
        let error = super::with_stopping_report_error(error, stopping_report_error.as_ref());
        return match verification.error.as_deref() {
            Some(verification_error) => Err(anyhow::anyhow!("{verification_error}; {error:#}")),
            None => Err(error),
        };
    }
    if let Some(report_error) = stopping_report_error {
        let error = verification.error.as_deref().map_or_else(
            || format!("failed to persist pre-shutdown image verification evidence: {report_error:#}"),
            |verification_error| {
                format!(
                    "{verification_error}; failed to persist pre-shutdown image verification evidence: {report_error:#}"
                )
            },
        );
        return finish_verification_failure(
            plan,
            &staged,
            Verification::failed(verification.probes, error),
        );
    }
    if cancellation.is_requested() {
        return finish_verification_failure(
            plan,
            &staged,
            Verification::cancelled(verification.probes),
        );
    }
    if verification.verdict != "pass" {
        return finish_verification_failure(plan, &staged, verification);
    }
    let promotion = match storage::promote_image(plan, &staged, || cancellation.is_requested()) {
        Ok(promotion) => promotion,
        Err(_error) if cancellation.is_requested() => {
            return finish_verification_failure(
                plan,
                &staged,
                Verification::cancelled(verification.probes),
            )
        }
        Err(error) => {
            cleanup_stage_or_mark(
                plan,
                &staged,
                &verification,
                "not-published",
                None,
                format!("{error:#}"),
            )?;
            report::terminalize_report(
                plan,
                "failed",
                report::workflow_json(plan, &verification, Some(&staged), "failed", None),
                Some(&format!("{error:#}")),
                true,
                true,
                &LifecycleCleanupProof::verified_vm(),
            )?;
            return Err(error);
        }
    };
    cleanup_stage_or_mark(
        plan,
        &staged,
        &verification,
        "published",
        Some(&promotion),
        "failed to remove the staged source".to_string(),
    )?;
    let registration = VerifiedImageRegistration {
        path: staged.image_path.clone(),
        revision: plan.guest_revision.clone(),
        sha256: staged.sha256.clone(),
        size_bytes: staged.size_bytes,
        run_id: plan.run_id.clone(),
        report: plan.report_path.clone(),
        provenance: VERIFIED_IMAGE_PROVENANCE.to_string(),
    };
    if let Err(error) =
        qol_dev_env::register_verified_image(&plan.config_path, &plan.environment.id, &registration)
    {
        report::terminalize_report(
            plan,
            "failed",
            report::workflow_json(
                plan,
                &verification,
                Some(&staged),
                "published",
                Some(&promotion),
            ),
            Some(&format!(
                "failed to publish local image registration: {error:#}"
            )),
            true,
            true,
            &LifecycleCleanupProof::verified_vm(),
        )?;
        return Err(error).context("failed to publish verified image registration");
    }
    report::terminalize_report(
        plan,
        "pass",
        report::workflow_json(
            plan,
            &verification,
            Some(&staged),
            "published",
            Some(&promotion),
        ),
        None,
        true,
        true,
        &LifecycleCleanupProof::verified_vm(),
    )?;
    Ok(ImageImportReceipt {
        run_id: plan.run_id.clone(),
        image_path: staged.image_path,
        report_path: plan.report_path.clone(),
    })
}

fn finish_verification_failure(
    plan: &ImageImportPlan,
    staged: &StagedImage,
    verification: Verification,
) -> Result<ImageImportReceipt> {
    let status = if verification.verdict == "cancelled" {
        "cancelled"
    } else {
        "failed"
    };
    let error = verification
        .error
        .clone()
        .unwrap_or_else(|| "guest verification failed".to_string());
    cleanup_stage_or_mark(
        plan,
        staged,
        &verification,
        "not-published",
        None,
        error.clone(),
    )?;
    report::terminalize_report(
        plan,
        status,
        report::workflow_json(plan, &verification, Some(staged), "not-published", None),
        Some(&error),
        true,
        true,
        &LifecycleCleanupProof::verified_vm(),
    )?;
    bail!("{error}")
}

fn finish_without_vm<T>(
    plan: &ImageImportPlan,
    stage_path: &Path,
    staged: Option<&StagedImage>,
    status: &str,
    error: anyhow::Error,
    cleanup: LifecycleCleanupProof,
) -> Result<T> {
    let verification = if status == "cancelled" {
        Verification::cancelled(Vec::new())
    } else {
        Verification::failed(Vec::new(), format!("{error:#}"))
    };
    if let Err(cleanup_error) = storage::remove_stage(stage_path) {
        report::terminalize_report(
            plan,
            "cleanup-incomplete",
            report::workflow_json(plan, &verification, staged, "not-published", None),
            Some(&format!(
                "{error:#}; stage cleanup failed: {cleanup_error:#}"
            )),
            false,
            false,
            &cleanup,
        )?;
        bail!("{error:#}; stage cleanup failed: {cleanup_error:#}");
    }
    let terminal_status = if cleanup.is_complete() {
        status
    } else {
        "cleanup-incomplete"
    };
    report::terminalize_report(
        plan,
        terminal_status,
        report::workflow_json(plan, &verification, staged, "not-published", None),
        Some(&format!("{error:#}")),
        true,
        cleanup.is_complete(),
        &cleanup,
    )?;
    Err(error)
}

fn cleanup_stage_or_mark(
    plan: &ImageImportPlan,
    staged: &StagedImage,
    verification: &Verification,
    promotion_status: &str,
    promotion: Option<&serde_json::Value>,
    error: String,
) -> Result<()> {
    let cleanup = if promotion.is_some() {
        storage::remove_promoted_stage(staged)
    } else {
        storage::remove_stage(&staged.path)
    };
    if let Err(cleanup_error) = cleanup {
        report::terminalize_report(
            plan,
            "cleanup-incomplete",
            report::workflow_json(
                plan,
                verification,
                Some(staged),
                promotion_status,
                promotion,
            ),
            Some(&format!("{error}; stage cleanup failed: {cleanup_error:#}")),
            false,
            false,
            &LifecycleCleanupProof::verified_vm(),
        )?;
        bail!("{error}; stage cleanup failed: {cleanup_error:#}");
    }
    Ok(())
}

fn import_launch(plan: &ImageImportPlan, stage_path: &Path) -> Result<LaunchOptions> {
    let target = stage_path
        .to_str()
        .with_context(|| format!("staged image path is not UTF-8: {}", stage_path.display()))?;
    let mut launch = LaunchOptions::new(target);
    launch.environment_id = Some(plan.environment.id.clone());
    launch.display = DisplayMode::None;
    launch.offline = true;
    launch.memory_mb = u32::try_from(plan.environment.boot.memory_mb)
        .context("environment memory does not fit in u32")?;
    launch.cpus = plan.environment.boot.cpus;
    launch.run_id = Some(plan.run_id.clone());
    launch.guest_adapter = Some(plan.guest_adapter);
    launch.guest_image_revision = Some(plan.guest_revision.clone());
    launch.image_import_config = Some(plan.config_path.clone());
    launch.run_root = plan
        .report_path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    launch.image_kind = Some(BackendImageKind::Qcow2);
    launch.acceleration = plan.backend.acceleration;
    launch.arch = Some(plan.backend.arch);
    launch.firmware = Some(plan.backend.firmware);
    Ok(launch)
}

fn combine_errors(
    primary: anyhow::Error,
    cleanup: Option<std::io::Error>,
    rollback: Option<anyhow::Error>,
) -> anyhow::Error {
    let mut errors = vec![format!("{primary:#}")];
    if let Some(cleanup) = cleanup {
        errors.push(format!("run-directory cleanup failed: {cleanup}"));
    }
    if let Some(rollback) = rollback {
        errors.push(format!("resource rollback failed: {rollback:#}"));
    }
    anyhow::anyhow!(errors.join("; "))
}

#[cfg(test)]
pub(super) mod tests {
    use std::path::Path;

    use super::*;

    pub(super) fn plan_fixture(root: &Path) -> ImageImportPlan {
        let environment = qol_dev_env::registry::parse_definition(
            r#"
id = "linux/mint-cinnamon"
name = "Linux Mint Cinnamon"
family = "linux"
backend = "qemu"

[image]
kind = "qcow2"
base = "mint.qcow2"
recommended_size_gb = 40
arch = "x86_64"
firmware = "uefi"

[boot]
memory_mb = 3072
cpus = 2
display = "headless"

[mounts]
workspace = false

[capabilities]
acceleration = "allow-tcg"
flow_adapter = "mint-cinnamon"
image_revision = "revision-1"
mint_release = "22.3"
mint_edition = "Cinnamon"
cinnamon_version = "6.6.7"
"#,
            Path::new("mint.toml"),
        )
        .unwrap();
        let backend = BackendSpec::from_manifest(
            &environment.backend,
            &environment.image.kind,
            environment.image.arch.as_deref(),
            environment.image.firmware.as_deref(),
            environment
                .capabilities
                .get("acceleration")
                .map(String::as_str),
        )
        .unwrap();
        ImageImportPlan {
            run_id: "image-import-test".to_string(),
            report_path: root.join("verified/imports/image-import-test/report.json"),
            image_root: root.to_path_buf(),
            config_path: root.join("dev-envs.toml"),
            environment,
            source: root.join("source.qcow2"),
            source_stamp: SourceStamp {
                size_bytes: 5,
                modified: None,
            },
            source_virtual_size: 1024,
            worktree: root.to_path_buf(),
            guest_adapter: GuestAdapter::MintCinnamon,
            guest_revision: "revision-1".to_string(),
            backend,
            started_at_unix_ms: 1,
        }
    }
}
