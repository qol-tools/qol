use crate::progress::{run_step, step_label, StepKind};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

#[derive(Serialize)]
pub(super) struct CheckReport {
    name: &'static str,
    started_at: String,
    finished_at: String,
    status: &'static str,
    inputs: CheckInputs,
    artifacts: CheckArtifacts,
    commands: Vec<CommandReport>,
    next: Vec<&'static str>,
}

#[derive(Serialize)]
struct CheckInputs {
    platform: &'static str,
    base_sha: Option<String>,
    head: &'static str,
}

#[derive(Serialize)]
struct CheckArtifacts {
    report: String,
    affected_plan: String,
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
        platform: &'static str,
        base_sha: Option<String>,
        report_path: &Path,
        affected_path: &Path,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            name: "qol-check",
            started_at: started_at.to_rfc3339(),
            finished_at: String::new(),
            status: "failed",
            inputs: CheckInputs {
                platform,
                base_sha,
                head: super::affected::WORKTREE_HEAD,
            },
            artifacts: CheckArtifacts {
                report: relative_path(root, report_path),
                affected_plan: relative_path(root, affected_path),
            },
            commands: Vec::new(),
            next: Vec::new(),
        }
    }

    pub(super) fn run(
        &mut self,
        name: &'static str,
        verb: &str,
        target: &str,
        command: &mut Command,
        verbose: bool,
    ) -> Result<()> {
        let argv = command_argv(command);
        let started = Instant::now();
        let result = run_step(verb, StepKind::Pending, target, command, verbose);
        self.commands.push(CommandReport {
            name,
            command: argv,
            status: if result.is_ok() { "pass" } else { "failed" },
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

    pub(super) fn finish(&mut self, passed: bool) {
        self.finished_at = Utc::now().to_rfc3339();
        self.status = if passed { "pass" } else { "failed" };
        self.next = if passed {
            Vec::new()
        } else {
            vec!["Fix the failed step, then rerun `qol check`."]
        };
    }

    pub(super) fn write(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)? + "\n";
        fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
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
