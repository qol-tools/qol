use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_dev_env::{
    CleanupState, LocalConfig, LocalImage, ReportKind, ReportStatus, RunReport,
    VerifiedImageRegistration, VERIFIED_IMAGE_PROVENANCE,
};
use serde_json::{json, Value};

use super::storage;

const RECONCILE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const RECONCILE_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImportLayout {
    image_root: PathBuf,
    run_dir: PathBuf,
    report_path: PathBuf,
    stage_path: PathBuf,
    conversion_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Publication {
    NotPublished,
    PublishedUnregistered { image_path: PathBuf },
    Registered { image_path: PathBuf },
}

pub(crate) fn reconcile_leased_imports() -> Result<()> {
    let inspection = qol_dev_env::resources::inspect()?;
    let config_path = crate::commands::dev_env::config_path();
    let mut failures = Vec::new();
    for lease in inspection.leases {
        if let Err(error) = reconcile_report_if_image_import(
            &lease.report_path,
            &lease.lease_id,
            config_path.as_deref(),
        ) {
            failures.push(format!("{}: {error:#}", lease.lease_id));
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    bail!(
        "failed to reconcile verified image imports:\n{}",
        failures.join("\n")
    )
}

fn reconcile_report_if_image_import(
    report_path: &Path,
    expected_run_id: &str,
    config_path: Option<&Path>,
) -> Result<()> {
    let Some(report) = qol_dev_env::read_report(report_path)? else {
        return Ok(());
    };
    if report.kind != ReportKind::ImageImport {
        return Ok(());
    }
    if report.run_id != expected_run_id {
        bail!(
            "image-import report belongs to `{}`, expected `{expected_run_id}`",
            report.run_id
        );
    }
    let layout = ImportLayout::checked(report_path, expected_run_id)?;
    reconcile_import(&layout, expected_run_id, config_path)
}

fn reconcile_import(
    layout: &ImportLayout,
    expected_run_id: &str,
    config_path: Option<&Path>,
) -> Result<()> {
    let report = read_exact_report(layout, expected_run_id)?;
    if report.cleanup == CleanupState::Complete {
        return ensure_completed_report_durability(&report);
    }
    let owner_pid = report
        .owner
        .pid
        .context("image-import report has no durable owner PID")?;
    let owner_identity = report
        .owner
        .process_identity
        .as_deref()
        .context("image-import report has no durable owner process identity")?;
    if process_identity_alive(owner_pid, owner_identity) {
        return Ok(());
    }
    if vm_may_have_started(report.document()) && !vm_cleanup_complete(report.document()) {
        if let Err(error) = super::super::live::reconcile_exact_image_import_vm(
            &layout.run_dir,
            expected_run_id,
            owner_pid,
            owner_identity,
        ) {
            return mark_cleanup_incomplete(
                layout,
                expected_run_id,
                "vm-identity",
                &format!("owned VM cleanup could not be proven: {error:#}"),
            );
        }
    }
    let report = read_exact_report(layout, expected_run_id)?;
    if vm_may_have_started(report.document()) && !vm_cleanup_complete(report.document()) {
        return Ok(());
    }
    let _lock = lock_reconciliation(layout, RECONCILE_LOCK_TIMEOUT)?;
    let report = read_exact_report(layout, expected_run_id)?;
    if process_identity_alive(owner_pid, owner_identity) {
        return Ok(());
    }
    if vm_may_have_started(report.document()) && !vm_cleanup_complete(report.document()) {
        return Ok(());
    }
    if !vm_may_have_started(report.document()) {
        if let Err(error) = require_dead_pre_vm_tree(layout, &report, owner_pid) {
            return commit_cleanup_incomplete(
                layout,
                &report,
                "pre-vm-tree",
                &format!("{error:#}"),
            );
        }
    }
    if let Err(error) = remove_staging(layout) {
        return commit_cleanup_incomplete(layout, &report, "staging", &format!("{error:#}"));
    }
    let publication = match inspect_publication(layout, &report, config_path) {
        Ok(publication) => publication,
        Err(error) => {
            return commit_cleanup_incomplete(layout, &report, "publication", &format!("{error:#}"))
        }
    };
    commit_recovered(layout, &report, publication)
}

impl ImportLayout {
    fn checked(report_path: &Path, run_id: &str) -> Result<Self> {
        if !report_path.is_absolute() {
            bail!("image-import report path is not absolute");
        }
        if report_path.file_name().and_then(|name| name.to_str()) != Some("report.json") {
            bail!("image-import report is not named report.json");
        }
        let run_dir = report_path
            .parent()
            .context("image-import report has no run directory")?;
        if run_dir.file_name().and_then(|name| name.to_str()) != Some(run_id) {
            bail!("image-import run directory does not match its run id");
        }
        let imports = run_dir
            .parent()
            .context("image-import run has no imports directory")?;
        let verified = imports
            .parent()
            .context("image-import run has no verified directory")?;
        let image_root = verified
            .parent()
            .context("image-import run has no image root")?;
        let expected = qol_dev_env::managed_verification_report_path(image_root, run_id)?;
        if expected != report_path {
            bail!("image-import report is outside the managed verification layout");
        }
        require_real_directory(image_root, "image root")?;
        require_real_directory(verified, "verified image directory")?;
        require_real_directory(imports, "verified import directory")?;
        require_real_directory(run_dir, "image-import run directory")?;
        require_real_file(report_path, "image-import report")?;
        let stage_path = run_dir.join("source.qcow2");
        Ok(Self {
            image_root: image_root.to_path_buf(),
            run_dir: run_dir.to_path_buf(),
            report_path: report_path.to_path_buf(),
            conversion_path: storage::conversion_journal_path(&stage_path),
            stage_path,
        })
    }
}

fn read_exact_report(layout: &ImportLayout, run_id: &str) -> Result<RunReport> {
    let report =
        qol_dev_env::read_report_checked(&layout.report_path, run_id, &ReportKind::ImageImport)?
            .context("image-import report disappeared during reconciliation")?;
    validate_report_layout(layout, &report)?;
    Ok(report)
}

fn validate_report_layout(layout: &ImportLayout, report: &RunReport) -> Result<()> {
    let document = report.document();
    let declared_run_dir = json_path(document, &["artifacts", "run_dir"])
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("image-import report has no artifacts.run_dir")?;
    let declared_report = json_path(document, &["artifacts", "report"])
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("image-import report has no artifacts.report")?;
    if declared_run_dir != layout.run_dir || declared_report != layout.report_path {
        bail!("image-import report artifact identity does not match its managed layout");
    }
    if report.owner.task.as_deref() != Some("image-import-verification") {
        bail!("image-import report has an unexpected owner task");
    }
    let worktree = report
        .owner
        .worktree
        .as_deref()
        .context("image-import report has no owner worktree")?;
    if !worktree.is_absolute() {
        bail!("image-import owner worktree is not absolute");
    }
    if json_path(document, &["launch", "display"]).and_then(Value::as_str) != Some("none")
        || json_path(document, &["launch", "network"]).and_then(Value::as_str) != Some("none")
    {
        bail!("image-import report does not prove an offline headless launch");
    }
    if let Some(staging) = json_path(document, &["workflow", "staging", "path"])
        .and_then(Value::as_str)
        .map(PathBuf::from)
    {
        if staging != layout.stage_path {
            bail!("image-import staging path does not match its managed run directory");
        }
    }
    if let Some(config_path) = json_path(document, &["artifacts", "image_import_config"])
        .and_then(Value::as_str)
        .map(PathBuf::from)
    {
        if !config_path.is_absolute() {
            bail!("image-import config path is not absolute");
        }
    }
    Ok(())
}

fn require_dead_pre_vm_tree(
    layout: &ImportLayout,
    report: &RunReport,
    owner_pid: u32,
) -> Result<()> {
    if qol_process::is_group_alive(owner_pid) {
        bail!("image-import owner process tree {owner_pid} is still alive");
    }
    let journal = read_conversion_journal(layout)?;
    let stage_exists = fs::symlink_metadata(&layout.stage_path).is_ok();
    let Some(journal) = journal else {
        if stage_exists {
            bail!("staged image exists without a durable conversion identity");
        }
        return Ok(());
    };
    validate_conversion_journal(layout, report, &journal)?;
    let pid = journal
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0);
    let process_group = journal
        .get("process_group")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0);
    if pid.is_some() != process_group.is_some() || pid != process_group {
        bail!("conversion journal has no exact owned process-group identity");
    }
    let process_identity = journal.get("process_identity").and_then(Value::as_str);
    let exact_process_alive = pid
        .zip(process_identity)
        .is_some_and(|(pid, identity)| process_identity_alive(pid, identity));
    let unidentified_process_alive = pid.is_some_and(process_alive) && process_identity.is_none();
    if exact_process_alive
        || unidentified_process_alive
        || process_group.is_some_and(qol_process::is_group_alive)
    {
        bail!("qemu-img conversion process tree is still alive or its identity was reused");
    }
    if journal.get("tree_exit_verified").and_then(Value::as_bool) == Some(true) || pid.is_some() {
        return Ok(());
    }
    bail!("qemu-img conversion launch has no terminal process-tree proof")
}

fn read_conversion_journal(layout: &ImportLayout) -> Result<Option<Value>> {
    let metadata = match fs::symlink_metadata(&layout.conversion_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", layout.conversion_path.display()))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("conversion journal is not a regular non-symlink file");
    }
    let content = fs::read(&layout.conversion_path)
        .with_context(|| format!("failed to read {}", layout.conversion_path.display()))?;
    serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse {}", layout.conversion_path.display()))
        .map(Some)
}

fn validate_conversion_journal(
    layout: &ImportLayout,
    report: &RunReport,
    journal: &Value,
) -> Result<()> {
    if journal.get("run_id").and_then(Value::as_str) != Some(report.run_id.as_str()) {
        bail!("conversion journal run identity does not match its report");
    }
    let destination = journal
        .get("destination")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("conversion journal has no destination")?;
    if destination != layout.stage_path {
        bail!("conversion journal destination is outside the managed run directory");
    }
    let program = journal
        .get("program")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .context("conversion journal has no program")?;
    if !program.is_absolute() {
        bail!("conversion journal program is not absolute");
    }
    if let Some(source) = json_path(report.document(), &["workflow", "source", "path"])
        .and_then(Value::as_str)
        .map(PathBuf::from)
    {
        let journal_source = journal
            .get("source")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .context("conversion journal has no source")?;
        if journal_source != source {
            bail!("conversion journal source does not match its report");
        }
    }
    Ok(())
}

fn inspect_publication(
    layout: &ImportLayout,
    report: &RunReport,
    config_path: Option<&Path>,
) -> Result<Publication> {
    let environment_id = report
        .environment_id
        .as_deref()
        .context("image-import report has no environment identity")?;
    let config_path = exact_config_path(report.document(), config_path)?;
    let config = qol_dev_env::registry::load_local_config(&config_path)?;
    let digest =
        json_path(report.document(), &["workflow", "source", "sha256"]).and_then(Value::as_str);
    let Some(digest) = digest else {
        refuse_same_run_registration(&config, environment_id, &report.run_id)?;
        return Ok(Publication::NotPublished);
    };
    let size_bytes = json_path(report.document(), &["workflow", "source", "size_bytes"])
        .and_then(Value::as_u64)
        .context("verified image-import report has no source size")?;
    let revision = json_path(report.document(), &["launch", "guest_image_revision"])
        .and_then(Value::as_str)
        .context("verified image-import report has no guest image revision")?;
    let image_path = qol_dev_env::managed_verified_image_path(&layout.image_root, digest)?;
    if let Some(reported_path) =
        json_path(report.document(), &["workflow", "promotion", "image_path"])
            .and_then(Value::as_str)
            .map(PathBuf::from)
    {
        if reported_path != image_path {
            bail!("image-import promotion path does not match its content identity");
        }
    }
    let metadata = match fs::symlink_metadata(&image_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            refuse_same_run_registration(&config, environment_id, &report.run_id)?;
            return Ok(Publication::NotPublished);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", image_path.display()))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("managed image target is not a regular non-symlink file");
    }
    if metadata.len() != size_bytes {
        bail!("managed image target size does not match its verified source");
    }
    if !metadata.permissions().readonly() {
        bail!("managed image target is writable");
    }
    if storage::sha256_file(&image_path, || false)? != digest {
        bail!("managed image target digest does not match its content path");
    }
    let expected = VerifiedImageRegistration {
        path: image_path.clone(),
        revision: revision.to_string(),
        sha256: digest.to_string(),
        size_bytes,
        run_id: report.run_id.clone(),
        report: layout.report_path.clone(),
        provenance: VERIFIED_IMAGE_PROVENANCE.to_string(),
    };
    match config.images.get(environment_id) {
        Some(LocalImage::Verified(actual)) if actual == &expected => {
            require_passing_verification(report.document())?;
            Ok(Publication::Registered { image_path })
        }
        Some(LocalImage::Verified(actual)) if actual.run_id == report.run_id => {
            bail!("local image registration for this run conflicts with its verified report")
        }
        _ => Ok(Publication::PublishedUnregistered { image_path }),
    }
}

fn exact_config_path(document: &Value, fallback: Option<&Path>) -> Result<PathBuf> {
    let recorded = json_path(document, &["artifacts", "image_import_config"])
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let Some(recorded) = recorded else {
        return fallback
            .map(Path::to_path_buf)
            .context("development environment config path is unavailable");
    };
    if !recorded.is_absolute() {
        bail!("recorded image-import config path is not absolute");
    }
    if fallback.is_some_and(|fallback| fallback != recorded) {
        bail!("recorded image-import config path differs from the active config path");
    }
    Ok(recorded)
}

fn refuse_same_run_registration(
    config: &LocalConfig,
    environment_id: &str,
    run_id: &str,
) -> Result<()> {
    if matches!(
        config.images.get(environment_id),
        Some(LocalImage::Verified(registration)) if registration.run_id == run_id
    ) {
        bail!("local image registration names this run but its managed image is unavailable");
    }
    Ok(())
}

fn require_passing_verification(document: &Value) -> Result<()> {
    if json_path(document, &["workflow", "verdict"]).and_then(Value::as_str) != Some("pass") {
        bail!("registered image import has no passing verification verdict");
    }
    let probes = json_path(document, &["workflow", "probes"])
        .and_then(Value::as_array)
        .context("registered image import has no verification probes")?;
    for required in [
        "linux-mint-release",
        "linux-mint-edition",
        "cinnamon-version",
    ] {
        let passed = probes.iter().any(|probe| {
            probe.get("id").and_then(Value::as_str) == Some(required)
                && probe.get("verdict").and_then(Value::as_str) == Some("pass")
        });
        if !passed {
            bail!("registered image import lacks passing probe `{required}`");
        }
    }
    Ok(())
}

fn remove_staging(layout: &ImportLayout) -> Result<()> {
    let metadata = match fs::symlink_metadata(&layout.stage_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", layout.stage_path.display()))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("staged image is not a regular non-symlink file");
    }
    storage::remove_stage(&layout.stage_path)
}

fn mark_cleanup_incomplete(
    layout: &ImportLayout,
    run_id: &str,
    phase: &str,
    error: &str,
) -> Result<()> {
    let _lock = lock_reconciliation(layout, RECONCILE_LOCK_TIMEOUT)?;
    let report = read_exact_report(layout, run_id)?;
    commit_cleanup_incomplete(layout, &report, phase, error)
}

fn commit_cleanup_incomplete(
    layout: &ImportLayout,
    report: &RunReport,
    phase: &str,
    error: &str,
) -> Result<()> {
    let mut document = report.document().clone();
    document["status"] = json!("cleanup-incomplete");
    document
        .as_object_mut()
        .context("image-import report must be an object")?
        .remove("finished_at_unix_ms");
    set_owner_state(&mut document, "cleanup-incomplete")?;
    let qemu_started = vm_may_have_started(&document);
    let qemu_exit = json_path(&document, &["teardown", "qemu_exit_verified"])
        .and_then(Value::as_bool)
        .unwrap_or(!qemu_started);
    let tree_exit = json_path(&document, &["teardown", "tree_exit_verified"])
        .and_then(Value::as_bool)
        .unwrap_or(false);
    document["teardown"]["status"] = json!("incomplete");
    document["teardown"]["phase"] = json!(phase);
    document["teardown"]["qemu_started"] = json!(qemu_started);
    document["teardown"]["qemu_exit_verified"] = json!(qemu_exit);
    document["teardown"]["tree_exit_verified"] = json!(tree_exit);
    document["teardown"]["staging_removed"] = json!(false);
    document["teardown"]["error"] = json!(error);
    document["error"] = json!(error);
    document["reconciliation"] = reconciliation(report, phase, error);
    document["next"] = json!([format!(
        "After verifying the recorded process identities, run `qol env doctor --repair` again; report: {}.",
        layout.report_path.display()
    )]);
    write_report(&layout.report_path, &document)
}

fn commit_recovered(
    layout: &ImportLayout,
    report: &RunReport,
    publication: Publication,
) -> Result<()> {
    let mut document = report.document().clone();
    let status = recovered_status(report, &publication);
    let detail = match &publication {
        Publication::NotPublished => "interrupted before managed image publication".to_string(),
        Publication::PublishedUnregistered { image_path } => format!(
            "managed content remains unregistered at {}",
            image_path.display()
        ),
        Publication::Registered { .. } => "verified registration was already durable".to_string(),
    };
    document["status"] = json!(status);
    document["finished_at_unix_ms"] = json!(qol_dev_env::unix_millis()?);
    set_owner_state(&mut document, status)?;
    document["teardown"]["status"] = json!("complete");
    document["teardown"]["phase"] = json!("recovered");
    document["teardown"]["qemu_started"] = json!(vm_may_have_started(report.document()));
    document["teardown"]["qemu_exit_verified"] = json!(true);
    document["teardown"]["tree_exit_verified"] = json!(true);
    document["teardown"]["staging_removed"] = json!(true);
    document["teardown"]
        .as_object_mut()
        .context("image-import teardown must be an object")?
        .remove("error");
    let promotion_status = match &publication {
        Publication::NotPublished => "not-published",
        Publication::PublishedUnregistered { .. } => "published-unregistered",
        Publication::Registered { .. } => "published",
    };
    document["workflow"]["promotion"]["status"] = json!(promotion_status);
    if let Publication::PublishedUnregistered { image_path }
    | Publication::Registered { image_path } = &publication
    {
        document["workflow"]["promotion"]["image_path"] = json!(image_path);
    }
    document["reconciliation"] = reconciliation(report, "complete", &detail);
    document["next"] = if status == "pass" {
        json!(["The verified image registration is ready for a sandbox run."])
    } else {
        json!([format!(
            "Inspect {} and start a fresh verified image import.",
            layout.report_path.display()
        )])
    };
    if status != "pass" {
        document["error"] = json!(format!(
            "image import was abandoned after owner loss: {detail}"
        ));
    }
    write_report(&layout.report_path, &document)?;
    if status == "pass" {
        set_readonly(&layout.report_path)?;
    }
    Ok(())
}

fn recovered_status<'a>(report: &RunReport, publication: &'a Publication) -> &'a str {
    if matches!(publication, Publication::Registered { .. }) {
        return "pass";
    }
    match report.status {
        ReportStatus::Cancelled => "cancelled",
        ReportStatus::Failed | ReportStatus::Skipped => "failed",
        _ => "abandoned",
    }
}

