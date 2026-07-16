use super::command as command_runner;
use crate::progress::{step_label, StepKind};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use qol_process::CancellationToken;
use serde::Serialize;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

#[derive(Serialize)]
pub(super) struct CheckReport {
    name: &'static str,
    started_at: String,
    finished_at: String,
    status: &'static str,
    error: Option<String>,
    inputs: CheckInputs,
    artifacts: CheckArtifacts,
    commands: Vec<CommandReport>,
    next: Vec<&'static str>,
}

#[derive(Serialize)]
struct CheckInputs {
    mode: &'static str,
    platform: &'static str,
    base_sha: Option<String>,
    head: String,
    source_head: Option<String>,
    index_tree: Option<String>,
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
            error: None,
            inputs: CheckInputs {
                mode,
                platform,
                base_sha: None,
                head: if mode == "staged" {
                    "INDEX".to_string()
                } else {
                    super::affected::WORKTREE_HEAD.to_string()
                },
                source_head: None,
                index_tree: None,
            },
            artifacts: CheckArtifacts {
                report: relative_path(root, report_path),
                affected_plan: None,
            },
            commands: Vec::new(),
            next: Vec::new(),
        }
    }

    pub(super) fn set_source_state(&mut self, source_head: &str, index_tree: &str) {
        self.inputs.source_head = Some(source_head.to_string());
        self.inputs.index_tree = Some(index_tree.to_string());
    }

    pub(super) fn set_head(&mut self, head: &str) {
        self.inputs.head = head.to_string();
    }

    pub(super) fn set_base_sha(&mut self, base_sha: Option<&str>) {
        self.inputs.base_sha = base_sha.map(str::to_string);
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
        let result = command_runner::run(command, cancellation, containment, verbose);
        self.commands.push(CommandReport {
            name,
            command: argv,
            status: command_status(&result),
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
            duration_ms: 0,
        });
    }

    pub(super) fn finish(&mut self, result: &Result<()>, cancelled: bool) {
        self.finished_at = Utc::now().to_rfc3339();
        self.status = if result.is_ok() { "pass" } else { "failed" };
        self.error = result.as_ref().err().map(|error| format!("{error:#}"));
        self.next = if result.is_ok() {
            Vec::new()
        } else if cancelled && self.inputs.mode == "staged" {
            vec!["The check was interrupted; rerun `qol check --staged`."]
        } else if cancelled {
            vec!["The check was interrupted; rerun `qol check`."]
        } else if self.inputs.mode == "staged" {
            vec!["Fix the failed step, then rerun `qol check --staged`."]
        } else {
            vec!["Fix the failed step, then rerun `qol check`."]
        };
    }

    pub(super) fn write(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)? + "\n";
        qol_fs::atomic_write_durable(path, content.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))
    }
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
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(report_path).unwrap()).unwrap();

        assert_eq!(value["status"], "failed");
        assert_eq!(value["error"], "index drift");
        assert_eq!(value["inputs"]["mode"], "staged");
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
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(report_path).unwrap()).unwrap();

        assert_eq!(value["status"], "failed");
        assert_eq!(value["artifacts"]["affected_plan"], "affected.json");
        assert_eq!(
            value["next"][0],
            "The check was interrupted; rerun `qol check --staged`."
        );
    }
}
