mod selection;

use crate::cli::optional_single_arg;
use crate::progress::{print_hint, print_title, run_step_inline, step_label, StepKind};
use crate::workspace::{non_host_plugin_packages, repo_root};
use anyhow::{bail, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let target = optional_single_arg(args, "qol build [name]")?;
    let root = repo_root()?;
    print_title("qol build");
    print_hint(verbose);

    let excluded = non_host_plugin_packages(&root)?;
    let features = declared_features(&root)?;
    let mut command = build_command(&root, target, &excluded, &features)?;
    run_step_inline(
        "build",
        StepKind::Pending,
        target.unwrap_or("workspace"),
        &mut command,
        verbose,
    )?;
    step_label("done", StepKind::Success, "");
    Ok(())
}

fn declared_features(root: &Path) -> Result<Vec<String>> {
    qol_dev_build::dev_feature_flags(root).map_err(anyhow::Error::msg)
}

fn build_command(
    root: &Path,
    name: Option<&str>,
    excluded: &[String],
    features: &[String],
) -> Result<Command> {
    let selection = name
        .map(|name| selection::resolve(root, name))
        .transpose()?;
    let mut command = Command::new("cargo");
    command.current_dir(root).args(["build", "--locked"]);
    match &selection {
        Some(target) => {
            if excluded.contains(&target.package) {
                bail!("{} does not support this host platform", target.package);
            }
            command.arg("--package").arg(&target.package);
            if let Some(binary) = &target.binary {
                command.arg("--bin").arg(binary);
            }
        }
        None => {
            command.arg("--workspace");
            for package in excluded {
                command.arg("--exclude").arg(package);
            }
        }
    }
    for feature in features {
        if selection.as_ref().is_some_and(|target| {
            feature.split_once('/').map(|(package, _)| package) != Some(target.package.as_str())
        }) {
            continue;
        }
        command.arg("--features").arg(feature);
    }
    qol_dev_build::configure_dev_cargo(&mut command);
    Ok(command)
}

#[cfg(test)]
mod tests;
