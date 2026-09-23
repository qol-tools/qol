use super::command as command_runner;
use super::snapshot::Materialization;
use crate::progress::{step_label, StepKind};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use qol_process::CancellationToken;
use serde::Serialize;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Serialize)]
pub(super) struct CheckReport {
    name: &'static str,
    started_at: String,
    finished_at: String,
    status: &'static str,
    state: &'static str,
    error: Option<String>,
    inputs: CheckInputs,
    fingerprint: FingerprintReport,
    formatting: FormattingReport,
    publication: PublicationReport,
    artifacts: CheckArtifacts,
    snapshot: Option<SnapshotReport>,
    commands: Vec<CommandReport>,
    next: Vec<&'static str>,
    #[serde(skip)]
    stale: bool,
}

#[derive(Serialize)]
struct SnapshotReport {
    materialization: Materialization,
    duration_ms: u64,
}

#[derive(Serialize)]
struct CheckInputs {
    mode: &'static str,
    platform: &'static str,
    verification: &'static str,
    requested_base: Option<String>,
    base_sha: Option<String>,
    head: String,
    source_head: Option<String>,
    index_tree: Option<String>,
}

#[derive(Serialize)]
struct FingerprintReport {
    algorithm: &'static str,
    status: &'static str,
    before: Option<String>,
    after: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct FormattingReport {
    status: &'static str,
    requested: Vec<String>,
    files: Vec<FormattedFile>,
    failed_file: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct PublicationReport {
    requested: Option<String>,
    status: &'static str,
    published: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct FormattedFile {
    pub(super) path: String,
    pub(super) before_sha256: String,
    pub(super) after_sha256: String,
    pub(super) changed: bool,
}

#[derive(Serialize)]
struct CheckArtifacts {
    report: String,
    affected_plan: Option<String>,
}

#[derive(Serialize)]
struct CommandReport {
    name: &'static str,
    command: Vec<String>,
    status: &'static str,
    exit_code: Option<i32>,
    duration_ms: u64,
}

impl CheckReport {
    pub(super) fn new(
        root: &Path,
        mode: &'static str,
        platform: &'static str,
        report_path: &Path,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            name: "qol-check",
            started_at: started_at.to_rfc3339(),
            finished_at: String::new(),
            status: "failed",
            state: "pending",
            error: None,
            inputs: CheckInputs {
                mode,
                platform,
                verification: verification_scope(mode),
                requested_base: None,
                base_sha: None,
                head: if mode == "staged" {
                    "INDEX".to_string()
                } else {
                    super::affected::WORKTREE_HEAD.to_string()
                },
                source_head: None,
                index_tree: None,
            },
            fingerprint: FingerprintReport {
                algorithm: "git-delta-sha256",
                status: "not-captured",
                before: None,
                after: None,
                error: None,
            },
            formatting: FormattingReport {
                status: "not-requested",
                requested: Vec::new(),
                files: Vec::new(),
                failed_file: None,
                error: None,
            },
            publication: PublicationReport {
                requested: None,
                status: "not-requested",
                published: None,
                error: None,
            },
            artifacts: CheckArtifacts {
                report: relative_path(root, report_path),
                affected_plan: None,
            },
            snapshot: None,
            commands: Vec::new(),
            next: Vec::new(),
            stale: false,
        }
    }

    pub(super) fn set_source_state(&mut self, source_head: &str, index_tree: &str) {
        self.inputs.source_head = Some(source_head.to_string());
        self.inputs.index_tree = Some(index_tree.to_string());
    }

    pub(super) fn set_head(&mut self, head: &str) {
        self.inputs.head = head.to_string();
    }

    pub(super) fn set_snapshot(&mut self, materialization: Materialization, duration: Duration) {
        self.snapshot = Some(SnapshotReport {
            materialization,
            duration_ms: duration.as_millis().min(u128::from(u64::MAX)) as u64,
        });
    }

    pub(super) fn set_requested_base(&mut self, base: Option<&str>) {
        self.inputs.requested_base = base.map(str::to_string);
    }

    pub(super) fn set_base_sha(&mut self, base_sha: Option<&str>) {
        self.inputs.base_sha = base_sha.map(str::to_string);
    }

    pub(super) fn set_fingerprint_before(&mut self, digest: &str) {
        self.fingerprint.before = Some(digest.to_string());
        self.fingerprint.status = "captured";
    }

    pub(super) fn set_fingerprint_after(&mut self, digest: &str) {
        self.fingerprint.after = Some(digest.to_string());
        self.fingerprint.status = "match";
    }

    pub(super) fn set_fingerprint_error(&mut self, error: &str) {
        self.fingerprint.error = Some(error.to_string());
        self.fingerprint.status = "unavailable";
    }

    pub(super) fn mark_stale(&mut self) {
        self.stale = true;
        self.fingerprint.status = "stale";
    }

    pub(super) fn set_formatting_requested(&mut self, requested: Vec<String>) {
        self.formatting.requested = requested;
        self.formatting.status = "pending";
    }

    pub(super) fn set_formatting_outcome(
        &mut self,
        files: Vec<FormattedFile>,
        failed_file: Option<String>,
        error: Option<String>,
    ) {
        self.formatting.files = files;
        self.formatting.failed_file = failed_file;
        self.formatting.status = if error.is_some() { "failed" } else { "pass" };
        self.formatting.error = error;
    }

    pub(super) fn set_publication_pending(&mut self, root: &Path, path: &Path) {
        self.publication.requested = Some(relative_path(root, path));
        self.publication.status = "pending";
        self.publication.published = None;
        self.publication.error = None;
    }

    pub(super) fn set_publication_published(&mut self) {
        self.publication.status = "published";
        self.publication
            .published
            .clone_from(&self.publication.requested);
    }

    pub(super) fn set_publication_failure(&mut self, error: &str) {
        self.publication.status = "failed";
        self.publication.published = None;
        self.publication.error = Some(error.to_string());
    }

    pub(super) fn set_affected_plan(&mut self, root: &Path, path: &Path) {
        if path.is_file() {
            self.artifacts.affected_plan = Some(relative_path(root, path));
        }
    }

    pub(super) fn run(
        &mut self,
        name: &'static str,
        label: (&str, &str),
        command: &mut Command,
        cancellation: &CancellationToken,
        containment: command_runner::Containment,
        verbose: bool,
    ) -> Result<()> {
        let (verb, target) = label;
        if !verbose {
            step_label(verb, StepKind::Pending, target);
        }
        let argv = command_argv(command);
        let started = Instant::now();
        let (exit_code, result) = command_runner::run(command, cancellation, containment, verbose);
        self.commands.push(CommandReport {
            name,
            command: argv,
            status: command_status(&result),
            exit_code,
            duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        });
        result.with_context(|| format!("{name} failed"))
    }

    pub(super) fn skip(&mut self, name: &'static str, reason: &str) {
        step_label("skip", StepKind::Info, &format!("{name}: {reason}"));
        self.commands.push(CommandReport {
            name,
            command: Vec::new(),
            status: "skipped",
            exit_code: None,
            duration_ms: 0,
        });
    }

    pub(super) fn finish(&mut self, result: &Result<()>, cancelled: bool) {
        self.finished_at = Utc::now().to_rfc3339();
        let publication_failed = self.publication.status == "failed";
        let publication_pending = self.publication.status == "pending";
        let publication_open = publication_failed || publication_pending;
        let quiet_run = !(cancelled || self.stale);
        let provisional = publication_pending && result.is_ok() && quiet_run;
        self.status = if provisional {
            "pending"
        } else if result.is_ok() && !publication_failed {
            "pass"
        } else {
            "failed"
        };
        self.state = if cancelled {
            "cancelled"
        } else if self.stale {
            "stale"
        } else if publication_failed {
            "failed"
        } else if provisional {
            "pending"
        } else if result.is_ok() {
            "verified"
        } else if self.fingerprint.status == "unavailable" {
            "error"
        } else {
            "failed"
        };
        let run_error = result.as_ref().err().map(|error| format!("{error:#}"));
        self.error = match (&run_error, &self.publication.error) {
            (Some(run), Some(publication)) => {
                Some(format!("{run}\npublication failed: {publication}"))
            }
            (Some(run), None) => Some(run.clone()),
            (None, Some(publication)) => Some(format!("publication failed: {publication}")),
            (None, None) => None,
        };
        self.next = if result.is_ok() && !publication_open {
            Vec::new()
        } else if cancelled && self.inputs.mode == "staged" {
            vec!["The check was interrupted; rerun `qol check --staged`."]
        } else if cancelled {
            vec!["The check was interrupted; rerun `qol check`."]
        } else if self.stale {
            vec!["The source changed while qol check was running; rerun it against a quiet tree."]
        } else if publication_failed {
            vec!["The report could not be published; fix the destination and rerun the same command."]
        } else if provisional {
            vec!["Publication did not complete; rerun the same command to republish the report."]
        } else if self.inputs.mode == "staged" {
            vec!["Fix the failed step, then rerun `qol check --staged`."]
        } else {
            vec!["Fix the failed step, then rerun `qol check`."]
        };
    }

    pub(super) fn render(&self) -> Result<Vec<u8>> {
        Ok((serde_json::to_string_pretty(self)? + "\n").into_bytes())
    }

    pub(super) fn write(&self, path: &Path) -> Result<()> {
        let content = self.render()?;
        publish_bytes(&content, path)
    }
}

fn verification_scope(mode: &str) -> &'static str {
    match mode {
        "worktree" => "full",
        "staged" => "staged",
        "lint" => "lint-only",
        _ => "validation",
    }
}

pub(super) fn publish_bytes(content: &[u8], path: &Path) -> Result<()> {
    qol_fs::atomic_write_durable(path, content)
        .with_context(|| format!("failed to write report to {}", path.display()))
}

fn command_status(result: &Result<()>) -> &'static str {
    if result.is_ok() {
        "pass"
    } else {
        "failed"
    }
}

