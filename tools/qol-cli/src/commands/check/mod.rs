mod affected;
mod command;
mod destination;
mod fingerprint;
mod format;
mod options;
mod report;
mod snapshot;
#[cfg(test)]
mod test_support;
mod testing;

use self::affected::{CargoPlan, Platform};
use self::format::{FormatFailure, FormatOutcome};
use self::options::{normalize_path, CheckMode, CheckOptions};
use self::report::{publish_bytes, relative_path, CheckReport};
use self::snapshot::{SourceState, StagedSnapshot};
use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use qol_process::CancellationToken;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let requested_report = options::requested_report(args);
    let mut options = match CheckOptions::parse(args) {
        Ok(options) => options,
        Err(error) => {
            let publication = match requested_report.as_deref() {
                Some(requested) => repo_root().and_then(|source_root| {
                    let report_destination = options::resolve_report_path(requested)?;
                    destination::validate_report_destination(&source_root, &report_destination)?;
                    publish_validation_failure(&source_root, &report_destination, &error)
                }),
                None => Ok(()),
            };
            return Err(combine_errors(error, publication));
        }
    };
    let source_root = repo_root()?;
    let resolved_report = match options.report.as_deref() {
        Some(requested) => {
            let report_destination = options::resolve_report_path(requested)?;
            destination::validate_report_destination(&source_root, &report_destination)?;
            Some(report_destination)
        }
        None => None,
    };
    options.report = resolved_report;
    let platform = Platform::current()?;
    let started_at = Utc::now();
    let run_dir = source_root.join("target").join("qol-check").join(format!(
        "{}-{}",
        started_at.timestamp_millis(),
        std::process::id()
    ));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;
    let affected_path = run_dir.join("affected.json");

    print_title("qol check");
    print_hint(verbose);
    let invocation = CheckInvocation {
        options: &options,
        source_root: &source_root,
        platform,
        run_dir: &run_dir,
        affected_path: &affected_path,
        verbose,
    };
    run_and_report(&invocation, started_at)
}

struct CheckInvocation<'a> {
    options: &'a CheckOptions,
    source_root: &'a Path,
    platform: Platform,
    run_dir: &'a Path,
    affected_path: &'a Path,
    verbose: bool,
}

fn publish_validation_failure(
    source_root: &Path,
    destination: &Path,
    error: &anyhow::Error,
) -> Result<()> {
    let platform = Platform::current().map(Platform::name).unwrap_or("unknown");
    let mut report = CheckReport::new(source_root, "validation", platform, destination, Utc::now());
    report.finish(&Err(anyhow!(format!("{error:#}"))), false);
    report.write(destination)
}

fn combine_errors(primary: anyhow::Error, secondary: Result<()>) -> anyhow::Error {
    match secondary {
        Ok(()) => primary,
        Err(secondary) => anyhow!("{primary:#}\n{secondary:#}"),
    }
}

fn run_and_report(invocation: &CheckInvocation<'_>, started_at: DateTime<Utc>) -> Result<()> {
    let report_path = invocation.run_dir.join("report.json");
    let mut report = CheckReport::new(
        invocation.source_root,
        invocation.options.mode.name(),
        invocation.platform.name(),
        &report_path,
        started_at,
    );
    report.set_requested_base(invocation.options.base.as_deref());
    let cancellation = CancellationToken::install();
    let mut result = match &cancellation {
        Ok(cancellation) => execute(invocation, &mut report, cancellation),
        Err(error) => {
            Err(anyhow!(error.to_string())).context("failed to install check cancellation handler")
        }
    };
    let cancelled = cancellation
        .as_ref()
        .is_ok_and(|cancellation| cancellation.is_cancelled());
    if cancelled && result.is_ok() {
        result = Err(anyhow!("check cancelled"));
    }
    report.set_affected_plan(invocation.source_root, invocation.affected_path);
    let publication = publish_and_finalize(
        &mut report,
        invocation.source_root,
        &report_path,
        invocation.options.report.as_deref(),
        &result,
        cancelled,
        publish_bytes,
    );
    if publication.is_ok() {
        step_label(
            "report",
            StepKind::Info,
            &relative_path(invocation.source_root, &report_path),
        );
        if let Some(destination) = invocation.options.report.as_deref() {
            step_label(
                "publish",
                StepKind::Info,
                &relative_path(invocation.source_root, destination),
            );
        }
    }
    combine_results([result, publication])?;
    step_label("done", StepKind::Success, "all checks passed");
    Ok(())
}

