use crate::cli::optional_single_arg;
use crate::progress::{print_hint, print_title, step_label, LoopProgress, StepKind};
use crate::workspace::{
    display_name, non_host_plugin_packages, plugin_build_features, qualified_plugin_build_features,
    repo_root, resolve_target_crates, scan_buildable_plugins,
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
    let mut progress = LoopProgress::new("build", crates.len(), verbose);
    let mut failed = Vec::new();
    for path in &crates {
        let features = declared_features(path, &root)?;
        let result = progress.step_inline(
            "build",
            StepKind::Pending,
            &display_name(path),
            &mut build_command(path, &root, &excluded, &features),
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

fn declared_features(path: &Path, root: &Path) -> Result<Vec<String>> {
    if path != root {
        return Ok(plugin_build_features(path));
    }
    let mut features = Vec::new();
    for plugin in scan_buildable_plugins(root)?.buildable {
        features.extend(qualified_plugin_build_features(&plugin.dir)?);
    }
    Ok(features)
}

fn build_command(path: &Path, root: &Path, excluded: &[String], features: &[String]) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(path).arg("build");
    if path == root && !excluded.is_empty() {
        command.arg("--workspace");
        for package in excluded {
            command.arg("--exclude").arg(package);
        }
    }
    for feature in features {
        command.arg("--features").arg(feature);
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
        let none = &[] as &[String];
        let bare = ["local-stt".to_string()];
        let qualified = ["qol-voice/local-stt".to_string()];
        let cases = [
            (
                root,
                excluded.as_slice(),
                none,
                "build --workspace --exclude keyremap",
                "workspace-root build scopes out host-incompatible plugins",
            ),
            (
                root,
                none,
                none,
                "build",
                "no excludes (e.g. on macOS) leaves the root build untouched",
            ),
            (
                Path::new("/a/b/ws/plugins/plugin-x"),
                excluded.as_slice(),
                none,
                "build",
                "non-root crate builds stay plain",
            ),
            (
                Path::new("/a/b/ws/plugins/plugin-x"),
                excluded.as_slice(),
                bare.as_slice(),
                "build --features local-stt",
                "a plugin's declared features reach its own build",
            ),
            (
                root,
                excluded.as_slice(),
                qualified.as_slice(),
                "build --workspace --exclude keyremap --features qol-voice/local-stt",
                "workspace-root builds qualify declared features by package",
            ),
        ];
        for (path, excl, features, want, label) in cases {
            assert_eq!(
                args_of(&build_command(path, root, excl, features)).as_str(),
                want,
                "{label}"
            );
        }
    }
}
