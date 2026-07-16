pub(crate) use qol_dev_env::{registry, resources};

use crate::commands::emu;
use crate::workspace::repo_root;
use anyhow::{Context, Result};
use qol_dev_env::Inventory;
use qol_dev_env::{EnvironmentDefinition, LocalConfig, ResolvedEnvironment};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn discover() -> Result<Vec<ResolvedEnvironment>> {
    let root = repo_root()?;
    discover_in(&root)
}

pub(crate) fn discover_in(root: &Path) -> Result<Vec<ResolvedEnvironment>> {
    let definitions = registry::discover_definitions(root)?;
    let (_, config) = local_config_in(root)?;
    registry::resolve_definitions(definitions, &config, backend_supported)
}

pub(crate) fn local_config_in(root: &Path) -> Result<(PathBuf, LocalConfig)> {
    let config_path = config_path().context("could not determine dev environment config path")?;
    let config = with_defaults_in(registry::load_local_config(&config_path)?, root);
    Ok((config_path, config))
}

pub(crate) fn find(id: &str) -> Result<Option<ResolvedEnvironment>> {
    Ok(discover()?
        .into_iter()
        .find(|environment| environment.definition.id == id))
}

pub(crate) fn find_in(root: &Path, id: &str) -> Result<Option<ResolvedEnvironment>> {
    Ok(discover_in(root)?
        .into_iter()
        .find(|environment| environment.definition.id == id))
}

pub(crate) fn snapshot_in(root: &Path) -> Result<Inventory> {
    let environments = discover_in(root)?;
    Ok(qol_dev_env::scan_inventory(&environments))
}

pub(crate) fn host_capacity(run_root: &std::path::Path) -> resources::HostCapacity {
    resources::host_capacity(
        run_root,
        crate::host_facade::available_memory_mb(),
        crate::host_facade::available_cpus(),
    )
}

pub(crate) fn reconcile_resources() -> Result<resources::ReservedResources> {
    emu::image_import::reconcile_leased_imports()?;
    let (reserved, diagnostics) = resources::reconcile()?;
    for diagnostic in diagnostics {
        eprintln!("warning: {diagnostic}");
    }
    Ok(reserved)
}

pub(crate) fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|root| root.join("qol").join("dev-envs.toml"))
}

pub(crate) fn clear_host_session(command: &mut Command) {
    qol_dev_env::clear_host_session(command);
}

pub(crate) fn run_owner(task: &str, state: &str) -> Value {
    let worktree = repo_root()
        .ok()
        .and_then(|root| root.canonicalize().ok().or(Some(root)));
    run_owner_value(task, state, worktree.as_deref())
}

pub(crate) fn run_owner_in(task: &str, state: &str, worktree: &Path) -> Value {
    run_owner_value(task, state, Some(worktree))
}

fn run_owner_value(task: &str, state: &str, worktree: Option<&Path>) -> Value {
    json!({
        "pid": std::process::id(),
        "process_identity": qol_process::process_identity(std::process::id()).ok(),
        "state": state,
        "worktree": worktree,
        "task": task,
    })
}

fn backend_supported(definition: &EnvironmentDefinition) -> std::result::Result<(), String> {
    let spec = emu::BackendSpec::from_manifest(
        &definition.backend,
        &definition.image.kind,
        definition.image.arch.as_deref(),
        definition.image.firmware.as_deref(),
        definition
            .capabilities
            .get("acceleration")
            .map(String::as_str),
    )?;
    emu::resolve_backend(spec).map(|_| ())
}

fn with_defaults_in(mut config: LocalConfig, root: &Path) -> LocalConfig {
    if config.image_root.is_none() {
        config.image_root = emu::emu_dir();
    }
    if config.run_root.is_none() {
        config.run_root = Some(root.join("target/qol-env"));
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_owner_identifies_the_worktree_and_task() {
        let owner = run_owner("qol-shot-capture", "running");

        assert_eq!(owner["pid"], std::process::id());
        assert_eq!(owner["state"], "running");
        assert_eq!(owner["task"], "qol-shot-capture");
        assert!(owner["worktree"]
            .as_str()
            .is_some_and(|path| !path.is_empty()));
    }

    #[test]
    fn explicit_run_owner_uses_the_build_checkout() {
        let worktree = Path::new("/qol/worktrees/shot-speed");
        let owner = run_owner_in("qol-shot-capture", "running", worktree);

        assert_eq!(owner["worktree"], worktree.to_string_lossy().as_ref());
    }
}