fn publish_and_finalize(
    report: &mut CheckReport,
    root: &Path,
    report_path: &Path,
    destination: Option<&Path>,
    run_result: &Result<()>,
    cancelled: bool,
    publish: impl FnOnce(&[u8], &Path) -> Result<()>,
) -> Result<()> {
    let Some(destination) = destination else {
        report.finish(run_result, cancelled);
        return report.write(report_path);
    };
    report.set_publication_pending(root, destination);
    report.finish(run_result, cancelled);
    let provisional = report.write(report_path);
    if provisional.is_err() {
        report.set_publication_failure("the authoritative report could not be written");
        return provisional;
    }
    report.set_publication_published();
    report.finish(run_result, cancelled);
    let final_bytes = report.render()?;
    if let Err(publication_error) = publish(&final_bytes, destination) {
        let failure_text = format!("{publication_error:#}");
        report.set_publication_failure(&failure_text);
        report.finish(run_result, cancelled);
        return match report.write(report_path) {
            Ok(()) => Err(publication_error),
            Err(rewrite_error) => Err(anyhow!("{failure_text}\n{rewrite_error:#}")),
        };
    }
    publish_bytes(&final_bytes, report_path)
}

fn execute(
    invocation: &CheckInvocation<'_>,
    report: &mut CheckReport,
    cancellation: &CancellationToken,
) -> Result<()> {
    let options = invocation.options;
    let source_root = invocation.source_root;
    let source_state = SourceState::capture(source_root)?;
    report.set_source_state(&source_state.head, &source_state.index_tree);
    let base_sha = resolve_base(source_root, options, &source_state.head)?;
    report.set_base_sha(base_sha.as_deref());
    let target = resolve_check_target(
        options.mode,
        source_root,
        std::env::var_os("CARGO_TARGET_DIR").as_deref(),
    )?;
    let target_lock = acquire_target_lock(options.mode, &target, cancellation)?;
    if !options.format_owned.is_empty() {
        run_owned_formatting(source_root, &options.format_owned, report, cancellation)?;
    }
    let exclusions = {
        let mut exclusions = fingerprint_exclusions(invocation.run_dir);
        if let Some(exclusion) = &target.exclusion {
            exclusions.push(exclusion.clone());
        }
        exclusions
    };
    let before = match fingerprint::SourceFingerprint::capture(source_root, &exclusions) {
        Ok(before) => before,
        Err(error) => {
            report.set_fingerprint_error(&format!("{error:#}"));
            return Err(error).context("failed to capture the source fingerprint");
        }
    };
    report.set_fingerprint_before(&before.digest);
    let execution = CheckExecution {
        source_root,
        platform: invocation.platform,
        base_sha: base_sha.as_deref(),
        affected_path: invocation.affected_path,
        cancellation,
        verbose: invocation.verbose,
    };
    let result = match options.mode {
        CheckMode::Worktree => run_checks(
            &execution.context(
                source_root,
                target.path.clone(),
                affected::WORKTREE_HEAD,
                command::Containment::Preferred,
                false,
            ),
            report,
        ),
        CheckMode::Staged => run_staged_checks(&execution, source_state, report),
        CheckMode::Lint => run_lint_checks(
            &execution.lint_context(source_root, target.path.clone()),
            report,
        ),
    };
    drop(target_lock);
    verify_fingerprint(source_root, &exclusions, &before, report, result)
}

struct CheckTarget {
    path: PathBuf,
    exclusion: Option<PathBuf>,
}