pub(super) fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn command_argv(command: &Command) -> Vec<String> {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(OsStr::to_string_lossy)
        .map(|arg| arg.into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::fs;

    fn report(mode: &'static str, root: &Path) -> CheckReport {
        CheckReport::new(root, mode, "linux", &root.join("report.json"), Utc::now())
    }

    fn read(path: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn failed_staged_report_records_exact_source_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let mut report = CheckReport::new(
            directory.path(),
            "staged",
            "linux",
            &report_path,
            Utc::now(),
        );
        report.set_source_state("source-head", "index-tree");
        report.set_head("snapshot-commit");
        report.set_base_sha(Some("base"));
        report.finish(&Err(anyhow!("index drift")), false);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["status"], "failed");
        assert_eq!(value["state"], "failed");
        assert_eq!(value["error"], "index drift");
        assert_eq!(value["inputs"]["mode"], "staged");
        assert_eq!(value["inputs"]["verification"], "staged");
        assert_eq!(value["inputs"]["source_head"], "source-head");
        assert_eq!(value["inputs"]["index_tree"], "index-tree");
        assert_eq!(value["inputs"]["head"], "snapshot-commit");
        assert_eq!(value["artifacts"]["affected_plan"], serde_json::Value::Null);
    }

    #[test]
    fn report_only_records_an_affected_plan_after_it_exists() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let affected_path = directory.path().join("affected.json");
        let mut report = CheckReport::new(
            directory.path(),
            "staged",
            "linux",
            &report_path,
            Utc::now(),
        );

        report.set_affected_plan(directory.path(), &affected_path);
        fs::write(&affected_path, "{}\n").unwrap();
        report.set_affected_plan(directory.path(), &affected_path);
        report.finish(&Err(anyhow!("cancelled")), true);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["status"], "failed");
        assert_eq!(value["state"], "cancelled");
        assert_eq!(value["artifacts"]["affected_plan"], "affected.json");
        assert_eq!(
            value["next"][0],
            "The check was interrupted; rerun `qol check --staged`."
        );
    }

    #[test]
    fn lint_reports_never_claim_full_verification() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let mut report = report("lint", directory.path());
        report.finish(&Ok(()), false);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["status"], "pass");
        assert_eq!(value["state"], "verified");
        assert_eq!(value["inputs"]["verification"], "lint-only");
    }

    #[test]
    fn full_reports_capture_fingerprints_formatting_and_publication() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let published = directory.path().join("verification/report.json");
        let mut report = report("worktree", directory.path());
        report.set_requested_base(Some("origin/main"));
        report.set_base_sha(Some("resolved-base"));
        report.set_publication_pending(directory.path(), &published);
        report.set_publication_published();
        report.set_fingerprint_before("before-digest");
        report.set_fingerprint_after("before-digest");
        report.set_formatting_requested(vec!["src/lib.rs".to_string()]);
        report.set_formatting_outcome(
            vec![FormattedFile {
                path: "src/lib.rs".to_string(),
                before_sha256: "before-hash".to_string(),
                after_sha256: "after-hash".to_string(),
                changed: true,
            }],
            None,
            None,
        );
        report.finish(&Ok(()), false);
        report.write(&report_path).unwrap();
        publish_bytes(&report.render().unwrap(), &published).unwrap();
        let value = read(&report_path);

        assert_eq!(value["inputs"]["verification"], "full");
        assert_eq!(value["inputs"]["requested_base"], "origin/main");
        assert_eq!(value["inputs"]["base_sha"], "resolved-base");
        assert_eq!(value["fingerprint"]["status"], "match");
        assert_eq!(value["fingerprint"]["before"], "before-digest");
        assert_eq!(value["fingerprint"]["after"], "before-digest");
        assert_eq!(value["formatting"]["status"], "pass");
        assert!(value["formatting"]["files"][0]["changed"]
            .as_bool()
            .unwrap());
        assert_eq!(value["publication"]["status"], "published");
        assert_eq!(
            value["publication"]["published"],
            "verification/report.json"
        );
        assert_eq!(
            fs::read(&published).unwrap(),
            fs::read(&report_path).unwrap()
        );
    }

    #[test]
    fn failed_formatting_keeps_completed_file_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let mut report = report("worktree", directory.path());
        report.set_formatting_requested(vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
        report.set_formatting_outcome(
            vec![FormattedFile {
                path: "src/a.rs".to_string(),
                before_sha256: "before-hash".to_string(),
                after_sha256: "after-hash".to_string(),
                changed: true,
            }],
            Some("src/b.rs".to_string()),
            Some("rustfmt failed for src/b.rs".to_string()),
        );
        report.finish(&Err(anyhow!("rustfmt failed for src/b.rs")), false);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["formatting"]["status"], "failed");
        assert_eq!(value["formatting"]["failed_file"], "src/b.rs");
        assert_eq!(value["formatting"]["files"][0]["path"], "src/a.rs");
        assert_eq!(value["state"], "failed");
    }

    #[test]
    fn publication_pending_is_not_a_success_claim() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let destination = directory.path().join("verification/report.json");
        let mut report = report("worktree", directory.path());
        report.set_fingerprint_before("before-digest");
        report.set_fingerprint_after("before-digest");
        report.set_publication_pending(directory.path(), &destination);
        report.finish(&Ok(()), false);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["status"], "pending");
        assert_eq!(value["state"], "pending");
        assert_ne!(value["state"], "verified");
        assert_eq!(value["publication"]["status"], "pending");
        assert!(value["publication"]["published"].is_null());
        assert!(value["next"][0]
            .as_str()
            .unwrap()
            .contains("Publication did not complete"));
    }

    #[test]
    fn publication_failure_downgrades_a_verified_report() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let mut report = report("worktree", directory.path());
        report.set_publication_failure("injected publication failure");
        report.finish(&Ok(()), false);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["status"], "failed");
        assert_ne!(value["state"], "verified");
        assert_eq!(value["publication"]["status"], "failed");
        assert!(value["publication"]["published"].is_null());
        assert!(value["error"]
            .as_str()
            .unwrap()
            .contains("injected publication failure"));
        assert!(value["next"][0]
            .as_str()
            .unwrap()
            .contains("could not be published"));
    }

    #[test]
    fn stale_reports_never_pass_and_name_the_rerun() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let mut report = report("worktree", directory.path());
        report.set_fingerprint_before("before-digest");
        report.set_fingerprint_after("after-digest");
        report.mark_stale();
        report.finish(&Err(anyhow!("source changed")), false);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["status"], "failed");
        assert_eq!(value["state"], "stale");
        assert_eq!(value["fingerprint"]["status"], "stale");
        assert_eq!(value["fingerprint"]["before"], "before-digest");
        assert_eq!(value["fingerprint"]["after"], "after-digest");
        assert!(value["next"][0]
            .as_str()
            .unwrap()
            .contains("source changed"));
    }

    #[test]
    fn failure_reports_publish_the_same_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let published = directory.path().join("verification/report.json");
        let mut report = report("worktree", directory.path());
        report.set_fingerprint_before("before-digest");
        report.set_fingerprint_after("after-digest");
        report.mark_stale();
        report.finish(&Err(anyhow!("source changed")), false);
        report.write(&report_path).unwrap();
        publish_bytes(&report.render().unwrap(), &published).unwrap();

        assert_eq!(
            fs::read(&published).unwrap(),
            fs::read(&report_path).unwrap()
        );
        let value = read(&published);
        assert_eq!(value["state"], "stale");
    }

    #[test]
    fn unavailable_fingerprints_are_explicit_failures() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let mut report = report("worktree", directory.path());
        report.set_fingerprint_error("index has unresolved merge conflicts");
        report.finish(&Err(anyhow!("source identity unavailable")), false);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["status"], "failed");
        assert_eq!(value["state"], "error");
        assert_eq!(value["fingerprint"]["status"], "unavailable");
        assert_eq!(
            value["fingerprint"]["error"],
            "index has unresolved merge conflicts"
        );
    }

    #[test]
    fn validation_failures_write_a_machine_report() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("nested/report.json");
        let mut report = CheckReport::new(
            directory.path(),
            "validation",
            "linux",
            &destination,
            Utc::now(),
        );
        report.finish(&Err(anyhow!("usage: qol check")), false);
        report.write(&destination).unwrap();
        let value = read(&destination);

        assert_eq!(value["inputs"]["mode"], "validation");
        assert_eq!(value["inputs"]["verification"], "validation");
        assert_eq!(value["state"], "failed");
        assert!(value["error"]
            .as_str()
            .unwrap()
            .contains("usage: qol check"));
    }

    #[cfg(unix)]
    #[test]
    fn command_reports_record_argv_and_exit_code() {
        let directory = tempfile::tempdir().unwrap();
        let report_path = directory.path().join("report.json");
        let mut report = report("worktree", directory.path());
        let cancellation = CancellationToken::new();
        let mut command = Command::new("sh");
        command.args(["-c", "exit 5"]);
        let error = report
            .run(
                "sample",
                ("step", "target"),
                &mut command,
                &cancellation,
                command_runner::Containment::Preferred,
                false,
            )
            .unwrap_err();
        report.finish(&Err(error), false);
        report.write(&report_path).unwrap();
        let value = read(&report_path);

        assert_eq!(value["commands"][0]["name"], "sample");
        assert_eq!(value["commands"][0]["command"][0], "sh");
        assert_eq!(value["commands"][0]["status"], "failed");
        assert_eq!(value["commands"][0]["exit_code"], 5);
    }
}
