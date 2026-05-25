use crate::cli::optional_single_arg;
use crate::progress::{print_hint, print_title, step_label, LoopProgress, StepKind};
use crate::workspace::{display_name, repo_root, resolve_crate_target, sibling_crates};
use anyhow::{bail, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let target = optional_single_arg(args, "qol build [name]")?;
    let root = repo_root()?;
    print_title("qol build");
    print_hint(verbose);

    let crates = match target {
        Some(name) => vec![resolve_crate_target(&root, name)?],
        None => {
            let mut all = vec![root.clone()];
            all.extend(sibling_crates(&root)?);
            all
        }
    };

    let mut progress = LoopProgress::new("build", crates.len(), verbose);
    let mut failed = Vec::new();
    for path in &crates {
        let result = progress.step_inline(
            "build",
            StepKind::Pending,
            &display_name(path),
            &mut build_command(path),
            verbose,
        );
        if result.is_err() {
            failed.push(display_name(path));
        }
    }
    progress.finish();

    if !failed.is_empty() {
        bail!("failed: {}", failed.join(" "));
    }
    step_label("done", StepKind::Success, "");
    Ok(())
}

fn build_command(path: &Path) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(path).arg("build");
    command
}