fn resolve_check_target(
    mode: CheckMode,
    source_root: &Path,
    configured: Option<&OsStr>,
) -> Result<CheckTarget> {
    let canonical_root = source_root
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", source_root.display()))?;
    let requested = match mode {
        CheckMode::Worktree => resolve_cargo_target(source_root, configured),
        CheckMode::Lint => source_root.join("target").join("qol-lint"),
        CheckMode::Staged => source_root.join("target").join("qol-check").join("staged"),
    };
    let canonical = options::resolve_existing_ancestors(&requested)?;
    if canonical == canonical_root {
        bail!(
            "the check target resolves to the repository root; refusing to use {}",
            requested.display()
        );
    }
    if !canonical.starts_with(&canonical_root) {
        return Ok(CheckTarget {
            path: canonical,
            exclusion: None,
        });
    }
    let default_target = canonical_root.join("target");
    let relative = canonical
        .strip_prefix(&canonical_root)
        .with_context(|| format!("failed to relativize {}", canonical.display()))?;
    if fingerprint::git_tracks(source_root, relative)? {
        bail!(
            "the check target {} contains tracked source files; refusing to exclude it",
            requested.display()
        );
    }
    let node_owned = canonical.starts_with(&default_target);
    let approved = node_owned || fingerprint::git_ignores(source_root, relative)?;
    if !approved {
        bail!(
            "the check target {} is inside the repository and is neither the default target nor git-ignored; refusing to use it",
            requested.display()
        );
    }
    Ok(CheckTarget {
        path: canonical.clone(),
        exclusion: Some(canonical),
    })
}

fn acquire_target_lock(
    mode: CheckMode,
    target: &CheckTarget,
    cancellation: &CancellationToken,
) -> Result<Option<snapshot::TargetLock>> {
    if mode == CheckMode::Staged {
        return Ok(None);
    }
    snapshot::TargetLock::acquire(&target.path, cancellation).map(Some)
}

fn resolve_cargo_target(source_root: &Path, configured: Option<&OsStr>) -> PathBuf {
    match configured {
        Some(target) if Path::new(target).is_absolute() => normalize_path(Path::new(target)),
        Some(target) => normalize_path(&source_root.join(target)),
        None => source_root.join("target"),
    }
}

fn fingerprint_exclusions(run_dir: &Path) -> Vec<PathBuf> {
    vec![normalize_path(run_dir)]
}

fn run_owned_formatting(
    source_root: &Path,
    requested: &[OsString],
    report: &mut CheckReport,
    cancellation: &CancellationToken,
) -> Result<()> {
    report.set_formatting_requested(
        requested
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect(),
    );
    match format::run(source_root, requested, cancellation) {
        Ok(FormatOutcome { files }) => {
            report.set_formatting_outcome(files, None, None);
            Ok(())
        }
        Err(FormatFailure {
            files,
            failed_file,
            error,
        }) => {
            report.set_formatting_outcome(files, failed_file, Some(format!("{error:#}")));
            Err(error)
        }
    }
}

fn verify_fingerprint(
    source_root: &Path,
    exclusions: &[PathBuf],
    before: &fingerprint::SourceFingerprint,
    report: &mut CheckReport,
    result: Result<()>,
) -> Result<()> {
    match fingerprint::SourceFingerprint::capture(source_root, exclusions) {
        Ok(after) => {
            report.set_fingerprint_after(&after.digest);
            match fingerprint::compare(before, &after) {
                Ok(()) => result,
                Err(error) => {
                    report.mark_stale();
                    step_label("stale", StepKind::Info, "source changed during checks");
                    combine_results([result, Err(error)])
                }
            }
        }
        Err(error) => {
            report.set_fingerprint_error(&format!("{error:#}"));
            combine_results([
                result,
                Err(error).context("failed to recapture the source fingerprint"),
            ])
        }
    }
}

fn resolve_base(source_root: &Path, options: &CheckOptions, head: &str) -> Result<Option<String>> {
    if let Some(revision) = &options.base {
        return resolve_commit(source_root, revision).map(Some);
    }
    Ok(match options.mode {
        CheckMode::Lint => Some("HEAD".to_string()),
        _ => affected::comparison_base(source_root, head),
    })
}

