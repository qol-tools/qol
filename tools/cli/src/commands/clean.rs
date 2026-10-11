use crate::cli::optional_single_arg;
use crate::progress::{print_hint, print_title, step_label, LoopProgress, StepKind};
use crate::workspace::{display_name, repo_root, resolve_target_crates};
use anyhow::{anyhow, bail, Result};
use qol_dev_build::target_cache::{
    cargo_cache_dirs, format_bytes, sweep_cargo_cache_dir, SweepMode, SweepReport,
};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const USAGE: &str = "qol clean [name] | qol clean --prune [--dry-run] [dir...]";

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    if let Some(prune) = PruneArgs::parse(&utf8_args(args)?)? {
        return run_prune(&prune);
    }
    let target = optional_single_arg(args, USAGE)?;
    let root = repo_root()?;
    print_title("qol clean");
    print_hint(verbose);

    let crates = resolve_target_crates(&root, target)?;

    let mut progress = LoopProgress::new("clean", crates.len(), verbose);
    for path in &crates {
        unseal_env_runs(path);
        let _ = progress.step_silent(
            "clean",
            StepKind::Pending,
            &display_name(path),
            &mut clean_command(path, verbose),
            verbose,
        );
    }
    progress.finish();

    step_label("done", StepKind::Success, "");
    Ok(())
}

pub(crate) fn prune_json(args: &[String]) -> Result<Value> {
    let (value, problems) = prune_value(args)?;
    if problems > 0 {
        bail!("qol clean --prune could not remove {problems} cache entries: {value}");
    }
    Ok(value)
}

fn prune_value(args: &[String]) -> Result<(Value, usize)> {
    let prune =
        PruneArgs::parse(args)?.ok_or_else(|| anyhow!("usage: {USAGE} (--json needs --prune)"))?;
    let outcomes = sweep_all(&prune)?;
    let problems: usize = outcomes
        .iter()
        .map(|(_, outcome)| outcome.as_ref().map_or(1, |report| report.failures.len()))
        .sum();
    let freed_bytes: u64 = outcomes
        .iter()
        .filter_map(|(_, outcome)| outcome.as_ref().ok())
        .map(|report| report.freed_bytes)
        .sum();
    let dirs: Vec<Value> = outcomes
        .into_iter()
        .map(|(dir, outcome)| match outcome {
            Ok(report) => json!(report),
            Err(error) => json!({ "dir": dir, "error": error }),
        })
        .collect();
    let value = json!({
        "dry_run": prune.mode == SweepMode::DryRun,
        "freed_bytes": freed_bytes,
        "dirs": dirs,
    });
    Ok((value, problems))
}

#[derive(Debug, PartialEq, Eq)]
struct PruneArgs {
    mode: SweepMode,
    dirs: Vec<PathBuf>,
}

impl PruneArgs {
    fn parse(args: &[String]) -> Result<Option<Self>> {
        let mut prune = false;
        let mut mode = SweepMode::Apply;
        let mut dirs = Vec::new();
        for arg in args {
            match arg.as_str() {
                "--prune" => prune = true,
                "--dry-run" => mode = SweepMode::DryRun,
                flag if flag.starts_with('-') => bail!("usage: {USAGE} (unknown flag {flag})"),
                dir => dirs.push(PathBuf::from(dir)),
            }
        }
        if !prune {
            if mode == SweepMode::DryRun {
                bail!("usage: {USAGE} (--dry-run needs --prune)");
            }
            return Ok(None);
        }
        Ok(Some(Self { mode, dirs }))
    }
}

fn utf8_args(args: &[OsString]) -> Result<Vec<String>> {
    args.iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("argument is not valid UTF-8"))
        })
        .collect()
}

type Outcome = (PathBuf, Result<SweepReport, String>);

fn sweep_all(prune: &PruneArgs) -> Result<Vec<Outcome>> {
    let dirs = if prune.dirs.is_empty() {
        cargo_cache_dirs(&repo_root()?)
            .into_iter()
            .filter(|dir| dir.exists())
            .collect()
    } else {
        prune.dirs.clone()
    };
    Ok(dirs
        .into_iter()
        .map(|dir| {
            let outcome = sweep_cargo_cache_dir(&dir, prune.mode);
            (dir, outcome)
        })
        .collect())
}