fn reconciliation(report: &RunReport, phase: &str, detail: &str) -> Value {
    json!({
        "previous_status": report.status.as_str(),
        "reason": "image-import owner exited before terminal cleanup",
        "phase": phase,
        "detail": detail,
    })
}

fn ensure_completed_report_durability(report: &RunReport) -> Result<()> {
    if report.status != ReportStatus::Pass {
        return Ok(());
    }
    let metadata = fs::metadata(&report.report_path)
        .with_context(|| format!("failed to inspect {}", report.report_path.display()))?;
    if metadata.permissions().readonly() {
        return Ok(());
    }
    set_readonly(&report.report_path)
}

fn lock_reconciliation(layout: &ImportLayout, timeout: Duration) -> Result<std::fs::File> {
    let path = layout.run_dir.join("reconcile.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("image-import reconcile lock timeout is too large")?;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("failed to lock {}", path.display()))
            }
        }
        let now = Instant::now();
        if now >= deadline {
            bail!(
                "timed out after {timeout:?} locking {}; cleanup remains retained",
                path.display()
            );
        }
        std::thread::sleep(RECONCILE_LOCK_POLL_INTERVAL.min(deadline.duration_since(now)));
    }
    Ok(lock)
}

fn vm_may_have_started(document: &Value) -> bool {
    json_path(document, &["runtime", "qemu_pid"])
        .and_then(Value::as_u64)
        .is_some_and(|pid| pid != 0)
        || json_path(document, &["qmp", "port"])
            .and_then(Value::as_u64)
            .is_some()
        || json_path(document, &["spawn", "state"])
            .and_then(Value::as_str)
            .is_some_and(|state| matches!(state, "launching" | "spawned"))
        || json_path(document, &["teardown", "qemu_started"]).and_then(Value::as_bool) == Some(true)
}

