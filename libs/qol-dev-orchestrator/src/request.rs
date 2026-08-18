use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use qol_dev_env::ReportKind;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::RunTicket;

pub const FLOW_WORKER_COMMAND: &str = "__qol-flow-worker";
pub const IMAGE_IMPORT_WORKER_COMMAND: &str = "__qol-image-import-worker";
pub const MAX_FLOW_REPEATS: u32 = 128;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowStart {
    pub workflow: String,
    pub environment_id: String,
    pub worktree: PathBuf,
    pub run_id: String,
    pub repeat: u32,
    pub jobs: u32,
    pub memory_mb: Option<u32>,
    pub cpus: Option<u16>,
    pub force: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowWorkerRequest {
    pub start: FlowStart,
    pub run_root: PathBuf,
    pub plan_fingerprint: String,
    pub verbose: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageImportStart {
    pub environment_id: String,
    pub source: PathBuf,
    pub worktree: PathBuf,
    pub run_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageImportWorkerRequest {
    pub start: ImageImportStart,
    pub image_root: PathBuf,
    pub plan_fingerprint: String,
    pub verbose: bool,
}

impl FlowStart {
    pub fn validate(&self) -> Result<()> {
        validate_identity(&self.workflow, "workflow")?;
        validate_identity(&self.environment_id, "environment id")?;
        validate_absolute_path(&self.worktree, "flow worktree")?;
        qol_dev_env::validate_run_id(&self.run_id)?;
        if !(1..=MAX_FLOW_REPEATS).contains(&self.repeat) {
            bail!("flow repeat must be from 1 to {MAX_FLOW_REPEATS}");
        }
        if !(1..=qol_dev_env::resources::MAX_CONCURRENT_LANES).contains(&self.jobs) {
            bail!(
                "flow jobs must be from 1 to {}",
                qol_dev_env::resources::MAX_CONCURRENT_LANES
            );
        }
        validate_memory(self.memory_mb)?;
        validate_cpus(self.cpus)
    }

    pub fn ticket(&self, run_root: &Path) -> Result<RunTicket> {
        self.validate()?;
        validate_absolute_path(run_root, "flow run root")?;
        RunTicket::new(
            self.run_id.clone(),
            ReportKind::FlowFanout,
            run_root
                .join("flows")
                .join(&self.run_id)
                .join("report.json"),
        )
    }
}

impl FlowWorkerRequest {
    pub fn validate(&self) -> Result<()> {
        self.start.validate()?;
        validate_absolute_path(&self.run_root, "flow run root")?;
        validate_plan_fingerprint(&self.plan_fingerprint, "flow")
    }
}

impl ImageImportStart {
    pub fn validate(&self) -> Result<()> {
        validate_identity(&self.environment_id, "environment id")?;
        validate_absolute_path(&self.source, "image import source")?;
        validate_absolute_path(&self.worktree, "image import worktree")?;
        qol_dev_env::validate_run_id(&self.run_id)
    }

    pub fn ticket(&self, image_root: &Path) -> Result<RunTicket> {
        self.validate()?;
        validate_absolute_path(image_root, "image root")?;
        RunTicket::new(
            self.run_id.clone(),
            ReportKind::ImageImport,
            qol_dev_env::managed_verification_report_path(image_root, &self.run_id)?,
        )
    }
}

impl ImageImportWorkerRequest {
    pub fn validate(&self) -> Result<()> {
        self.start.validate()?;
        validate_absolute_path(&self.image_root, "image root")?;
        validate_plan_fingerprint(&self.plan_fingerprint, "image import")
    }
}

fn validate_plan_fingerprint(fingerprint: &str, context: &str) -> Result<()> {
    if qol_fs::is_lowercase_sha256_digest(fingerprint) {
        return Ok(());
    }
    bail!("{context} plan fingerprint must be a lowercase SHA-256 digest")
}

pub fn read_flow_worker_request(input: impl Read) -> Result<FlowWorkerRequest> {
    let request: FlowWorkerRequest = decode_worker_request(input, "flow")?;
    request.validate()?;
    Ok(request)
}

pub fn read_image_import_worker_request(input: impl Read) -> Result<ImageImportWorkerRequest> {
    let request: ImageImportWorkerRequest = decode_worker_request(input, "image import")?;
    request.validate()?;
    Ok(request)
}

fn decode_worker_request<T: DeserializeOwned>(input: impl Read, kind: &str) -> Result<T> {
    serde_json::from_reader(BufReader::new(input))
        .with_context(|| format!("failed to decode {kind} worker request"))
}

fn validate_identity(value: &str, field: &str) -> Result<()> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        bail!("worker {field} is invalid");
    }
    Ok(())
}

pub(crate) fn validate_absolute_path(path: &Path, field: &str) -> Result<()> {
    if !path.is_absolute() {
        bail!("{field} must be absolute");
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    }) {
        bail!("{field} must be lexically normalized");
    }
    Ok(())
}

