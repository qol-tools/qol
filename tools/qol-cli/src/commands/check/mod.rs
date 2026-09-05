mod affected;
mod command;
mod report;
mod snapshot;
mod testing;

use self::affected::{CargoPlan, Platform};
use self::report::{relative_path, CheckReport};
use self::snapshot::{SourceState, StagedSnapshot};
use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use qol_process::CancellationToken;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let mode = CheckMode::parse(args)?;
    let source_root = repo_root()?;
    let platform = Platform::current()?;
    let started_at = Utc::now();
    let run_dir = source_root.join("target").join("qol-check").join(format!(
        "{}-{}",
        started_at.timestamp_millis(),
        std::process::id()
    ));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;
    let report_path = run_dir.join("report.json");
    let affected_path = run_dir.join("affected.json");
    let mut report = CheckReport::new(
        &source_root,
        mode.name(),
        platform.name(),
        &report_path,
        started_at,
    );

    print_title("qol check");
    print_hint(verbose);
    run_and_report(
        mode,
        &source_root,
        platform,
        &affected_path,
        &report_path,
        &mut report,
        verbose,
    )
}

fn run_and_report(
    mode: CheckMode,
    source_root: &Path,
    platform: Platform,
    affected_path: &Path,
    report_path: &Path,
    report: &mut CheckReport,
    verbose: bool,
) -> Result<()> {
    let cancellation = CancellationToken::install();
    let mut result = match &cancellation {
        Ok(cancellation) => execute(
            mode,
            source_root,
            platform,
            affected_path,
            report,
            cancellation,
            verbose,
        ),
        Err(error) => Err(anyhow::anyhow!(error.to_string()))
            .context("failed to install check cancellation handler"),
    };
    let cancelled = cancellation
        .as_ref()
        .is_ok_and(|cancellation| cancellation.is_cancelled());
    if cancelled && result.is_ok() {
        result = Err(anyhow::anyhow!("check cancelled"));
    }
    report.set_affected_plan(source_root, affected_path);
    report.finish(&result, cancelled);
    let write_result = report.write(report_path);
    if write_result.is_ok() {
        step_label(
            "report",
            StepKind::Info,
            &relative_path(source_root, report_path),
        );
    }
    combine_results([result, write_result])?;
    step_label("done", StepKind::Success, "all checks passed");
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckMode {
    Worktree,
    Staged,
    Lint,
}

impl CheckMode {
    fn parse(args: &[OsString]) -> Result<Self> {
        match args {
            [] => Ok(Self::Worktree),
            [argument] if argument == "--staged" => Ok(Self::Staged),
            [argument] if argument == "--lint" => Ok(Self::Lint),
            _ => bail!("usage: qol check [--staged|--lint]"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Staged => "staged",
            Self::Lint => "lint",
        }
    }
}

fn execute(
    mode: CheckMode,
    source_root: &Path,
    platform: Platform,
    affected_path: &Path,
    report: &mut CheckReport,
    cancellation: &CancellationToken,
    verbose: bool,
) -> Result<()> {
    let source_state = SourceState::capture(source_root)?;
    report.set_source_state(&source_state.head, &source_state.index_tree);
    let base_sha = match mode {
        CheckMode::Lint => Some("HEAD".to_string()),
        _ => affected::comparison_base(source_root, &source_state.head),
    };
    report.set_base_sha(base_sha.as_deref());
    let execution = CheckExecution {
        source_root,
        platform,
        base_sha: base_sha.as_deref(),
        affected_path,
        cancellation,
        verbose,
    };
    match mode {
        CheckMode::Worktree => run_checks(
            &execution.context(
                source_root,
                source_root.join("target"),
                affected::WORKTREE_HEAD,
                command::Containment::Preferred,
                false,
            ),
            report,
        ),
        CheckMode::Staged => run_staged_checks(&execution, source_state, report),
        CheckMode::Lint => run_lint_checks(&execution.lint_context(source_root), report),
    }
}

struct CheckExecution<'a> {
    source_root: &'a Path,
    platform: Platform,
    base_sha: Option<&'a str>,
    affected_path: &'a Path,
    cancellation: &'a CancellationToken,
    verbose: bool,
}

impl CheckExecution<'_> {
    fn context<'a>(
        &'a self,
        root: &'a Path,
        cargo_target: PathBuf,
        head: &'a str,
        containment: command::Containment,
        sanitize_git: bool,
    ) -> CheckContext<'a> {
        CheckContext {
            root,
            cargo_target,
            platform: self.platform,
            base_sha: self.base_sha,
            head,
            affected_path: self.affected_path,
            cancellation: self.cancellation,
            containment,
            sanitize_git,
            verbose: self.verbose,
        }
    }

    fn lint_context<'a>(&'a self, root: &'a Path) -> CheckContext<'a> {
        CheckContext {
            root,
            cargo_target: root.join("target").join("qol-lint"),
            platform: self.platform,
            base_sha: Some("HEAD"),
            head: affected::WORKTREE_HEAD,
            affected_path: self.affected_path,
            cancellation: self.cancellation,
            containment: command::Containment::Preferred,
            sanitize_git: false,
            verbose: self.verbose,
        }
    }
}

