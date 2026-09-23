use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use qol_terminal_sessions::SpawnKey;

use super::super::agent_policy::AgentAssignment;
use super::super::spawn::SpawnLocks;

pub(super) const SCHEMA_VERSION: u32 = 1;

const REPORT_FILE: &str = "report.md";
const RECEIPT_FILE: &str = "receipt.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    Report,
    Receipt,
}

impl Stage {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Stage::Report => "report",
            Stage::Receipt => "receipt",
        }
    }
}

#[derive(Debug)]
pub(super) struct Published {
    pub(super) report: PathBuf,
    pub(super) receipt: PathBuf,
}

#[derive(Debug)]
pub(super) struct Failure {
    pub(super) stage: Stage,
    pub(super) report: Option<PathBuf>,
    pub(super) error: String,
}

impl Failure {
    fn before_report(error: impl std::fmt::Display) -> Self {
        Self {
            stage: Stage::Report,
            report: None,
            error: error.to_string(),
        }
    }

    fn after_report(report: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            stage: Stage::Receipt,
            report: Some(report.to_path_buf()),
            error: error.to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct Receipt {
    schema_version: u32,
    label: Option<String>,
    session: String,
    completion_marker: String,
    completed_at: String,
    markerless: bool,
    report: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_assignment: Option<AgentAssignment>,
}

fn identity_digest(session: &str, marker: &str) -> String {
    let digest = Sha256::digest(format!("{session}\u{0}{marker}").as_bytes());
    format!("{digest:x}")
}

pub(super) fn round_dir(trace_dir: &Path, session: &str, marker: &str) -> PathBuf {
    trace_dir
        .join("lanes")
        .join("rounds")
        .join(identity_digest(session, marker))
}

pub(super) fn report_path(trace_dir: &Path, session: &str, marker: &str) -> PathBuf {
    round_dir(trace_dir, session, marker).join(REPORT_FILE)
}

pub(super) fn receipt_path(trace_dir: &Path, session: &str, marker: &str) -> PathBuf {
    round_dir(trace_dir, session, marker).join(RECEIPT_FILE)
}

fn read_receipt(path: &Path) -> Option<Receipt> {
    let encoded = fs::read_to_string(path).ok()?;
    serde_json::from_str(&encoded).ok()
}

fn receipt_matches(receipt: &Receipt, session: &str, marker: &str, report: &Path) -> bool {
    receipt.schema_version == SCHEMA_VERSION
        && receipt.session == session
        && receipt.completion_marker == marker
        && receipt.report == report.display().to_string()
}

fn published_at(report: &Path) -> String {
    match fs::metadata(report).and_then(|metadata| metadata.modified()) {
        Ok(modified) => chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339(),
        Err(_) => chrono::Utc::now().to_rfc3339(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn publish(
    trace_dir: &Path,
    locks: &SpawnLocks,
    session: &str,
    marker: &str,
    label: Option<&str>,
    markerless: bool,
    report_bytes: &[u8],
    assignment: Option<&AgentAssignment>,
) -> std::result::Result<Published, Failure> {
    let report = report_path(trace_dir, session, marker);
    let receipt = receipt_path(trace_dir, session, marker);
    let key = SpawnKey::new(format!(
        "session-evidence-{}",
        identity_digest(session, marker)
    ))
    .map_err(Failure::before_report)?;
    let _guard = locks.acquire(&key).map_err(Failure::before_report)?;
    let round = report.parent().expect("report path always has a parent");
    fs::create_dir_all(round).map_err(Failure::before_report)?;
    match fs::read(&report) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            qol_fs::atomic_write_durable(&report, report_bytes).map_err(Failure::before_report)?;
        }
        Err(error) => return Err(Failure::before_report(error)),
    }
    let reusable = match read_receipt(&receipt) {
        Some(receipt) => receipt_matches(&receipt, session, marker, &report),
        None => false,
    };
    if !reusable {
        let body = Receipt {
            schema_version: SCHEMA_VERSION,
            label: label.map(str::to_owned),
            session: session.to_owned(),
            completion_marker: marker.to_owned(),
            completed_at: published_at(&report),
            markerless,
            report: report.display().to_string(),
            agent_assignment: assignment.cloned(),
        };
        let encoded = serde_json::to_string(&body)
            .context("failed to serialize the completion receipt")
            .map_err(|error| Failure::after_report(&report, error))?;
        qol_fs::atomic_write_durable(&receipt, encoded.as_bytes())
            .map_err(|error| Failure::after_report(&report, error))?;
    }
    Ok(Published { report, receipt })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locks(dir: &Path) -> SpawnLocks {
        SpawnLocks::with_dir(dir.join("locks"))
    }

    fn publish_body(
        trace: &Path,
        locks: &SpawnLocks,
        session: &str,
        marker: &str,
        label: Option<&str>,
        markerless: bool,
        body: &str,
    ) -> std::result::Result<Published, Failure> {
        publish(
            trace,
            locks,
            session,
            marker,
            label,
            markerless,
            body.as_bytes(),
            None,
        )
    }

    fn receipt_of(published: &Published) -> serde_json::Value {
        let encoded = fs::read_to_string(&published.receipt)
            .unwrap_or_else(|error| panic!("receipt unreadable: {error}"));
        serde_json::from_str(&encoded).unwrap()
    }

    #[test]
    fn round_paths_are_deterministic_and_identity_specific() {
        let trace = Path::new("/trace");
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_a";
        assert_eq!(
            report_path(trace, session, marker),
            report_path(trace, session, marker)
        );
        assert_ne!(
            report_path(trace, session, marker),
            report_path(trace, session, "QOL_BRIDGE_DONE_b")
        );
        assert_ne!(
            report_path(trace, session, marker),
            report_path(trace, "v1:pi:8:200", marker)
        );
        let name = round_dir(trace, session, marker)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(qol_fs::is_lowercase_sha256_digest(&name), "{name}");
        assert_eq!(
            receipt_path(trace, session, marker),
            round_dir(trace, session, marker).join(RECEIPT_FILE)
        );
    }

    #[test]
    fn a_receipt_records_the_round_identity_and_schema() {
        let dir = tempfile::TempDir::new().unwrap();
        let locks = locks(dir.path());
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_a";
        let published = publish_body(
            dir.path(),
            &locks,
            session,
            marker,
            Some("round-label"),
            true,
            "report body",
        )
        .unwrap();
        let receipt = receipt_of(&published);
        assert_eq!(receipt["schema_version"], 1);
        assert_eq!(receipt["session"], session);
        assert_eq!(receipt["completion_marker"], marker);
        assert_eq!(receipt["label"], "round-label");
        assert_eq!(receipt["markerless"], true);
        assert_eq!(receipt["report"], published.report.display().to_string());
        assert!(
            chrono::DateTime::parse_from_rfc3339(receipt["completed_at"].as_str().unwrap()).is_ok()
        );
        assert_eq!(
            fs::read_to_string(&published.report).unwrap(),
            "report body"
        );
    }

    #[test]
    fn two_markers_on_one_session_stay_distinct() {
        let dir = tempfile::TempDir::new().unwrap();
        let locks = locks(dir.path());
        let session = "v1:pi:7:100";
        let attempt = publish_body(
            dir.path(),
            &locks,
            session,
            "QOL_BRIDGE_DONE_a",
            None,
            false,
            "first",
        );
        let first = attempt.unwrap();
        let second = publish_body(
            dir.path(),
            &locks,
            session,
            "QOL_BRIDGE_DONE_b",
            Some("second-round"),
            false,
            "second",
        )
        .unwrap();
        assert_ne!(first.report, second.report);
        assert_ne!(first.receipt, second.receipt);
        assert_eq!(fs::read_to_string(&first.report).unwrap(), "first");
        assert_eq!(fs::read_to_string(&second.report).unwrap(), "second");
        assert_eq!(receipt_of(&first)["completion_marker"], "QOL_BRIDGE_DONE_a");
        assert_eq!(
            receipt_of(&second)["completion_marker"],
            "QOL_BRIDGE_DONE_b"
        );
        assert_eq!(fs::read_to_string(&first.report).unwrap(), "first");
    }

    #[test]
    fn replaying_a_round_with_different_text_keeps_the_first_publication() {
        let dir = tempfile::TempDir::new().unwrap();
        let locks = locks(dir.path());
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_a";
        let attempt = publish_body(dir.path(), &locks, session, marker, None, false, "first");
        let first = attempt.unwrap();
        let before = receipt_of(&first);
        let second = publish_body(
            dir.path(),
            &locks,
            session,
            marker,
            Some("relabeled"),
            true,
            "second",
        )
        .unwrap();
        assert_eq!(first.report, second.report);
        assert_eq!(first.receipt, second.receipt);
        assert_eq!(fs::read_to_string(&first.report).unwrap(), "first");
        assert_eq!(receipt_of(&second), before);
    }

    #[test]
    fn a_retry_reuses_the_persisted_report_and_keeps_the_completion_time() {
        let dir = tempfile::TempDir::new().unwrap();
        let locks = locks(dir.path());
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_a";
        let report = report_path(dir.path(), session, marker);
        fs::create_dir_all(report.parent().unwrap()).unwrap();
        fs::write(&report, "persisted report").unwrap();
        let attempt = publish_body(dir.path(), &locks, session, marker, None, false, "new");
        let published = attempt.unwrap();
        assert_eq!(
            fs::read_to_string(&published.report).unwrap(),
            "persisted report"
        );
        let first = receipt_of(&published);
        assert_eq!(first["completed_at"], published_at(&report));
        fs::remove_file(&published.receipt).unwrap();
        let retried = publish_body(
            dir.path(),
            &locks,
            session,
            marker,
            None,
            true,
            "another report",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(&retried.report).unwrap(),
            "persisted report"
        );
        let second = receipt_of(&retried);
        assert_eq!(second["completed_at"], first["completed_at"]);
        assert_eq!(second["markerless"], true);
    }

    #[test]
    fn an_invalid_existing_receipt_is_replaced() {
        let dir = tempfile::TempDir::new().unwrap();
        let locks = locks(dir.path());
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_a";
        let receipt = receipt_path(dir.path(), session, marker);
        fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        fs::write(&receipt, "{\"schema_version\":1,\"session\":\"other\"}").unwrap();
        let attempt = publish_body(dir.path(), &locks, session, marker, None, false, "report");
        let published = attempt.unwrap();
        let stored = receipt_of(&published);
        assert_eq!(stored["session"], session);
        assert_eq!(stored["completion_marker"], marker);
        assert_eq!(stored["report"], published.report.display().to_string());
        fs::write(
            &receipt,
            serde_json::json!({
                "schema_version": 1,
                "label": serde_json::Value::Null,
                "session": session,
                "completion_marker": marker,
                "completed_at": "2000-01-01T00:00:00+00:00",
                "markerless": false,
                "report": "/elsewhere/report.md",
            })
            .to_string(),
        )
        .unwrap();
        let attempt = publish_body(dir.path(), &locks, session, marker, None, false, "report");
        let republished = attempt.unwrap();
        assert_eq!(
            receipt_of(&republished)["report"],
            republished.report.display().to_string()
        );
    }

    #[test]
    fn a_report_stage_failure_publishes_no_receipt() {
        let dir = tempfile::TempDir::new().unwrap();
        let locks = locks(dir.path());
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_a";
        fs::create_dir_all(report_path(dir.path(), session, marker)).unwrap();
        let attempt = publish_body(dir.path(), &locks, session, marker, None, false, "report");
        let failure = attempt.unwrap_err();
        assert_eq!(failure.stage, Stage::Report);
        assert!(failure.report.is_none());
        assert!(!receipt_path(dir.path(), session, marker).exists());
    }

    #[test]
    fn a_receipt_carries_the_recorded_agent_assignment_and_reads_old_files() {
        use super::super::super::agent_policy::{AgentRole, ImageInput, VisualReview};

        let dir = tempfile::TempDir::new().unwrap();
        let locks = locks(dir.path());
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_a";
        let assignment = AgentAssignment {
            profile: "worker".to_owned(),
            tool: "pi".to_owned(),
            model: "flash".to_owned(),
            provider: None,
            roles: vec![AgentRole::Implement],
            image_input: ImageInput::Unknown,
            visual_review: VisualReview::Deny,
            task_role: AgentRole::Implement,
            requires: Vec::new(),
            evidence_basis: "configuration_declared".to_owned(),
        };
        let published = publish(
            dir.path(),
            &locks,
            session,
            marker,
            None,
            false,
            b"report body",
            Some(&assignment),
        )
        .unwrap();
        let receipt = receipt_of(&published);
        assert_eq!(receipt["agent_assignment"]["profile"], "worker");
        assert_eq!(receipt["agent_assignment"]["model"], "flash");
        assert_eq!(
            receipt["agent_assignment"]["evidence_basis"],
            "configuration_declared"
        );

        let legacy = serde_json::json!({
            "schema_version": 1,
            "label": serde_json::Value::Null,
            "session": session,
            "completion_marker": marker,
            "completed_at": "2000-01-01T00:00:00+00:00",
            "markerless": false,
            "report": published.report.display().to_string(),
        });
        let parsed = serde_json::from_str::<Receipt>(&legacy.to_string()).unwrap();
        assert!(parsed.agent_assignment.is_none());
        assert!(receipt_matches(&parsed, session, marker, &published.report));
    }

    #[test]
    fn a_receipt_stage_failure_names_the_published_report() {
        let dir = tempfile::TempDir::new().unwrap();
        let locks = locks(dir.path());
        let session = "v1:pi:7:100";
        let marker = "QOL_BRIDGE_DONE_a";
        fs::create_dir_all(receipt_path(dir.path(), session, marker)).unwrap();
        let attempt = publish_body(dir.path(), &locks, session, marker, None, false, "report");
        let failure = attempt.unwrap_err();
        assert_eq!(failure.stage, Stage::Receipt);
        assert_eq!(
            failure.report.as_deref(),
            Some(report_path(dir.path(), session, marker).as_path())
        );
        assert!(!failure.error.is_empty());
        assert_eq!(
            fs::read_to_string(report_path(dir.path(), session, marker)).unwrap(),
            "report"
        );
    }
}