fn resolve_commit(source_root: &Path, revision: &str) -> Result<String> {
    let mut command = Command::new("git");
    command
        .current_dir(source_root)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{revision}^{{commit}}"));
    snapshot::sanitize_git_environment(&mut command);
    let output = command
        .output()
        .context("failed to resolve the requested --base revision")?;
    if !output.status.success() {
        bail!("--base {revision} does not resolve to a commit");
    }
    let resolved = String::from_utf8(output.stdout)
        .context("git returned a non-UTF-8 --base revision")?
        .trim()
        .to_string();
    if resolved.is_empty() {
        bail!("--base {revision} does not resolve to a commit");
    }
    Ok(resolved)
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

    fn lint_context<'a>(&'a self, root: &'a Path, cargo_target: PathBuf) -> CheckContext<'a> {
        CheckContext {
            root,
            cargo_target,
            platform: self.platform,
            base_sha: self.base_sha.or(Some("HEAD")),
            head: affected::WORKTREE_HEAD,
            affected_path: self.affected_path,
            cancellation: self.cancellation,
            containment: command::Containment::Preferred,
            sanitize_git: false,
            verbose: self.verbose,
        }
    }
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
    use super::test_support::{commit_file, git, repository, rustfmt_available};
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
    fn explicit_base_overrides_the_default_comparison_base() {
        let repository = repository();
        let root = repository.path();
        let first = commit_file(root, "tracked.txt", "first\n");
        let head = commit_file(root, "tracked.txt", "second\n");
        let explicit = CheckOptions {
            mode: CheckMode::Worktree,
            base: Some("HEAD".to_string()),
            report: None,
            format_owned: Vec::new(),
        };
        let default = CheckOptions {
            mode: CheckMode::Worktree,
            base: None,
            report: None,
            format_owned: Vec::new(),
        };

        assert_eq!(
            resolve_base(root, &explicit, &head).unwrap(),
            Some(head.clone())
        );
        assert_eq!(resolve_base(root, &default, &head).unwrap(), Some(first));
    }

    #[test]
    fn an_unknown_base_revision_is_rejected() {
        let repository = repository();
        let root = repository.path();
        let head = commit_file(root, "tracked.txt", "next\n");
        let options = CheckOptions {
            mode: CheckMode::Worktree,
            base: Some("does-not-exist".to_string()),
            report: None,
            format_owned: Vec::new(),
        };

        let error = resolve_base(root, &options, &head).unwrap_err().to_string();

        assert!(
            error.contains("does not resolve to a commit"),
            "got: {error}"
        );
    }

    #[test]
    fn cargo_target_resolution_honors_absolute_and_relative_overrides() {
        let root = Path::new("/repo");

        assert_eq!(
            resolve_cargo_target(root, None),
            PathBuf::from("/repo/target")
        );
        assert_eq!(
            resolve_cargo_target(root, Some(OsStr::new("/shared/target"))),
            PathBuf::from("/shared/target")
        );
        assert_eq!(
            resolve_cargo_target(root, Some(OsStr::new("build/target"))),
            PathBuf::from("/repo/build/target")
        );
    }

    #[test]
    fn fingerprint_exclusions_cover_only_the_run_directory() {
        let run_dir = Path::new("/repo/target/qol-check/run-1");

        assert_eq!(
            fingerprint_exclusions(run_dir),
            [PathBuf::from("/repo/target/qol-check/run-1")]
        );
        assert_eq!(fingerprint_exclusions(run_dir).len(), 1);
    }

    #[test]
    fn check_target_rejects_the_repository_root() {
        let repository = repository();
        let root = repository.path();

        let error = resolve_check_target(CheckMode::Worktree, root, Some(OsStr::new(".")))
            .err()
            .unwrap()
            .to_string();

        assert!(error.contains("repository root"), "got: {error}");
    }

    #[test]
    fn check_target_rejects_tracked_source_directories() {
        let repository = repository();
        let root = repository.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn main() {}\n").unwrap();
        git(root, ["add", "src/lib.rs"]);
        git(root, ["commit", "--quiet", "-m", "src"]);

        let error = resolve_check_target(CheckMode::Worktree, root, Some(OsStr::new("src")))
            .err()
            .unwrap()
            .to_string();

        assert!(error.contains("tracked source files"), "got: {error}");
    }

    #[test]
    fn check_target_excludes_node_owned_ignored_and_external_locations() {
        let repository = repository();
        let root = repository.path();
        let canonical_root = root.canonicalize().unwrap();

        let default = resolve_check_target(CheckMode::Worktree, root, None).unwrap();
        assert_eq!(default.path, canonical_root.join("target"));
        assert_eq!(default.exclusion, Some(canonical_root.join("target")));

        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        git(root, ["add", ".gitignore"]);
        git(root, ["commit", "--quiet", "-m", "ignore build"]);
        let ignored =
            resolve_check_target(CheckMode::Worktree, root, Some(OsStr::new("build/target")))
                .unwrap();
        assert_eq!(ignored.exclusion, Some(canonical_root.join("build/target")));

        let error = resolve_check_target(CheckMode::Worktree, root, Some(OsStr::new("untracked")))
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("neither the default target nor git-ignored"),
            "got: {error}"
        );

        let external = tempfile::tempdir().unwrap();
        let external_target =
            resolve_check_target(CheckMode::Worktree, root, Some(external.path().as_os_str()))
                .unwrap();
        assert!(external_target.exclusion.is_none());

        for mode in [CheckMode::Lint, CheckMode::Staged] {
            let target = resolve_check_target(mode, root, None).unwrap();
            assert!(target.exclusion.is_some(), "mode: {mode:?}");
            assert!(target.path.starts_with(canonical_root.join("target")));
        }
    }

    #[test]
    fn publication_failure_is_recorded_in_the_authoritative_report() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let report_path = root.join("report.json");
        let destination = root.join("external/report.json");
        let mut report = CheckReport::new(root, "worktree", "linux", &report_path, Utc::now());
        let run_result: Result<()> = Ok(());

        let outcome = publish_and_finalize(
            &mut report,
            root,
            &report_path,
            Some(&destination),
            &run_result,
            false,
            |_, _| {
                let durable: serde_json::Value =
                    serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
                assert_eq!(durable["publication"]["status"], "pending");
                assert!(durable["publication"]["published"].is_null());
                assert_ne!(durable["state"], "verified");
                Err(anyhow!("injected publication failure"))
            },
        );

        assert!(outcome.is_err());
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        assert_eq!(value["status"], "failed");
        assert_ne!(value["state"], "verified");
        assert_eq!(value["publication"]["status"], "failed");
        assert!(value["publication"]["published"].is_null());
        assert!(value["publication"]["error"]
            .as_str()
            .unwrap()
            .contains("injected publication failure"));
        assert!(!destination.exists());
    }

    #[test]
    fn publication_pending_is_durable_during_the_attempt() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let report_path = root.join("report.json");
        let destination = root.join("external/report.json");
        let mut report = CheckReport::new(root, "worktree", "linux", &report_path, Utc::now());
        report.set_fingerprint_before("before-digest");
        report.set_fingerprint_after("before-digest");
        let run_result: Result<()> = Ok(());

        let outcome = publish_and_finalize(
            &mut report,
            root,
            &report_path,
            Some(&destination),
            &run_result,
            false,
            |bytes, destination| {
                let durable: serde_json::Value =
                    serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
                assert_eq!(durable["publication"]["status"], "pending");
                assert!(durable["publication"]["published"].is_null());
                assert_eq!(durable["state"], "pending");
                assert_eq!(durable["status"], "pending");
                let payload: serde_json::Value = serde_json::from_slice(bytes).unwrap();
                assert_eq!(payload["publication"]["status"], "published");
                assert_eq!(payload["state"], "verified");
                publish_bytes(bytes, destination)
            },
        );

        assert!(outcome.is_ok());
        let internal: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        let external: serde_json::Value =
            serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
        assert_eq!(internal, external);
        assert_eq!(internal["state"], "verified");
        assert_eq!(internal["publication"]["status"], "published");
        assert_eq!(internal["publication"]["published"], "external/report.json");
        assert_eq!(
            fs::read(&report_path).unwrap(),
            fs::read(&destination).unwrap()
        );
    }

    #[test]
    fn cancelled_publication_never_reports_published() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let report_path = root.join("report.json");
        let destination = root.join("external/report.json");
        let mut report = CheckReport::new(root, "worktree", "linux", &report_path, Utc::now());
        let run_result: Result<()> = Err(anyhow!("check cancelled"));

        let outcome = publish_and_finalize(
            &mut report,
            root,
            &report_path,
            Some(&destination),
            &run_result,
            true,
            |_, _| {
                let durable: serde_json::Value =
                    serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
                assert_eq!(durable["state"], "cancelled");
                assert!(durable["publication"]["published"].is_null());
                Err(anyhow!("injected publication failure"))
            },
        );

        assert!(outcome.is_err());
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        assert_eq!(value["status"], "failed");
        assert_eq!(value["state"], "cancelled");
        assert_eq!(value["publication"]["status"], "failed");
        assert!(value["publication"]["published"].is_null());
        assert!(!destination.exists());
    }

    #[test]
    fn successful_publication_matches_the_final_internal_report() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let report_path = root.join("report.json");
        let destination = root.join("external/report.json");
        let mut report = CheckReport::new(root, "worktree", "linux", &report_path, Utc::now());
        let run_result: Result<()> = Ok(());

        let outcome = publish_and_finalize(
            &mut report,
            root,
            &report_path,
            Some(&destination),
            &run_result,
            false,
            publish_bytes,
        );

        assert!(outcome.is_ok());
        assert_eq!(
            fs::read(&destination).unwrap(),
            fs::read(&report_path).unwrap()
        );
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        assert_eq!(value["state"], "verified");
        assert_eq!(value["publication"]["status"], "published");
        assert_eq!(value["publication"]["published"], "external/report.json");
    }

    #[test]
    fn partial_formatting_failure_records_rewritten_files() {
        if !rustfmt_available() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "fn  a() {}\n").unwrap();
        fs::write(root.join("src/b.rs"), "fn broken( {\n").unwrap();
        let report_path = root.join("report.json");
        let mut report = CheckReport::new(root, "worktree", "linux", &report_path, Utc::now());
        let cancellation = CancellationToken::new();
        let requested = [OsString::from("src/a.rs"), OsString::from("src/b.rs")];

        let error = run_owned_formatting(root, &requested, &mut report, &cancellation).unwrap_err();
        report.finish(&Err(error), false);
        report.write(&report_path).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();

        assert_eq!(value["formatting"]["status"], "failed");
        assert_eq!(value["formatting"]["failed_file"], "src/b.rs");
        assert_eq!(value["formatting"]["files"][0]["path"], "src/a.rs");
        assert!(value["formatting"]["files"][0]["changed"]
            .as_bool()
            .unwrap());
    }

    #[test]
    fn a_source_change_during_checks_marks_the_report_stale() {
        let repository = repository();
        let root = repository.path();
        let mut report = CheckReport::new(
            root,
            "worktree",
            "linux",
            &root.join("report.json"),
            Utc::now(),
        );
        let before = fingerprint::SourceFingerprint::capture(root, &[]).unwrap();

        assert!(verify_fingerprint(root, &[], &before, &mut report, Ok(())).is_ok());

        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        let error = verify_fingerprint(root, &[], &before, &mut report, Ok(()))
            .unwrap_err()
            .to_string();
        report.finish(&Err(anyhow!("source changed during checks")), false);
        let value: serde_json::Value = serde_json::from_slice(&report.render().unwrap()).unwrap();

        assert!(error.contains("source changed"), "got: {error}");
        assert_eq!(value["status"], "failed");
        assert_eq!(value["state"], "stale");
        assert_eq!(value["fingerprint"]["status"], "stale");
    }

    #[test]
    fn validation_failures_publish_a_machine_report_when_requested() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("nested/report.json");
        let error = anyhow!("usage: qol check [--staged|--lint]");

        publish_validation_failure(directory.path(), &destination, &error).unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&destination).unwrap()).unwrap();
        assert_eq!(value["inputs"]["mode"], "validation");
        assert_eq!(value["status"], "failed");
        assert!(value["error"]
            .as_str()
            .unwrap()
            .contains("usage: qol check"));
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
        let cargo_target = directory.path().join("custom-target");
        let context = execution.lint_context(directory.path(), cargo_target.clone());

        assert_eq!(context.cargo_target, cargo_target);
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