fn run_staged_checks(
    execution: &CheckExecution<'_>,
    source_state: SourceState,
    report: &mut CheckReport,
) -> Result<()> {
    step_label("snapshot", StepKind::Pending, "staged index");
    let started = Instant::now();
    let mut snapshot = StagedSnapshot::materialize(execution.source_root, source_state)?;
    report.set_snapshot(snapshot.materialization(), started.elapsed());
    report.set_head(snapshot.commit());
    let context = execution.context(
        snapshot.root(),
        snapshot.cargo_target().to_path_buf(),
        snapshot.commit(),
        command::Containment::Required,
        true,
    );
    let checks = run_checks(&context, report);
    let snapshot_unchanged = snapshot.verify_snapshot();
    let unchanged = snapshot.verify_source_unchanged();
    let cleanup = if snapshot_unchanged.is_ok() {
        snapshot.retain()
    } else {
        snapshot.cleanup()
    };
    combine_results([checks, snapshot_unchanged, unchanged, cleanup])
}

struct CheckContext<'a> {
    root: &'a Path,
    cargo_target: PathBuf,
    platform: Platform,
    base_sha: Option<&'a str>,
    head: &'a str,
    affected_path: &'a Path,
    cancellation: &'a CancellationToken,
    containment: command::Containment,
    sanitize_git: bool,
    verbose: bool,
}

impl CheckContext<'_> {
    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.current_dir(self.root);
        command
    }

    fn prepare(&self, command: &mut Command) {
        if self.sanitize_git {
            snapshot::sanitize_git_environment(command);
        }
    }

    fn run(
        &self,
        report: &mut CheckReport,
        name: &'static str,
        verb: &str,
        target: &str,
        command: &mut Command,
    ) -> Result<()> {
        self.prepare(command);
        report.run(
            name,
            (verb, target),
            command,
            self.cancellation,
            self.containment,
            self.verbose,
        )
    }
}

fn run_checks(context: &CheckContext<'_>, report: &mut CheckReport) -> Result<()> {
    let mut guard = context.command("node");
    guard.arg(".githooks/single-source-guard.mjs");
    context.run(
        report,
        "single-source-guard",
        "guard",
        "single source",
        &mut guard,
    )?;
    run_ui_tests(context, report)?;
    let mut scripts = context.command("python3");
    scripts
        .args(["-m", "unittest", "discover", "-s", ".github/scripts/tests"])
        .args(["-p", "test_*.py"]);
    context.run(
        report,
        "release-script-tests",
        "scripts",
        "release tests",
        &mut scripts,
    )?;
    let mut planner = affected::planner_command(
        context.root,
        context.base_sha,
        context.head,
        context.affected_path,
    );
    context.run(
        report,
        "affected-crates",
        "plan",
        "affected crates",
        &mut planner,
    )?;
    let mut format = context.command("cargo");
    format.args(["fmt", "--all", "--", "--check"]);
    context.run(report, "rustfmt", "format", "workspace", &mut format)?;
    let cargo = affected::load_plan(context.affected_path, context.platform)?;
    run_rust_checks(context, cargo, report)
}

fn run_ui_tests(context: &CheckContext<'_>, report: &mut CheckReport) -> Result<()> {
    let ui_root = context.root.join("apps").join("qol-tray").join("ui");
    let tests = discover_ui_tests(&ui_root)?;
    if tests.is_empty() {
        bail!("no QoL Tray UI tests found");
    }
    let mut command = context.command("node");
    command.current_dir(&ui_root).arg("--test");
    for test in tests {
        command.arg(test);
    }
    context.run(report, "ui-tests", "ui", "QoL Tray", &mut command)
}

fn run_rust_checks(
    context: &CheckContext<'_>,
    cargo: CargoPlan,
    report: &mut CheckReport,
) -> Result<()> {
    if cargo.skip {
        report.skip("rust-build", "no affected crates");
        report.skip("clippy", "no affected crates");
        report.skip("rust-tests", "no affected crates");
        return Ok(());
    }

    let mut build = cargo_command(context, &["build"], &cargo.clippy_args);
    context.run(report, "rust-build", "build", "affected crates", &mut build)?;

    let mut clippy = cargo_command(context, &["clippy"], &cargo.clippy_args);
    clippy.args(["--", "-D", "warnings"]);
    context.run(report, "clippy", "clippy", "affected crates", &mut clippy)?;

    testing::run(context, &cargo.test_args, cargo.doctest, report)
}

fn run_lint_checks(context: &CheckContext<'_>, report: &mut CheckReport) -> Result<()> {
    let mut planner = affected::planner_command(
        context.root,
        context.base_sha,
        context.head,
        context.affected_path,
    );
    context.run(
        report,
        "affected-crates",
        "plan",
        "affected crates",
        &mut planner,
    )?;
    let cargo = affected::load_plan(context.affected_path, context.platform)?;
    if cargo.skip {
        report.skip("clippy", "no affected crates");
        return Ok(());
    }
    let mut clippy = cargo_command(context, &["clippy"], &cargo.clippy_args);
    clippy.args(["--", "-D", "warnings"]);
    context.run(report, "clippy", "clippy", "affected crates", &mut clippy)
}

