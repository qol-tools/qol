use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use qol_dev_env::ReportKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunTicket {
    pub run_id: String,
    pub kind: ReportKind,
    pub report_path: PathBuf,
}

impl RunTicket {
    pub fn new(run_id: String, kind: ReportKind, report_path: PathBuf) -> Result<Self> {
        qol_dev_env::validate_run_id(&run_id)?;
        crate::request::validate_absolute_path(&report_path, "run report path")?;
        Ok(Self {
            run_id,
            kind,
            report_path,
        })
    }

    pub fn read(&self) -> Result<Option<qol_dev_env::RunReport>> {
        qol_dev_env::read_report_checked(&self.report_path, &self.run_id, &self.kind)
    }

    pub fn cancel(&self) -> Result<PathBuf> {
        qol_dev_env::request_cancellation(&self.run_id)
    }

    pub fn worker_log_path(&self) -> Result<PathBuf> {
        let worker_root = self.validate_worker_layout()?;
        Ok(worker_root
            .join(".workers")
            .join(format!("{}.log", self.run_id)))
    }

    pub(super) fn validate_worker_layout(&self) -> Result<&Path> {
        if self.report_path.file_name().and_then(|name| name.to_str()) != Some("report.json") {
            bail!("worker report must be named report.json");
        }
        let run_dir = self
            .report_path
            .parent()
            .context("worker report has no run directory")?;
        if run_dir.file_name().and_then(|name| name.to_str()) != Some(self.run_id.as_str()) {
            bail!("worker report directory does not match its run id");
        }
        let worker_root = run_dir
            .parent()
            .context("worker report has no worker root directory")?;
        validate_kind_layout(self, worker_root)?;
        Ok(worker_root)
    }
}

fn validate_kind_layout(ticket: &RunTicket, worker_root: &Path) -> Result<()> {
    match ticket.kind {
        ReportKind::FlowFanout => validate_parent_name(worker_root, "flows"),
        ReportKind::ImageImport => validate_image_layout(ticket, worker_root),
        _ => bail!(
            "report kind `{}` has no typed worker layout",
            ticket.kind.as_str()
        ),
    }
}

fn validate_image_layout(ticket: &RunTicket, worker_root: &Path) -> Result<()> {
    let verified = worker_root
        .parent()
        .context("image import report has no verified directory")?;
    let image_root = verified
        .parent()
        .context("image import report has no image root")?;
    let expected = qol_dev_env::managed_verification_report_path(image_root, &ticket.run_id)?;
    if ticket.report_path != expected {
        bail!("image import report is outside the managed verification layout");
    }
    Ok(())
}

fn validate_parent_name(path: &Path, expected: &str) -> Result<()> {
    if path.file_name().and_then(|name| name.to_str()) != Some(expected) {
        bail!("worker report is outside a {expected} directory");
    }
    Ok(())
}
