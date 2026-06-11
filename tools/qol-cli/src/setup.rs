use crate::progress::{print_hint, print_title, run_step, step_label, StepKind};
use crate::workspace::{exe_name, repo_root};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value as JsonValue;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use toml::Value as TomlValue;

pub(crate) fn cmd_setup(args: &[OsString], verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol setup");
    }
    let root = repo_root()?;
    let package = root.join("tools").join("qol-cli");
    let target = installed_qol_path()?;
    let version = package_version(&package)?;
    print_title("qol setup");
    print_hint(verbose);
    let target_display = target.display().to_string();
    if install_is_current(&root, &package, &target, &version)? {
        step_label("current", StepKind::Success, &target_display);
        return Ok(());
    }
    let mut command = Command::new("cargo");
    command
        .arg("install")
        .arg("--path")
        .arg(package)
        .args(["--locked", "--force", "--debug"]);
    run_step(
        "install",
        StepKind::Pending,
        &target_display,
        &mut command,
        verbose,
    )?;
    step_label("ready", StepKind::Success, &target_display);
    Ok(())
}

pub(crate) fn installed_qol_path() -> Result<PathBuf> {
    Ok(cargo_bin_dir()?.join(exe_name("qol")))
}

fn install_is_current(root: &Path, package: &Path, target: &Path, version: &str) -> Result<bool> {
    if !target.is_file() {
        return Ok(false);
    }
    if !cargo_registry_has_install(package, version)? {
        return Ok(false);
    }
    let installed = fs::metadata(target)
        .with_context(|| format!("failed to inspect {}", target.display()))?
        .modified()
        .with_context(|| format!("failed to read modified time for {}", target.display()))?;
    Ok(installed >= newest_setup_input(root, package)?)
}

fn cargo_registry_has_install(package: &Path, version: &str) -> Result<bool> {
    let registry = cargo_home()?.join(".crates2.json");
    if !registry.is_file() {
        return Ok(false);
    }
    let content = fs::read_to_string(&registry)
        .with_context(|| format!("failed to read {}", registry.display()))?;
    let parsed: JsonValue = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", registry.display()))?;
    let installs = match parsed.get("installs").and_then(JsonValue::as_object) {
        Some(installs) => installs,
        None => return Ok(false),
    };
    let prefix = format!("qol {version} (path+file://");
    let package = normalized_path(package)?;
    for (id, metadata) in installs {
        if !id.starts_with(&prefix) {
            continue;
        }
        if !id.contains(&package) {
            continue;
        }
        if bins_include_qol(metadata) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn bins_include_qol(metadata: &JsonValue) -> bool {
    metadata
        .get("bins")
        .and_then(JsonValue::as_array)
        .is_some_and(|bins| bins.iter().any(|bin| bin.as_str() == Some("qol")))
}

fn package_version(package: &Path) -> Result<String> {
    let manifest = package.join("Cargo.toml");
    let content = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    let parsed: TomlValue = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    parsed
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(TomlValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{} has no package.version", manifest.display()))
}

fn newest_setup_input(root: &Path, package: &Path) -> Result<SystemTime> {
    let mut newest = UNIX_EPOCH;
    record_mtime(&package.join("Cargo.toml"), &mut newest)?;
    record_mtime(&root.join("Cargo.lock"), &mut newest)?;
    record_tree_mtime(&package.join("src"), &mut newest)?;
    Ok(newest)
}

fn record_tree_mtime(path: &Path, newest: &mut SystemTime) -> Result<()> {
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
    let home = home_dir()
        .ok_or_else(|| anyhow!("CARGO_HOME is not set and no home directory was found"))?;
    Ok(home.join(".cargo"))
}

fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(path) = env::var_os("USERPROFILE") {
            return Some(PathBuf::from(path));
        }
    }
    env::var_os("HOME").map(PathBuf::from)
}

fn normalized_path(path: &Path) -> Result<String> {
    let path = path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", path.display()))?;
    Ok(path.to_string_lossy().replace('\\', "/"))
}