fn cargo_command(context: &CheckContext<'_>, verbs: &[&str], args: &[OsString]) -> Command {
    let mut command = context.command("cargo");
    command
        .env("CARGO_TARGET_DIR", &context.cargo_target)
        .args(verbs)
        .arg("--locked")
        .args(args);
    command
}

fn combine_results<const N: usize>(results: [Result<()>; N]) -> Result<()> {
    let failures = results
        .into_iter()
        .filter_map(Result::err)
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>();
    if failures.is_empty() {
        return Ok(());
    }
    bail!(failures.join("\n"))
}

fn discover_ui_tests(root: &Path) -> Result<Vec<PathBuf>> {
    let mut tests = Vec::new();
    collect_ui_tests(root, root, &mut tests)?;
    tests.sort();
    Ok(tests)
}

fn collect_ui_tests(root: &Path, directory: &Path, tests: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to read {}", directory.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_ui_tests(root, &entry.path(), tests)?;
            continue;
        }
        let path = entry.path();
        if !file_type.is_file() || !path.to_string_lossy().ends_with(".test.js") {
            continue;
        }
        tests.push(path.strip_prefix(root)?.to_path_buf());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_test_discovery_is_recursive_and_sorted() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("nested")).unwrap();
        fs::write(directory.path().join("z.test.js"), "").unwrap();
        fs::write(directory.path().join("nested/a.test.js"), "").unwrap();
        fs::write(directory.path().join("nested/no.js"), "").unwrap();

        assert_eq!(
            discover_ui_tests(directory.path()).unwrap(),
            [
                PathBuf::from("nested/a.test.js"),
                PathBuf::from("z.test.js")
            ]
        );
    }

    #[test]
    fn check_mode_accepts_only_the_mode_switches() {
        let cases = [
            (Vec::new(), Ok(CheckMode::Worktree)),
            (vec![OsString::from("--staged")], Ok(CheckMode::Staged)),
            (vec![OsString::from("--lint")], Ok(CheckMode::Lint)),
            (vec![OsString::from("--other")], Err(())),
            (
                vec![OsString::from("--staged"), OsString::from("--lint")],
                Err(()),
            ),
        ];
        for (args, expected) in cases {
            let actual = CheckMode::parse(&args).map_err(|_| ());
            assert_eq!(actual, expected, "args: {args:?}");
        }
    }

    #[test]
    fn lint_context_uses_its_own_target_dir_and_head_base() {
        let directory = tempfile::tempdir().unwrap();
        let affected = directory.path().join("affected.json");
        let cancellation = CancellationToken::new();
        let execution = CheckExecution {
            source_root: directory.path(),
            platform: Platform::Linux,
            base_sha: None,
            affected_path: &affected,
            cancellation: &cancellation,
            verbose: false,
        };
        let context = execution.lint_context(directory.path());

        assert_eq!(
            context.cargo_target,
            directory.path().join("target").join("qol-lint")
        );
        assert_eq!(context.base_sha, Some("HEAD"));
        assert_eq!(context.head, affected::WORKTREE_HEAD);
        assert!(matches!(
            context.containment,
            command::Containment::Preferred
        ));
        assert!(!context.sanitize_git);
    }

    #[test]
    fn staged_context_clears_git_routing_and_cargo_is_locked() {
        let directory = tempfile::tempdir().unwrap();
        let affected = directory.path().join("affected.json");
        let cancellation = CancellationToken::new();
        let context = test_context(directory.path(), &affected, &cancellation, true);
        let mut cargo = cargo_command(&context, &["build"], &[OsString::from("-p"), "qol".into()]);
        context.prepare(&mut cargo);
        let environment = cargo
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();
        let arguments = cargo.get_args().collect::<Vec<_>>();

        assert_eq!(arguments[0], "build");
        assert_eq!(arguments[1], "--locked");
        for variable in ["GIT_INDEX_FILE", "GIT_DIR", "GIT_WORK_TREE", "GIT_PREFIX"] {
            assert_eq!(environment.get(std::ffi::OsStr::new(variable)), Some(&None));
        }
    }

    #[test]
    fn worktree_context_preserves_inherited_git_routing() {
        let directory = tempfile::tempdir().unwrap();
        let affected = directory.path().join("affected.json");
        let cancellation = CancellationToken::new();
        let context = test_context(directory.path(), &affected, &cancellation, false);
        let mut command = context.command("git");
        context.prepare(&mut command);
        let environment = command
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();

        assert!(!environment.contains_key(std::ffi::OsStr::new("GIT_DIR")));
    }

    pub(super) fn test_context<'a>(
        root: &'a Path,
        affected_path: &'a Path,
        cancellation: &'a CancellationToken,
        sanitize_git: bool,
    ) -> CheckContext<'a> {
        CheckContext {
            root,
            cargo_target: root.join("target"),
            platform: Platform::Linux,
            base_sha: Some("base"),
            head: "head",
            affected_path,
            cancellation,
            containment: command::Containment::Preferred,
            sanitize_git,
            verbose: false,
        }
    }
}