fn run_prune(prune: &PruneArgs) -> Result<()> {
    print_title(match prune.mode {
        SweepMode::DryRun => "qol clean --prune --dry-run",
        SweepMode::Apply => "qol clean --prune",
    });
    let mut problems = 0;
    for (dir, outcome) in sweep_all(prune)? {
        match outcome {
            Ok(report) => {
                print_report(&report);
                problems += report.failures.len();
            }
            Err(error) => {
                step_label(
                    "failed",
                    StepKind::Info,
                    &format!("{}: {error}", dir.display()),
                );
                problems += 1;
            }
        }
    }
    if problems > 0 {
        bail!("qol clean --prune could not remove {problems} cache entries");
    }
    step_label("done", StepKind::Success, "");
    Ok(())
}

fn print_report(report: &SweepReport) {
    let verb = if report.dry_run { "prunable" } else { "freed" };
    step_label(
        verb,
        StepKind::Success,
        &format!(
            "{} in {}",
            format_bytes(report.freed_bytes),
            report.dir.display()
        ),
    );
    for (label, tally) in [
        ("unused build outputs", &report.units),
        ("replaced incremental sessions", &report.superseded_sessions),
        ("idle incremental crate caches", &report.idle_crate_caches),
        ("idle cache roots", &report.stale_roots),
    ] {
        if tally.count > 0 {
            step_label(
                "",
                StepKind::Info,
                &format!("{} {label} ({})", tally.count, format_bytes(tally.bytes)),
            );
        }
    }
    if !report.access_times {
        step_label(
            "note",
            StepKind::Info,
            "this disk does not record file reads, so only caches cargo can no longer use go",
        );
    }
    for reason in &report.skipped {
        step_label("skipped", StepKind::Info, reason);
    }
    for failure in &report.failures {
        step_label("failed", StepKind::Info, failure);
    }
}

fn unseal_env_runs(path: &Path) {
    let runs = path.join("target").join("qol-env");
    if let Err(error) = qol_dev_env::payload::make_tree_writable(&runs) {
        step_label(
            "unseal",
            StepKind::Info,
            &format!("{}: {error:#}", runs.display()),
        );
    }
}

fn clean_command(path: &Path, verbose: bool) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(path).arg("clean");
    if !verbose {
        command.arg("-q");
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn prune_flags_parse_into_a_mode_and_directories() {
        assert_eq!(PruneArgs::parse(&args(&[])).expect("no args"), None);
        assert_eq!(PruneArgs::parse(&args(&["qol-tray"])).expect("name"), None);
        assert_eq!(
            PruneArgs::parse(&args(&["--prune"])).expect("prune"),
            Some(PruneArgs {
                mode: SweepMode::Apply,
                dirs: Vec::new(),
            })
        );
        assert_eq!(
            PruneArgs::parse(&args(&["--dry-run", "--prune", "/a", "/b"])).expect("dry run"),
            Some(PruneArgs {
                mode: SweepMode::DryRun,
                dirs: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            })
        );
    }

    #[test]
    fn prune_flags_reject_what_they_cannot_honour() {
        for (values, needle) in [
            (&["--dry-run"][..], "--dry-run needs --prune"),
            (&["--prune", "--force"][..], "unknown flag --force"),
        ] {
            let error = PruneArgs::parse(&args(values)).expect_err("usage error");
            assert!(error.to_string().contains(needle), "{error}");
        }
    }

    #[test]
    fn json_prune_reports_every_directory_with_its_own_outcome() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing");
        std::fs::write(dir.path().join("CACHEDIR.TAG"), b"").expect("marker");

        let prune_args = args(&[
            "--prune",
            "--dry-run",
            dir.path().to_str().expect("utf8"),
            missing.to_str().expect("utf8"),
        ]);
        let (value, problems) = prune_value(&prune_args).expect("json");

        assert_eq!(problems, 1);
        assert!(prune_json(&prune_args).is_err(), "a failed dir exits non-zero");

        assert_eq!(value["dry_run"], true);
        assert_eq!(value["freed_bytes"], 0);
        assert_eq!(value["dirs"][0]["dir"], dir.path().to_str().expect("utf8"));
        assert_eq!(value["dirs"][0]["dry_run"], true);
        assert!(
            value["dirs"][1]["error"]
                .as_str()
                .is_some_and(|error| error.contains("failed to read cargo cache dir")),
            "{value}"
        );
        assert!(prune_json(&args(&["qol-tray"])).is_err());
    }
}
