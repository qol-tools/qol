use super::{
    BinaryDependency, DaemonConfig, Dependencies, MenuItem, PluginManifest, RuntimeConfig,
    CURRENT_MANIFEST_VERSION,
};
use anyhow::{bail, Result};
use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path};

impl PluginManifest {
    pub fn validate(&self) -> Result<()> {
        validate_manifest_version(self.manifest_version)?;
        let action_ids = collect_menu_action_ids(&self.menu.items)?;
        validate_runtime_config(self.runtime.as_ref(), &action_ids.executable)?;
        validate_daemon_config(self.daemon.as_ref())?;
        validate_dependencies(self.dependencies.as_ref())?;
        Ok(())
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<()> {
        validate_command_name("runtime.command", &self.command)?;
        validate_runtime_actions(self.actions.as_ref())?;
        Ok(())
    }
}

impl DaemonConfig {
    pub fn validate(&self) -> Result<()> {
        validate_command_name("daemon.command", &self.command)?;
        validate_socket_config(self.socket.as_deref())?;
        Ok(())
    }
}

impl Dependencies {
    pub fn validate(&self) -> Result<()> {
        for binary in &self.binaries {
            binary.validate()?;
        }
        Ok(())
    }
}

impl BinaryDependency {
    pub fn validate(&self) -> Result<()> {
        validate_command_name("dependencies.binaries.name", &self.name)
    }
}

pub fn is_valid_action_id(action: &str) -> bool {
    !action.is_empty()
        && action.len() <= 64
        && !action.starts_with('-')
        && action
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

pub fn is_valid_command_basename(value: &str) -> bool {
    !value.trim().is_empty()
        && value.trim() == value
        && !value.contains('\0')
        && !value.starts_with('-')
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn validate_manifest_version(version: u32) -> Result<()> {
    if version == CURRENT_MANIFEST_VERSION {
        return Ok(());
    }

    bail!(
        "Unsupported manifest_version {} (expected {})",
        version,
        CURRENT_MANIFEST_VERSION
    )
}

fn validate_runtime_config(
    runtime: Option<&RuntimeConfig>,
    executable_action_ids: &BTreeSet<String>,
) -> Result<()> {
    let Some(runtime) = runtime else {
        return Ok(());
    };

    runtime.validate()?;
    validate_runtime_action_coverage(runtime.actions.as_ref(), executable_action_ids)
}

fn validate_daemon_config(daemon: Option<&DaemonConfig>) -> Result<()> {
    let Some(daemon) = daemon else {
        return Ok(());
    };

    daemon.validate()
}

fn validate_dependencies(dependencies: Option<&Dependencies>) -> Result<()> {
    let Some(dependencies) = dependencies else {
        return Ok(());
    };

    dependencies.validate()
}

#[derive(Default)]
struct MenuActionIds {
    all: BTreeSet<String>,
    executable: BTreeSet<String>,
}

fn collect_menu_action_ids(items: &[MenuItem]) -> Result<MenuActionIds> {
    let mut action_ids = MenuActionIds::default();
    collect_item_slice(items, &mut action_ids)?;
    Ok(action_ids)
}

fn collect_item_slice(items: &[MenuItem], action_ids: &mut MenuActionIds) -> Result<()> {
    for item in items {
        collect_menu_item(item, action_ids)?;
    }
    Ok(())
}

fn collect_menu_item(item: &MenuItem, action_ids: &mut MenuActionIds) -> Result<()> {
    match item {
        MenuItem::Action { id, .. } => collect_executable_action(id, action_ids),
        MenuItem::Checkbox { id, .. } => collect_checkbox_action(id, action_ids),
        MenuItem::Submenu { items, .. } => collect_item_slice(items, action_ids),
        MenuItem::Separator => Ok(()),
    }
}

fn collect_executable_action(id: &str, action_ids: &mut MenuActionIds) -> Result<()> {
    validate_menu_action_id(id, &mut action_ids.all)?;
    action_ids.executable.insert(id.to_string());
    Ok(())
}

fn collect_checkbox_action(id: &str, action_ids: &mut MenuActionIds) -> Result<()> {
    validate_menu_action_id(id, &mut action_ids.all)
}

fn validate_menu_action_id(id: &str, action_ids: &mut BTreeSet<String>) -> Result<()> {
    if !is_valid_action_id(id) {
        bail!("menu contains invalid action id {:?}", id);
    }

    if action_ids.insert(id.to_string()) {
        return Ok(());
    }

    bail!("menu contains duplicate action id {:?}", id)
}

fn validate_runtime_actions(actions: Option<&HashMap<String, Vec<String>>>) -> Result<()> {
    let Some(actions) = actions else {
        return Ok(());
    };

    if actions.is_empty() {
        bail!("runtime.actions cannot be empty when present");
    }

    for (action_id, args) in actions {
        validate_runtime_action(action_id, args)?;
    }
    Ok(())
}

fn validate_runtime_action(action_id: &str, args: &[String]) -> Result<()> {
    if !is_valid_action_id(action_id) {
        bail!("runtime.actions contains invalid action id {:?}", action_id);
    }

    validate_runtime_args(action_id, args)
}

fn validate_runtime_args(action_id: &str, args: &[String]) -> Result<()> {
    if args.iter().all(|arg| !arg.contains('\0')) {
        return Ok(());
    }

    bail!("runtime.actions for {:?} contains null bytes", action_id)
}

fn validate_runtime_action_coverage(
    actions: Option<&HashMap<String, Vec<String>>>,
    executable_action_ids: &BTreeSet<String>,
) -> Result<()> {
    let Some(actions) = actions else {
        return Ok(());
    };

    for action_id in executable_action_ids {
        if actions.contains_key(action_id) {
            continue;
        }
        bail!(
            "runtime.actions missing mapping for menu action {:?}",
            action_id
        );
    }
    Ok(())
}

fn validate_command_name(field: &str, value: &str) -> Result<()> {
    if is_valid_command_basename(value) {
        return Ok(());
    }

    bail!("{field} must contain only [A-Za-z0-9_-]")
}

fn validate_socket_config(socket: Option<&str>) -> Result<()> {
    let Some(socket) = socket else {
        return Ok(());
    };

    validate_absolute_socket_path(socket)
}

fn validate_absolute_socket_path(path_value: &str) -> Result<()> {
    validate_socket_not_empty(path_value)?;
    validate_socket_whitespace(path_value)?;
    validate_socket_bytes(path_value)?;
    validate_socket_path_shape(path_value)
}

fn validate_socket_not_empty(path_value: &str) -> Result<()> {
    if !path_value.trim().is_empty() {
        return Ok(());
    }

    bail!("daemon.socket cannot be empty")
}

fn validate_socket_whitespace(path_value: &str) -> Result<()> {
    if path_value.trim() == path_value {
        return Ok(());
    }

    bail!("daemon.socket cannot have leading or trailing whitespace")
}

fn validate_socket_bytes(path_value: &str) -> Result<()> {
    if !path_value.contains('\0') {
        return Ok(());
    }

    bail!("daemon.socket cannot contain null bytes")
}

fn validate_socket_path_shape(path_value: &str) -> Result<()> {
    let path = Path::new(path_value);
    ensure_absolute_socket_path(path)?;
    ensure_socket_has_normal_component(path)
}

fn ensure_absolute_socket_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        return Ok(());
    }

    bail!("daemon.socket must be an absolute path")
}

fn ensure_socket_has_normal_component(path: &Path) -> Result<()> {
    if has_socket_file_component(path)? {
        return Ok(());
    }

    bail!("daemon.socket must reference a socket file path")
}

fn has_socket_file_component(path: &Path) -> Result<bool> {
    let mut has_normal_component = false;
    for component in path.components() {
        if let Component::ParentDir = component {
            bail!("daemon.socket cannot contain parent directory traversal");
        }
        if matches!(component, Component::Normal(_)) {
            has_normal_component = true;
        }
    }
    Ok(has_normal_component)
}
