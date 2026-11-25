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
const SHARED_BUILD_DIR: &str = ".cargo-build";

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
    let package = root.join("tools").join("cli");
    let target = installed_qol_path()?;
    print_title("qol setup");
    print_hint(verbose);
    register_cargo_lock_driver(root).context("failed to configure Cargo.lock merge driver")?;
    step_label("merge", StepKind::Success, "Cargo.lock auto-resolve");
    register_git_hooks(root).context("failed to configure Git hooks")?;
    step_label("hooks", StepKind::Success, GIT_HOOKS_PATH);
    match ensure_shared_build_dir(root) {
        Ok(build_dir) => step_label("build-dir", StepKind::Success, &build_dir),
        Err(error) => step_label("build-dir", StepKind::Info, &format!("not set: {error:#}")),
    }
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

fn ensure_shared_build_dir(root: &Path) -> Result<String> {
    ensure_build_dir(env::var_os("CARGO_BUILD_BUILD_DIR"), root, &cargo_home()?)
}

fn ensure_build_dir(from_env: Option<OsString>, root: &Path, cargo_home: &Path) -> Result<String> {
    if let Some(value) = from_env {
        return Ok(value.to_string_lossy().into_owned());
    }
    if let Some(existing) = configured_build_dir(root, cargo_home)? {
        return Ok(existing);
    }
    let clone = main_clone(root)?;
    let workspace_home = clone
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent folder", clone.display()))?;
    let cargo_dir = workspace_home.join(".cargo");
    if same_dir(&cargo_dir, cargo_home)
        || crate::host_facade::home_dir().is_some_and(|home| same_dir(&home, workspace_home))
    {
        bail!(
            "{} would set it for every Rust project; set build.build-dir by hand",
            cargo_dir.display()
        );
    }
    if !canonical(root).starts_with(canonical(workspace_home)) {
        bail!(
            "{} is outside {}, so Cargo would not read a config written there",
            root.display(),
            workspace_home.display()
        );
    }
    let build_dir = workspace_home.join(SHARED_BUILD_DIR);
    write_build_dir(&config_file_in(&cargo_dir), &build_dir)?;
    Ok(build_dir.display().to_string())
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn same_dir(left: &Path, right: &Path) -> bool {
    canonical(left) == canonical(right)
}

fn configured_build_dir(root: &Path, cargo_home: &Path) -> Result<Option<String>> {
    let config_dirs = root
        .ancestors()
        .map(|dir| dir.join(".cargo"))
        .chain(std::iter::once(cargo_home.to_path_buf()));
    for dir in config_dirs {
        if let Some(found) = build_dir_in(&config_file_in(&dir))? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn config_file_in(cargo_dir: &Path) -> PathBuf {
    let legacy = cargo_dir.join("config");
    if legacy.is_file() {
        legacy
    } else {
        cargo_dir.join("config.toml")
    }
}

fn build_dir_in(config: &Path) -> Result<Option<String>> {
    let text = match fs::read_to_string(config) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config.display()))
        }
    };
    let parsed: TomlValue =
        toml::from_str(&text).with_context(|| format!("failed to parse {}", config.display()))?;
    Ok(parsed
        .get("build")
        .and_then(|build| build.get("build-dir"))
        .and_then(TomlValue::as_str)
        .map(str::to_owned))
}

fn main_clone(root: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .context("failed to run git rev-parse --git-common-dir")?;
    if !output.status.success() {
        bail!(
            "git rev-parse --git-common-dir exited with {}",
            output.status
        );
    }
    let common_dir: PathBuf = Path::new(String::from_utf8_lossy(&output.stdout).trim())
        .components()
        .collect();
    common_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("{} has no parent folder", common_dir.display()))
}

