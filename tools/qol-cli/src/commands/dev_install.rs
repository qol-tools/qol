use crate::progress::{print_title, run_cargo_step, step_label, StepKind};
use crate::workspace::{
    cargo_build_command, exe_name, qualified_plugin_build_features, repo_root,
    scan_buildable_plugins,
};
use anyhow::{bail, Context, Result};
use qol_plugin_api::manifest::PluginManifest;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let (worktree, plugin_filter) = parse_args(args)?;
    let root = match worktree {
        Some(path) => path,
        None => repo_root()?,
    };
    print_title("qol dev install");

    let scan = scan_buildable_plugins(&root)?;
    let mut targets = Vec::new();
    for buildable in scan.buildable {
        let manifest_path = buildable.dir.join("plugin.toml");
        if !manifest_path.exists() {
            continue;
        }
        let manifest = PluginManifest::load_and_validate(&manifest_path)?;
        if !manifest.plugin.auto_install_host {
            continue;
        }
        if let Some(filter) = &plugin_filter {
            if manifest.plugin.id.as_ref().map(|id| id.as_str()) != Some(filter.as_str()) {
                continue;
            }
        }
        targets.push((buildable, manifest));
    }

    if targets.is_empty() {
        step_label(
            "scan",
            StepKind::Info,
            "no plugins opted in to host install",
        );
        step_label(
            "hint",
            StepKind::Info,
            "add `auto_install_host = true` under [plugin] in plugin.toml",
        );
        return Ok(());
    }

    let mut build = cargo_build_command(&root, &["build", "--locked"]);
    for (buildable, _) in &targets {
        build.arg("-p").arg(&buildable.package_name);
        for feature in qualified_plugin_build_features(&buildable.dir)? {
            build.arg("--features").arg(feature);
        }
    }
    run_cargo_step(
        "build",
        StepKind::Pending,
        "debug binaries",
        &mut build,
        verbose,
    )?;

    let Some(config_dir) = qol_config::config_dir() else {
        bail!("no user config directory is available");
    };
    let dest_root = config_dir.join("plugins");
    let mut installed = Vec::new();
    for (buildable, manifest) in &targets {
        let plugin_id = manifest
            .plugin
            .id
            .as_ref()
            .context("plugin.toml declares no id")?
            .as_str()
            .to_string();
        let command = manifest
            .runtime
            .as_ref()
            .context("plugin.toml declares no runtime command")?
            .command
            .clone();
        let binary_source = root.join("target").join("debug").join(exe_name(&command));
        let files = install_plugin(
            &buildable.dir,
            &plugin_id,
            &command,
            &binary_source,
            &dest_root,
        )?;
        installed.push((plugin_id, files.len()));
    }
    for (plugin_id, count) in &installed {
        step_label(
            "installed",
            StepKind::Success,
            &format!("{plugin_id} ({count} files) -> {}", dest_root.display()),
        );
    }
    step_label(
        "hint",
        StepKind::Info,
        "restart qol-tray to load the installed plugin(s)",
    );
    Ok(())
}

pub(crate) fn install_plugin(
    plugin_root: &Path,
    plugin_id: &str,
    command: &str,
    binary_source: &Path,
    dest_root: &Path,
) -> Result<Vec<PathBuf>> {
    let dest = dest_root.join(plugin_id);
    fs::create_dir_all(&dest)?;
    let mut installed = Vec::new();
    for file in qol_workspace::plugin_delivery_files(plugin_root, &[command, &exe_name(command)])? {
        let target = dest.join(&file.relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&file.source, &target).with_context(|| {
            format!(
                "failed to copy {} to {}",
                file.source.display(),
                target.display()
            )
        })?;
        installed.push(target);
    }
    let binary_target = dest.join(command);
    fs::copy(binary_source, &binary_target).with_context(|| {
        format!(
            "failed to copy built binary {} to {}",
            binary_source.display(),
            binary_target.display()
        )
    })?;
    installed.push(binary_target);
    Ok(installed)
}

fn parse_args(args: &[OsString]) -> Result<(Option<PathBuf>, Option<String>)> {
    let mut worktree = None;
    let mut plugin_filter = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--plugin") => {
                let value = iter
                    .next()
                    .context("--plugin requires a plugin id")?
                    .to_str()
                    .context("non-UTF-8 plugin id")?
                    .to_string();
                plugin_filter = Some(value);
            }
            Some(other) if other.starts_with("--") => {
                bail!("unknown argument `{other}` for qol dev install")
            }
            Some(path) => {
                if worktree.is_some() {
                    bail!("qol dev install accepts at most one worktree path");
                }
                worktree = Some(PathBuf::from(path));
            }
            None => bail!("non-UTF-8 argument for qol dev install"),
        }
    }
    Ok((worktree, plugin_filter))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_plugin(root: &Path) {
        fs::create_dir_all(root.join("ui")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("plugin.toml"), "[plugin]").unwrap();
        fs::write(root.join("ui/panel.js"), "// ui").unwrap();
        fs::write(root.join("src/lib.rs"), "// not delivered").unwrap();
        fs::write(root.join("target/debug/placeholder"), "// not delivered").unwrap();
        fs::write(root.join(".git/HEAD"), "ref").unwrap();
        fs::write(root.join("diff-viewer"), "// excluded root binary").unwrap();
    }

    #[test]
    fn install_plugin_copies_delivery_files_and_binary() {
        let root = std::env::temp_dir().join(format!("qol-dev-install-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let plugin_root = root.join("plugin-diff-viewer");
        let dest_root = root.join("host/plugins");
        let binary = root.join("bin/diff-viewer");
        fake_plugin(&plugin_root);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "ELF").unwrap();

        let installed = install_plugin(
            &plugin_root,
            "plugin-diff-viewer",
            "diff-viewer",
            &binary,
            &dest_root,
        )
        .unwrap();

        assert!(installed.contains(&dest_root.join("plugin-diff-viewer/plugin.toml")));
        assert!(installed.contains(&dest_root.join("plugin-diff-viewer/ui/panel.js")));
        assert!(installed.contains(&dest_root.join("plugin-diff-viewer/diff-viewer")));
        assert!(!dest_root.join("plugin-diff-viewer/src/lib.rs").exists());
        assert!(!dest_root.join("plugin-diff-viewer/target").exists());
        assert!(!dest_root.join("plugin-diff-viewer/.git").exists());
        assert!(!dest_root.join("plugin-diff-viewer/diff-viewer").is_dir());
        assert_eq!(
            fs::read_to_string(dest_root.join("plugin-diff-viewer/ui/panel.js")).unwrap(),
            "// ui"
        );
        assert_eq!(
            fs::read_to_string(dest_root.join("plugin-diff-viewer/diff-viewer")).unwrap(),
            "ELF"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_args_accepts_worktree_and_filter() {
        let args: Vec<OsString> = vec![
            "/tmp/wt".into(),
            "--plugin".into(),
            "plugin-diff-viewer".into(),
        ];
        let (worktree, filter) = parse_args(&args).unwrap();
        assert_eq!(worktree, Some(PathBuf::from("/tmp/wt")));
        assert_eq!(filter.as_deref(), Some("plugin-diff-viewer"));
        assert!(parse_args(&[OsString::from("--nope")]).is_err());
    }
}
