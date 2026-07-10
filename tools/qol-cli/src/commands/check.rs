mod affected;
mod report;

use self::affected::{CargoPlan, Platform};
use self::report::{relative_path, CheckReport};
use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol check");
    }
    let root = repo_root()?;
    let platform = Platform::current()?;
    let started_at = Utc::now();
    let run_dir = root.join("target").join("qol-check").join(format!(
        "{}-{}",
        started_at.timestamp_millis(),
        std::process::id()
    ));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;
    let report_path = run_dir.join("report.json");
    let affected_path = run_dir.join("affected.json");
    let base_sha = affected::comparison_base(&root);
    let mut report = CheckReport::new(
        &root,
        platform.name(),
        base_sha.clone(),
        &report_path,
        &affected_path,
        started_at,
    );

    print_title("qol check");
    print_hint(verbose);
    let result = run_checks(
        &root,
        platform,
        base_sha.as_deref(),
        &affected_path,
        &mut report,
        verbose,
    );
    report.finish(result.is_ok());
    report.write(&report_path)?;
    step_label(
        "report",
        StepKind::Info,
        &relative_path(&root, &report_path),
    );
    result?;
    step_label("done", StepKind::Success, "all checks passed");
    Ok(())
}

fn run_checks(
    root: &Path,
    platform: Platform,
    base_sha: Option<&str>,
    affected_path: &Path,
    report: &mut CheckReport,
    verbose: bool,
) -> Result<()> {
    report.run(
        "single-source-guard",
        "guard",
        "single source",
        Command::new("node")
            .current_dir(root)
            .arg(".githooks/single-source-guard.mjs"),
        verbose,
    )?;
    run_ui_tests(root, report, verbose)?;
    report.run(
        "release-script-tests",
        "scripts",
        "release tests",
        Command::new("python3")
            .current_dir(root)
            .args(["-m", "unittest", "discover", "-s", ".github/scripts/tests"])
            .args(["-p", "test_*.py"]),
        verbose,
    )?;
    report.run(
        "affected-crates",
        "plan",
        "affected crates",
        &mut affected::planner_command(root, base_sha, affected_path),
        verbose,
    )?;
    report.run(
        "rustfmt",
        "format",
        "workspace",
        Command::new("cargo")
            .current_dir(root)
            .args(["fmt", "--all", "--", "--check"]),
        verbose,
    )?;
    let cargo = affected::load_plan(affected_path, platform)?;
    run_rust_checks(root, cargo, report, verbose)
}

fn run_ui_tests(root: &Path, report: &mut CheckReport, verbose: bool) -> Result<()> {
    let ui_root = root.join("apps").join("qol-tray").join("ui");
    let tests = discover_ui_tests(&ui_root)?;
    if tests.is_empty() {
        bail!("no QoL Tray UI tests found");
    }
    let mut command = Command::new("node");
    command.current_dir(&ui_root).arg("--test");
    for test in tests {
        command.arg(test);
    }
    report.run("ui-tests", "ui", "QoL Tray", &mut command, verbose)
}

fn run_rust_checks(
    root: &Path,
    cargo: CargoPlan,
    report: &mut CheckReport,
    verbose: bool,
) -> Result<()> {
    if cargo.skip {
        report.skip("clippy", "no affected crates");
        report.skip("rust-tests", "no affected crates");
        return Ok(());
    }

    let mut clippy = Command::new("cargo");
    clippy
        .current_dir(root)
        .arg("clippy")
        .args(cargo.clippy_args)
        .args(["--", "-D", "warnings"]);
    report.run("clippy", "clippy", "affected crates", &mut clippy, verbose)?;

    let mut tests = Command::new("cargo");
    tests.current_dir(root).arg("test").args(cargo.test_args);
    report.run("rust-tests", "test", "affected crates", &mut tests, verbose)
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
}
