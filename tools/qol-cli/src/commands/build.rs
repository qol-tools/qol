use crate::cli::optional_single_arg;
use crate::progress::{print_hint, print_title, step_label, LoopProgress, StepKind};
use crate::workspace::{
    cargo_bin_name, display_name, non_host_plugin_packages, repo_root, resolve_target_crates,
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

    let crates = resolve_target_crates(&root, target)?;

    let excluded = non_host_plugin_packages(&root)?;
    let features = declared_features(&root)?;
    let mut progress = LoopProgress::new("build", crates.len(), verbose);
    let mut failed = Vec::new();
    for path in &crates {
        let mut command = build_command(path, &root, &excluded, &features)?;
        let result = progress.step_inline(
            "build",
            StepKind::Pending,
            &display_name(path),
            &mut command,
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

fn declared_features(root: &Path) -> Result<Vec<String>> {
    qol_dev_build::dev_feature_flags(root).map_err(anyhow::Error::msg)
}

fn build_command(
    path: &Path,
    root: &Path,
    excluded: &[String],
    features: &[String],
) -> Result<Command> {
    let mut command = Command::new("cargo");
    command.current_dir(root).arg("build");
    if path == root {
        if !excluded.is_empty() {
            command.arg("--workspace");
            for package in excluded {
                command.arg("--exclude").arg(package);
            }
        }
    } else {
        command.arg("--workspace").arg("--bin");
        command.arg(cargo_bin_name(path)?);
    }
    for feature in features {
        command.arg("--features").arg(feature);
    }
    Ok(command)
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
    fn build_command_excludes_non_host_plugins_at_root_and_builds_workspace_bins() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        let plugin = root.join("plugins/plugin-x");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(
            plugin.join("Cargo.toml"),
            "[package]\nname = \"plugin-x\"\n\n[[bin]]\nname = \"x-bin\"\npath = \"src/main.rs\"\n",
        )
        .unwrap();
        let excluded = vec!["keyremap".to_string()];
        let none = &[] as &[String];
        let features = ["qol-tray/dev".to_string()];

        assert_eq!(
            args_of(&build_command(&root, &root, &excluded, &features).unwrap()),
            "build --workspace --exclude keyremap --features qol-tray/dev",
            "workspace-root build scopes out host-incompatible plugins"
        );
        assert_eq!(
            args_of(&build_command(&root, &root, none, &features).unwrap()),
            "build --features qol-tray/dev",
            "no excludes (e.g. on macOS) leaves the root build untouched"
        );
        assert_eq!(
            args_of(&build_command(&plugin, &root, &excluded, none).unwrap()),
            "build --workspace --bin x-bin",
            "non-root crates build their workspace bin"
        );
        assert_eq!(
            build_command(&plugin, &root, &excluded, none)
                .unwrap()
                .get_current_dir(),
            Some(root.as_path()),
            "plugin builds run from the workspace root"
        );
    }
}
