use crate::progress::{print_hint, print_title, run_cargo_step, step_label, StepKind};
use crate::workspace::{exe_name, record_default_workspace, repo_root};
use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use toml::Value as TomlValue;

const CARGO_LOCK_DRIVER_CMD: &str = ".githooks/cargo-lock-merge %O %A %B %P";
const GIT_HOOKS_PATH: &str = ".githooks";

pub(crate) fn cmd_setup(args: &[OsString], verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol setup");
    }
    let root = repo_root()?;
    run_setup(&root, verbose)?;
    record_default_workspace(&root)?;
    step_label("root", StepKind::Success, &root.display().to_string());
    Ok(())
}

pub(crate) fn run_setup(root: &Path, verbose: bool) -> Result<()> {
    run_setup_with_install(root, verbose, true)
}

pub(crate) fn run_setup_with_install(root: &Path, verbose: bool, install: bool) -> Result<()> {
    let package = root.join("tools").join("qol-cli");
    let target = installed_qol_path()?;
    print_title("qol setup");
    print_hint(verbose);
    register_cargo_lock_driver(root).context("failed to configure Cargo.lock merge driver")?;
    step_label("merge", StepKind::Success, "Cargo.lock auto-resolve");
    register_git_hooks(root).context("failed to configure Git hooks")?;
    step_label("hooks", StepKind::Success, GIT_HOOKS_PATH);
    if !install {
        step_label("install", StepKind::Info, "skipped (worktree target)");
        return Ok(());
    }
    let target_display = target.display().to_string();
    if install_is_current(root, &package, &target)? {
        step_label("current", StepKind::Success, &target_display);
        return Ok(());
    }
    let manifest = package.join("Cargo.toml");
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--workspace")
        .args(["--bin", "qol", "--locked", "--offline"]);
    for feature in qol_dev_build::dev_feature_flags(root).map_err(anyhow::Error::msg)? {
        command.arg("--features").arg(feature);
    }
    qol_dev_build::configure_dev_cargo(&mut command);
    let artifacts = run_cargo_step(
        "install",
        StepKind::Pending,
        &target_display,
        &mut command,
        verbose,
    )?;
    let built = qol_dev_build::cargo_build::select_binary_executable(&artifacts, &manifest, "qol")?;
    replace_binary(&built, &target)?;
    step_label("ready", StepKind::Success, &target_display);
    Ok(())
}

fn replace_binary(built: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().ok_or_else(|| {
        anyhow!(
            "install target has no parent directory: {}",
            target.display()
        )
    })?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let staged = parent.join(exe_name("qol-staged"));
    fs::copy(built, &staged).with_context(|| format!("failed to stage {}", built.display()))?;
    fs::rename(&staged, target).with_context(|| format!("failed to replace {}", target.display()))
}

pub(crate) fn installed_qol_path() -> Result<PathBuf> {
    Ok(cargo_bin_dir()?.join(exe_name("qol")))
}

fn register_cargo_lock_driver(root: &Path) -> Result<()> {
    git_config(
        root,
        "merge.cargo-lock.name",
        "Cargo.lock auto-resolve (regenerate from manifests)",
    )?;
    git_config(root, "merge.cargo-lock.driver", CARGO_LOCK_DRIVER_CMD)?;
    git_config_unset(root, "merge.lockfile.driver");
    git_config_unset(root, "merge.lockfile.name");
    Ok(())
}

fn register_git_hooks(root: &Path) -> Result<()> {
    git_config(root, "core.hooksPath", GIT_HOOKS_PATH)
}

fn git_config(root: &Path, key: &str, value: &str) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", key, value])
        .status()
        .with_context(|| format!("failed to run git config {key}"))?;
    if !status.success() {
        bail!("git config {key} exited with {status}");
    }
    Ok(())
}

fn git_config_unset(root: &Path, key: &str) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--unset", key])
        .status();
}

fn install_is_current(root: &Path, package: &Path, target: &Path) -> Result<bool> {
    if !target.is_file() {
        return Ok(false);
    }
    let installed = fs::metadata(target)
        .with_context(|| format!("failed to inspect {}", target.display()))?
        .modified()
        .with_context(|| format!("failed to read modified time for {}", target.display()))?;
    Ok(installed >= newest_setup_input(root, package)?)
}

pub(crate) fn newest_setup_input(root: &Path, package: &Path) -> Result<SystemTime> {
    let mut newest = UNIX_EPOCH;
    record_mtime(&package.join("Cargo.toml"), &mut newest)?;
    record_mtime(&root.join("Cargo.lock"), &mut newest)?;
    record_tree_mtime(&package.join("src"), &mut newest)?;
    for dependency in workspace_path_dependencies(root, package)? {
        record_mtime(&dependency.join("Cargo.toml"), &mut newest)?;
        record_tree_mtime(&dependency.join("src"), &mut newest)?;
    }
    Ok(newest)
}