fn write_build_dir(config: &Path, build_dir: &Path) -> Result<()> {
    let text = match fs::read_to_string(config) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config.display()))
        }
    };
    let mut document: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("failed to parse {}", config.display()))?;
    let build = document
        .entry("build")
        .or_insert(toml_edit::table())
        .as_table_like_mut()
        .ok_or_else(|| anyhow!("`build` in {} is not a table", config.display()))?;
    build.insert(
        "build-dir",
        toml_edit::value(build_dir.to_string_lossy().as_ref()),
    );
    let parent = config
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent folder", config.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    fs::write(config, document.to_string())
        .with_context(|| format!("failed to write {}", config.display()))
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
    record_mtime(&root.join("Cargo.lock"), &mut newest)?;
    record_crate_mtime(package, &mut newest)?;
    for dependency in workspace_path_dependencies(root, package)? {
        record_crate_mtime(&dependency, &mut newest)?;
    }
    Ok(newest)
}

fn record_crate_mtime(crate_dir: &Path, newest: &mut SystemTime) -> Result<()> {
    record_mtime(&crate_dir.join("Cargo.toml"), newest)?;
    record_mtime(&crate_dir.join("build.rs"), newest)?;
    record_tree_mtime(&crate_dir.join("src"), newest)
}

const BUILD_DEPENDENCY_SECTIONS: [&str; 2] = ["dependencies", "build-dependencies"];

