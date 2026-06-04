use crate::cli::optional_single_arg;
use crate::progress::{print_hint, print_title, step_label, LoopProgress, StepKind};
use crate::workspace::{
    display_name, non_host_plugin_packages, repo_root, resolve_crate_target, sibling_crates,
};
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

    let excluded = non_host_plugin_packages(&root)?;
    let mut progress = LoopProgress::new("build", crates.len(), verbose);
    let mut failed = Vec::new();
    for path in &crates {
        let result = progress.step_inline(
            "build",
            StepKind::Pending,
            &display_name(path),
            &mut build_command(path, &root, &excluded),
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

fn build_command(path: &Path, root: &Path, excluded: &[String]) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(path).arg("build");
    if path == root && !excluded.is_empty() {
        command.arg("--workspace");
        for package in excluded {
            command.arg("--exclude").arg(package);
        }
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(command: &Command) -> String {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn build_command_excludes_non_host_plugins_only_at_workspace_root() {
        let root = Path::new("/a/b/ws");
        let excluded = vec!["keyremap".to_string()];
        let cases = [
            (
                root,
                excluded.as_slice(),
                "build --workspace --exclude keyremap",
                "workspace-root build scopes out host-incompatible plugins",
            ),
            (
                root,
                &[] as &[String],
                "build",
                "no excludes (e.g. on macOS) leaves the root build untouched",
            ),
            (
                Path::new("/a/b/ws/plugins/plugin-x"),
                excluded.as_slice(),
                "build",
                "non-root crate builds stay plain",
            ),
        ];
        for (path, excl, want, label) in cases {
            assert_eq!(
                args_of(&build_command(path, root, excl)).as_str(),
                want,
                "{label}"
            );
        }
    }
}