fn validate_memory(memory_mb: Option<u32>) -> Result<()> {
    let Some(memory_mb) = memory_mb else {
        return Ok(());
    };
    let memory_mb = u64::from(memory_mb);
    if !(qol_dev_env::resources::MIN_MEMORY_MB..=qol_dev_env::resources::MAX_MEMORY_MB)
        .contains(&memory_mb)
    {
        bail!(
            "flow memory must be from {} to {} MiB",
            qol_dev_env::resources::MIN_MEMORY_MB,
            qol_dev_env::resources::MAX_MEMORY_MB
        );
    }
    Ok(())
}

fn validate_cpus(cpus: Option<u16>) -> Result<()> {
    let Some(cpus) = cpus else {
        return Ok(());
    };
    let cpus = u64::from(cpus);
    if !(qol_dev_env::resources::MIN_CPUS..=qol_dev_env::resources::MAX_CPUS).contains(&cpus) {
        bail!(
            "flow CPUs must be from {} to {}",
            qol_dev_env::resources::MIN_CPUS,
            qol_dev_env::resources::MAX_CPUS
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn absolute(path: &str) -> PathBuf {
        std::env::temp_dir().join(path)
    }

    fn start(worktree: PathBuf) -> FlowStart {
        FlowStart {
            workflow: "qol-shot-capture-region".to_string(),
            environment_id: "linux/mint-cinnamon".to_string(),
            worktree,
            run_id: "flow-1".to_string(),
            repeat: 10,
            jobs: 10,
            memory_mb: Some(4096),
            cpus: Some(4),
            force: false,
        }
    }

    fn image_import(worktree: PathBuf) -> ImageImportStart {
        ImageImportStart {
            environment_id: "linux/mint-cinnamon".to_string(),
            source: absolute("qol/images/mint.qcow2"),
            worktree,
            run_id: "image-import-1".to_string(),
        }
    }

    fn flow_request(worktree: PathBuf) -> FlowWorkerRequest {
        FlowWorkerRequest {
            start: start(worktree),
            run_root: absolute("qol/runs"),
            plan_fingerprint: "a".repeat(64),
            verbose: false,
        }
    }

    #[test]
    fn validates_exact_worktree_and_resource_bounds() {
        let valid = start(absolute("qol/worktrees/shot"));
        assert!(valid.validate().is_ok());
        for invalid in [
            FlowStart {
                worktree: PathBuf::from("relative"),
                ..valid.clone()
            },
            FlowStart {
                run_id: "../escape".to_string(),
                ..valid.clone()
            },
            FlowStart {
                worktree: absolute("qol/worktrees").join("..").join("other"),
                ..valid.clone()
            },
            FlowStart {
                repeat: 0,
                ..valid.clone()
            },
            FlowStart {
                jobs: qol_dev_env::resources::MAX_CONCURRENT_LANES + 1,
                ..valid.clone()
            },
            FlowStart {
                memory_mb: Some(1),
                ..valid.clone()
            },
            FlowStart {
                cpus: Some(0),
                ..valid.clone()
            },
        ] {
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn typed_worker_input_rejects_unknown_fields() {
        let mut document =
            serde_json::to_value(flow_request(absolute("qol/worktrees/shot"))).unwrap();
        document["unexpected"] = json!(true);
        assert!(
            read_flow_worker_request(serde_json::to_vec(&document).unwrap().as_slice()).is_err()
        );
    }

    #[test]
    fn flow_worker_requires_an_exact_plan_identity() {
        let valid = flow_request(absolute("qol/worktrees/shot"));
        assert!(valid.validate().is_ok());
        for invalid in [
            FlowWorkerRequest {
                run_root: PathBuf::from("relative"),
                ..valid.clone()
            },
            FlowWorkerRequest {
                run_root: absolute("qol/runs").join("..").join("other"),
                ..valid.clone()
            },
            FlowWorkerRequest {
                plan_fingerprint: "A".repeat(64),
                ..valid.clone()
            },
            FlowWorkerRequest {
                plan_fingerprint: "a".repeat(63),
                ..valid
            },
        ] {
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn validates_image_import_identity_and_absolute_paths() {
        let valid = image_import(absolute("qol/worktrees/shot"));
        assert!(valid.validate().is_ok());
        for invalid in [
            ImageImportStart {
                environment_id: " linux/mint-cinnamon".to_string(),
                ..valid.clone()
            },
            ImageImportStart {
                source: PathBuf::from("relative.qcow2"),
                ..valid.clone()
            },
            ImageImportStart {
                worktree: PathBuf::from("relative"),
                ..valid.clone()
            },
            ImageImportStart {
                worktree: absolute("qol/worktrees").join("..").join("other"),
                ..valid.clone()
            },
            ImageImportStart {
                run_id: "../escape".to_string(),
                ..valid.clone()
            },
        ] {
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn typed_image_import_input_rejects_unknown_fields() {
        let request = ImageImportWorkerRequest {
            start: image_import(absolute("qol/worktrees/shot")),
            image_root: absolute("qol/image-root"),
            plan_fingerprint: "a".repeat(64),
            verbose: false,
        };
        assert_eq!(
            read_image_import_worker_request(serde_json::to_vec(&request).unwrap().as_slice())
                .unwrap(),
            request
        );
        let mut wrapper = serde_json::to_value(&request).unwrap();
        wrapper["unexpected"] = json!(true);
        assert!(
            read_image_import_worker_request(serde_json::to_vec(&wrapper).unwrap().as_slice())
                .is_err()
        );

        let mut nested = serde_json::to_value(request).unwrap();
        nested["start"]["unexpected"] = json!(true);
        assert!(
            read_image_import_worker_request(serde_json::to_vec(&nested).unwrap().as_slice())
                .is_err()
        );
    }

    #[test]
    fn image_import_ticket_uses_the_managed_registry_path() {
        let start = image_import(absolute("qol/worktrees/shot"));
        let image_root = absolute("qol/image-root");
        let ticket = start.ticket(&image_root).unwrap();
        assert_eq!(ticket.kind, ReportKind::ImageImport);
        assert_eq!(
            ticket.report_path,
            qol_dev_env::managed_verification_report_path(&image_root, &start.run_id).unwrap()
        );
    }

    #[test]
    fn image_import_worker_requires_an_exact_plan_identity() {
        let valid = ImageImportWorkerRequest {
            start: image_import(absolute("qol/worktrees/shot")),
            image_root: absolute("qol/image-root"),
            plan_fingerprint: "a".repeat(64),
            verbose: false,
        };
        assert!(valid.validate().is_ok());
        for invalid in [
            ImageImportWorkerRequest {
                image_root: PathBuf::from("relative"),
                ..valid.clone()
            },
            ImageImportWorkerRequest {
                plan_fingerprint: "A".repeat(64),
                ..valid.clone()
            },
            ImageImportWorkerRequest {
                plan_fingerprint: "a".repeat(63),
                ..valid
            },
        ] {
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }
    }
}