fn workspace_path_dependencies(root: &Path, package: &Path) -> Result<Vec<PathBuf>> {
    let workspace_manifest = read_manifest(&root.join("Cargo.toml"))?;
    let workspace_dependencies = workspace_manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(TomlValue::as_table);
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    let mut pending = vec![package.to_path_buf()];
    while let Some(crate_dir) = pending.pop() {
        let manifest = read_manifest(&crate_dir.join("Cargo.toml"))?;
        for (name, value) in dependency_entries(&manifest) {
            let path = match value.as_table() {
                Some(entry) if entry.contains_key("path") => entry
                    .get("path")
                    .and_then(TomlValue::as_str)
                    .map(|path| crate_dir.join(path)),
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
            let Some(path) = path else {
                continue;
            };
            let path = fs::canonicalize(&path).unwrap_or(path);
            if !path.join("Cargo.toml").is_file() || !seen.insert(path.clone()) {
                continue;
            }
            pending.push(path.clone());
            paths.push(path);
        }
    }
    Ok(paths)
}

fn read_manifest(path: &Path) -> Result<TomlValue> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

fn dependency_entries(manifest: &TomlValue) -> Vec<(&String, &TomlValue)> {
    let targets = manifest
        .get("target")
        .and_then(TomlValue::as_table)
        .into_iter()
        .flat_map(|targets| targets.values());
    std::iter::once(manifest)
        .chain(targets)
        .flat_map(|scope| {
            BUILD_DEPENDENCY_SECTIONS
                .iter()
                .filter_map(|section| scope.get(*section).and_then(TomlValue::as_table))
        })
        .flat_map(|table| table.iter())
        .collect()
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
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("qol");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        let package = repo.join("tools/cli");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("Cargo.toml"),
            "[package]\nversion = \"0.0.0\"\n",
        )
        .unwrap();

        let _ = run_setup(&repo, false);

        assert_eq!(configured_hooks_path(&repo), GIT_HOOKS_PATH);
    }

    #[test]
    fn newest_setup_input_tracks_workspace_path_dependency_sources() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("tools/cli");
        fs::create_dir_all(package.join("src")).unwrap();
        fs::create_dir_all(root.path().join("libs/lib/src")).unwrap();
        fs::write(
            package.join("Cargo.toml"),
            "[package]\nversion = \"0.0.0\"\n[dependencies]\nqol-lib.workspace = true\n",
        )
        .unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\n[workspace.dependencies]\nqol-lib = { path = \"libs/lib\" }\n",
        )
        .unwrap();
        fs::write(root.path().join("Cargo.lock"), "").unwrap();
        fs::write(package.join("src/lib.rs"), "").unwrap();
        fs::write(
            root.path().join("libs/lib/Cargo.toml"),
            "[package]\nname = \"qol-lib\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        fs::write(root.path().join("libs/lib/src/lib.rs"), "").unwrap();

        let newest = newest_setup_input(root.path(), &package).unwrap();
        let dependency_src = fs::metadata(root.path().join("libs/lib/src/lib.rs"))
            .unwrap()
            .modified()
            .unwrap();

        assert_eq!(newest, dependency_src);
    }

    #[test]
    fn newest_setup_input_tracks_dependencies_of_dependencies() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("tools/cli");
        for dir in [
            "tools/cli/src",
            "libs/lib/src",
            "libs/leaf/src",
            "libs/build/src",
        ] {
            fs::create_dir_all(root.path().join(dir)).unwrap();
        }
        fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\n[workspace.dependencies]\nqol-lib = { path = \"libs/lib\" }\n",
        )
        .unwrap();
        fs::write(
            package.join("Cargo.toml"),
            "[package]\nversion = \"0.0.0\"\n[dependencies]\nqol-lib.workspace = true\n",
        )
        .unwrap();
        fs::write(
            root.path().join("libs/lib/Cargo.toml"),
            "[package]\nname = \"qol-lib\"\nversion = \"0.0.0\"\n[target.'cfg(unix)'.dependencies]\nqol-leaf = { path = \"../leaf\" }\n[build-dependencies]\nqol-build = { path = \"../build\" }\n",
        )
        .unwrap();
        for name in ["leaf", "build"] {
            fs::write(
                root.path().join(format!("libs/{name}/Cargo.toml")),
                format!("[package]\nname = \"qol-{name}\"\nversion = \"0.0.0\"\n"),
            )
            .unwrap();
        }
        for file in [
            "tools/cli/src/lib.rs",
            "libs/lib/src/lib.rs",
            "libs/leaf/src/lib.rs",
            "libs/build/src/lib.rs",
        ] {
            fs::write(root.path().join(file), "").unwrap();
        }
        let base = fs::metadata(package.join("Cargo.toml"))
            .unwrap()
            .modified()
            .unwrap();
        for (offset, file) in [(60, "libs/leaf/src/lib.rs"), (120, "libs/build/src/lib.rs")] {
            let edited = base + std::time::Duration::from_secs(offset);
            fs::File::options()
                .write(true)
                .open(root.path().join(file))
                .unwrap()
                .set_modified(edited)
                .unwrap();
            assert_eq!(
                newest_setup_input(root.path(), &package).unwrap(),
                edited,
                "{file}"
            );
        }
    }

    fn clone_in(home: &Path) -> PathBuf {
        let clone = home.join("qol");
        fs::create_dir_all(&clone).unwrap();
        git(&clone, &["init", "--quiet"]);
        clone
    }

    #[test]
    fn shared_build_dir_is_written_beside_the_clone_once() {
        let home = tempfile::tempdir().unwrap();
        let clone = clone_in(home.path());
        let cargo_home = home.path().join("cargo-home");
        let config = home.path().join(".cargo/config.toml");

        let build_dir = ensure_build_dir(None, &clone, &cargo_home).unwrap();

        assert_eq!(Path::new(&build_dir).file_name().unwrap(), SHARED_BUILD_DIR);
        assert_eq!(
            Path::new(&build_dir)
                .parent()
                .unwrap()
                .canonicalize()
                .unwrap(),
            home.path().canonicalize().unwrap()
        );
        let written = fs::read_to_string(&config).unwrap();
        assert_eq!(build_dir_in(&config).unwrap(), Some(build_dir.clone()));
        assert_eq!(
            ensure_build_dir(None, &clone, &cargo_home).unwrap(),
            build_dir
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), written);
    }

    #[test]
    fn an_existing_build_dir_is_kept() {
        let home = tempfile::tempdir().unwrap();
        let clone = clone_in(home.path());
        let cargo_home = home.path().join("cargo-home");
        fs::create_dir_all(&cargo_home).unwrap();
        fs::write(
            cargo_home.join("config.toml"),
            "[build]\nbuild-dir = \"/elsewhere\"\n",
        )
        .unwrap();

        assert_eq!(
            ensure_build_dir(None, &clone, &cargo_home).unwrap(),
            "/elsewhere"
        );
        assert!(!home.path().join(".cargo").exists());
    }

    #[test]
    fn a_build_dir_from_the_environment_is_kept() {
        let home = tempfile::tempdir().unwrap();
        let clone = clone_in(home.path());
        let cargo_home = home.path().join("cargo-home");

        assert_eq!(
            ensure_build_dir(Some(OsString::from("/from-env")), &clone, &cargo_home).unwrap(),
            "/from-env"
        );
        assert!(!home.path().join(".cargo").exists());
    }

    #[test]
    fn a_legacy_config_file_gets_the_build_dir() {
        let home = tempfile::tempdir().unwrap();
        let clone = clone_in(home.path());
        let cargo_home = home.path().join("cargo-home");
        let legacy = home.path().join(".cargo/config");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, "[build]\njobs = 8\n").unwrap();

        let build_dir = ensure_build_dir(None, &clone, &cargo_home).unwrap();

        assert_eq!(build_dir_in(&legacy).unwrap(), Some(build_dir));
        assert!(!home.path().join(".cargo/config.toml").exists());
    }

    #[test]
    fn a_clone_whose_parent_holds_cargo_home_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let clone = clone_in(home.path());
        let cargo_home = home.path().join(".cargo");
        fs::create_dir_all(&cargo_home).unwrap();

        let error = ensure_build_dir(None, &clone, &cargo_home).unwrap_err();

        assert!(error.to_string().contains("every Rust project"), "{error}");
        assert!(!cargo_home.join("config.toml").exists());
    }

    #[test]
    fn a_tree_outside_the_main_clone_parent_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let clone = clone_in(home.path());
        git(
            &clone,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "--allow-empty",
                "--quiet",
                "-m",
                "init",
            ],
        );
        let elsewhere = tempfile::tempdir().unwrap();
        let worktree = elsewhere.path().join("qol");
        git(
            &clone,
            &["worktree", "add", "--quiet", worktree.to_str().unwrap()],
        );
        let cargo_home = home.path().join("cargo-home");

        let error = ensure_build_dir(None, &worktree, &cargo_home).unwrap_err();

        assert!(error.to_string().contains("is outside"), "{error}");
        assert!(!home.path().join(".cargo").exists());
    }

    #[test]
    fn shared_build_dir_keeps_the_rest_of_an_existing_config() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".cargo/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "# linker\n[target.x86_64-unknown-linux-gnu]\nlinker = \"clang\"\n\n[build]\njobs = 8\n").unwrap();
        let build_dir = home.path().join(SHARED_BUILD_DIR);

        write_build_dir(&config, &build_dir).unwrap();

        let written = fs::read_to_string(&config).unwrap();
        assert!(written.contains("# linker"), "{written}");
        assert!(written.contains("linker = \"clang\""), "{written}");
        assert!(written.contains("jobs = 8"), "{written}");
        assert_eq!(
            build_dir_in(&config).unwrap(),
            Some(build_dir.display().to_string())
        );
    }

    #[test]
    fn the_main_clone_of_a_worktree_is_the_clone_it_came_from() {
        let home = tempfile::tempdir().unwrap();
        let clone = home.path().join("qol");
        fs::create_dir_all(&clone).unwrap();
        git(&clone, &["init", "--quiet"]);
        git(
            &clone,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "--allow-empty",
                "--quiet",
                "-m",
                "init",
            ],
        );
        let worktree = home.path().join("worktrees/topic/qol");
        git(
            &clone,
            &["worktree", "add", "--quiet", worktree.to_str().unwrap()],
        );

        assert_eq!(
            main_clone(&worktree).unwrap().canonicalize().unwrap(),
            clone.canonicalize().unwrap()
        );
    }

    #[test]
    fn setup_fails_when_lockfile_merge_driver_cannot_be_registered() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("tools/cli");
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
