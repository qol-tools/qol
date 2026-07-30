use crate::cli::optional_single_arg;
use crate::progress::{print_hint, print_title, step_label, LoopProgress, StepKind};
use crate::workspace::{display_name, repo_root, resolve_target_crates};
use anyhow::Result;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let target = optional_single_arg(args, "qol clean [name]")?;
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