fn workspace_path_dependencies(root: &Path, package: &Path) -> Result<Vec<PathBuf>> {
    let manifest_content = fs::read_to_string(package.join("Cargo.toml"))
        .with_context(|| format!("failed to read {}", package.join("Cargo.toml").display()))?;
    let manifest: TomlValue = toml::from_str(&manifest_content)
        .with_context(|| format!("failed to parse {}", package.join("Cargo.toml").display()))?;
    let workspace_manifest_content = fs::read_to_string(root.join("Cargo.toml"))
        .with_context(|| format!("failed to read {}", root.join("Cargo.toml").display()))?;
    let workspace_manifest: TomlValue = toml::from_str(&workspace_manifest_content)
        .with_context(|| format!("failed to parse {}", root.join("Cargo.toml").display()))?;
    let workspace_dependencies = workspace_manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(TomlValue::as_table);
    let mut paths = Vec::new();
    if let Some(dependencies) = manifest.get("dependencies").and_then(TomlValue::as_table) {
        for (name, value) in dependencies {
            let path = match value.as_table() {
                Some(entry) if entry.contains_key("path") => entry
                    .get("path")
                    .and_then(TomlValue::as_str)
                    .map(|path| package.join(path)),
                Some(entry)
                    if entry.get("workspace").and_then(TomlValue::as_bool) == Some(true) =>
                {
                    workspace_dependencies
                        .and_then(|workspace| workspace.get(name))
                        .and_then(|value| value.get("path"))
                        .and_then(TomlValue::as_str)
                        .map(|path| root.join(path))
                }
                _ => None,
            };
            if let Some(path) = path {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

fn record_tree_mtime(path: &Path, newest: &mut SystemTime) -> Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            record_tree_mtime(&path, newest)?;
            continue;
        }
        record_mtime(&path, newest)?;
    }
    Ok(())
}

fn record_mtime(path: &Path, newest: &mut SystemTime) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let modified = fs::metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read modified time for {}", path.display()))?;
    if modified > *newest {
        *newest = modified;
    }
    Ok(())
}

fn cargo_bin_dir() -> Result<PathBuf> {
    Ok(cargo_home()?.join("bin"))
}

fn cargo_home() -> Result<PathBuf> {
    if let Some(path) = env::var_os("CARGO_HOME") {
        return Ok(PathBuf::from(path));
    }
    let home = crate::host_facade::home_dir()
        .ok_or_else(|| anyhow!("CARGO_HOME is not set and no home directory was found"))?;
    Ok(home.join(".cargo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} exited with {status}");
    }

    fn configured_hooks_path(root: &Path) -> String {
        let configured = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["config", "--get", "core.hooksPath"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&configured.stdout)
            .trim()
            .to_string()
    }

    #[test]
    fn setup_registers_the_repository_hooks_before_any_install_work_can_fail() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "--quiet"]);
        let package = repo.path().join("tools/qol-cli");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("Cargo.toml"),
            "[package]\nversion = \"0.0.0\"\n",
        )
        .unwrap();

        let _ = run_setup(repo.path(), false);

        assert_eq!(configured_hooks_path(repo.path()), GIT_HOOKS_PATH);
    }

    #[test]
    fn newest_setup_input_tracks_workspace_path_dependency_sources() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("tools/qol-cli");
        fs::create_dir_all(package.join("src")).unwrap();
        fs::create_dir_all(root.path().join("libs/qol-lib/src")).unwrap();
        fs::write(
            package.join("Cargo.toml"),
            "[package]\nversion = \"0.0.0\"\n[dependencies]\nqol-lib.workspace = true\n",
        )
        .unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\n[workspace.dependencies]\nqol-lib = { path = \"libs/qol-lib\" }\n",
        )
        .unwrap();
        fs::write(root.path().join("Cargo.lock"), "").unwrap();
        fs::write(package.join("src/lib.rs"), "").unwrap();
        fs::write(
            root.path().join("libs/qol-lib/Cargo.toml"),
            "[package]\nname = \"qol-lib\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        fs::write(root.path().join("libs/qol-lib/src/lib.rs"), "").unwrap();

        let newest = newest_setup_input(root.path(), &package).unwrap();
        let dependency_src = fs::metadata(root.path().join("libs/qol-lib/src/lib.rs"))
            .unwrap()
            .modified()
            .unwrap();

        assert_eq!(newest, dependency_src);
    }

    #[test]
    fn setup_fails_when_lockfile_merge_driver_cannot_be_registered() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("tools/qol-cli");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("Cargo.toml"),
            "[package]\nversion = \"0.0.0\"\n",
        )
        .unwrap();

        let error = run_setup(root.path(), false).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failed to configure Cargo.lock merge driver"),
            "unexpected setup error: {error:#}"
        );
    }
}