fn vm_cleanup_complete(document: &Value) -> bool {
    json_path(document, &["teardown", "status"]).and_then(Value::as_str) == Some("complete")
        && json_path(document, &["teardown", "qemu_exit_verified"]).and_then(Value::as_bool)
            == Some(true)
        && json_path(document, &["teardown", "tree_exit_verified"]).and_then(Value::as_bool)
            == Some(true)
}

fn process_alive(pid: u32) -> bool {
    qol_process::is_pid_alive(pid) && !qol_process::is_pid_zombie(pid)
}

fn process_identity_alive(pid: u32, identity: &str) -> bool {
    process_alive(pid) && qol_process::process_identity_matches(pid, identity)
}

fn require_real_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("{label} is not a real directory: {}", path.display());
    }
    Ok(())
}

fn require_real_file(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} is not a regular non-symlink file: {}",
            path.display()
        );
    }
    Ok(())
}

fn set_owner_state(document: &mut Value, state: &str) -> Result<()> {
    let owner = document
        .get_mut("owner")
        .and_then(Value::as_object_mut)
        .context("image-import report owner must be an object")?;
    owner.insert("state".to_string(), json!(state));
    Ok(())
}

fn set_readonly(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to make {} read-only", path.display()))
}

fn write_report(path: &Path, document: &Value) -> Result<()> {
    let mut content = serde_json::to_vec_pretty(document)
        .context("failed to encode reconciled image-import report")?;
    content.push(b'\n');
    qol_fs::atomic_write_durable(path, &content)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn json_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_fixture(root: &Path, run_id: &str, owner_pid: u32) -> (ImportLayout, Value) {
        let report_path = qol_dev_env::managed_verification_report_path(root, run_id).unwrap();
        fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        let layout = ImportLayout::checked_after_write_fixture(&report_path);
        let report = json!({
            "name": "qol-env-image-import",
            "kind": "image-import",
            "run_id": run_id,
            "started_at_unix_ms": 1,
            "status": "preparing",
            "owner": {
                "pid": owner_pid,
                "process_identity": "dead-owner-identity",
                "state": "preparing",
                "worktree": root,
                "task": "image-import-verification"
            },
            "environment": { "id": "linux/mint-cinnamon" },
            "launch": {
                "display": "none",
                "network": "none",
                "guest_image_revision": "revision-1"
            },
            "workflow": {
                "id": "image-import-verification",
                "verdict": "pending",
                "source": {
                    "path": root.join("incoming.qcow2"),
                    "sha256": null,
                    "size_bytes": 5
                },
                "staging": { "path": layout.stage_path },
                "probes": [],
                "promotion": { "status": "pending", "image_path": null }
            },
            "artifacts": {
                "run_dir": layout.run_dir,
                "report": layout.report_path
            },
            "teardown": null
        });
        write_report(&report_path, &report).unwrap();
        (ImportLayout::checked(&report_path, run_id).unwrap(), report)
    }

    impl ImportLayout {
        fn checked_after_write_fixture(report_path: &Path) -> Self {
            let run_dir = report_path.parent().unwrap().to_path_buf();
            let image_root = run_dir
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let stage_path = run_dir.join("source.qcow2");
            Self {
                image_root: image_root.to_path_buf(),
                run_dir,
                report_path: report_path.to_path_buf(),
                conversion_path: storage::conversion_journal_path(&stage_path),
                stage_path,
            }
        }
    }

    fn dead_pid() -> u32 {
        u32::MAX - 1
    }

    fn write_completed_conversion(layout: &ImportLayout, run_id: &str, source: &Path) {
        let journal = json!({
            "run_id": run_id,
            "state": "complete",
            "program": std::env::current_exe().unwrap(),
            "source": source,
            "destination": layout.stage_path,
            "pid": dead_pid(),
            "process_group": dead_pid(),
            "tree_exit_verified": true,
            "error": null
        });
        let content = serde_json::to_vec_pretty(&journal).unwrap();
        fs::write(&layout.conversion_path, content).unwrap();
    }

    #[test]
    fn dead_pre_vm_import_removes_only_exact_staging_and_becomes_terminal() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("dev-envs.toml");
        let (layout, report) = report_fixture(root.path(), "image-import-dead", dead_pid());
        fs::write(&layout.stage_path, b"stage").unwrap();
        let source = PathBuf::from(report["workflow"]["source"]["path"].as_str().unwrap());
        write_completed_conversion(&layout, "image-import-dead", &source);
        let evidence = layout.run_dir.join("evidence.txt");
        fs::write(&evidence, b"keep").unwrap();

        reconcile_import(&layout, "image-import-dead", Some(&config_path)).unwrap();

        let recovered = read_exact_report(&layout, "image-import-dead").unwrap();
        assert_eq!(recovered.status, ReportStatus::Abandoned);
        assert_eq!(recovered.cleanup, CleanupState::Complete);
        assert!(!layout.stage_path.exists());
        assert_eq!(fs::read(evidence).unwrap(), b"keep");
    }

    #[test]
    fn live_unverified_vm_identity_keeps_stage_and_lease_blocking_report() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("dev-envs.toml");
        let (layout, mut report) =
            report_fixture(root.path(), "image-import-uncertain", dead_pid());
        fs::write(&layout.stage_path, b"stage").unwrap();
        report["status"] = json!("running");
        report["runtime"] = json!({
            "supervisor_pid": dead_pid(),
            "qemu_pid": std::process::id()
        });
        report["qmp"] = json!({ "port": 1 });
        report["spawn"] = json!({
            "state": "spawned",
            "pidfile": layout.run_dir.join("qemu.pid")
        });
        write_report(&layout.report_path, &report).unwrap();

        reconcile_import(&layout, "image-import-uncertain", Some(&config_path)).unwrap();

        let recovered = read_exact_report(&layout, "image-import-uncertain").unwrap();
        assert_eq!(recovered.status, ReportStatus::CleanupIncomplete);
        assert!(matches!(recovered.cleanup, CleanupState::Incomplete(_)));
        assert!(layout.stage_path.is_file());
        assert_eq!(recovered.document()["teardown"]["phase"], "vm-identity");
    }

    #[test]
    fn durable_promotion_and_registration_recover_to_releasable_pass() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("dev-envs.toml");
        let (layout, mut report) =
            report_fixture(root.path(), "image-import-registered", dead_pid());
        fs::write(&layout.stage_path, b"stage").unwrap();
        let digest = storage::sha256_file(&layout.stage_path, || false).unwrap();
        let image_path = qol_dev_env::managed_verified_image_path(root.path(), &digest).unwrap();
        fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        fs::write(&image_path, b"stage").unwrap();
        set_readonly(&image_path).unwrap();
        report["status"] = json!("stopping");
        report["workflow"]["verdict"] = json!("pass");
        report["workflow"]["source"]["sha256"] = json!(digest);
        report["workflow"]["probes"] = json!([
            { "id": "linux-mint-release", "verdict": "pass" },
            { "id": "linux-mint-edition", "verdict": "pass" },
            { "id": "cinnamon-version", "verdict": "pass" }
        ]);
        report["workflow"]["promotion"] = json!({
            "status": "pending",
            "image_path": image_path
        });
        report["teardown"] = json!({
            "status": "complete",
            "qemu_started": true,
            "qemu_exit_verified": true,
            "tree_exit_verified": true
        });
        report["artifacts"]["image_import_config"] = json!(config_path);
        write_report(&layout.report_path, &report).unwrap();
        let registration = VerifiedImageRegistration {
            path: image_path,
            revision: "revision-1".to_string(),
            sha256: digest,
            size_bytes: 5,
            run_id: "image-import-registered".to_string(),
            report: layout.report_path.clone(),
            provenance: VERIFIED_IMAGE_PROVENANCE.to_string(),
        };
        qol_dev_env::register_verified_image(&config_path, "linux/mint-cinnamon", &registration)
            .unwrap();

        reconcile_import(&layout, "image-import-registered", Some(&config_path)).unwrap();

        let recovered = read_exact_report(&layout, "image-import-registered").unwrap();
        assert_eq!(recovered.status, ReportStatus::Pass);
        assert_eq!(recovered.cleanup, CleanupState::Complete);
        assert!(fs::metadata(&layout.report_path)
            .unwrap()
            .permissions()
            .readonly());
        assert!(!layout.stage_path.exists());
    }

    #[test]
    fn reconcile_lock_contention_is_bounded_and_retains_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let (layout, _) = report_fixture(root.path(), "image-import-locked", dead_pid());
        let lock_path = layout.run_dir.join("reconcile.lock");
        let held = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .unwrap();
        held.lock().unwrap();

        let started = Instant::now();
        let error = lock_reconciliation(&layout, Duration::from_millis(20)).unwrap_err();

        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.to_string().contains("cleanup remains retained"));
    }
}
