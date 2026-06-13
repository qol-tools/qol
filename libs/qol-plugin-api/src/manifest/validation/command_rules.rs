use crate::manifest::DaemonConfig;
use anyhow::{bail, Result};
use std::path::{Component, Path};

impl DaemonConfig {
    pub fn validate(&self) -> Result<()> {
        validate_command_name("daemon.command", &self.command)?;
        validate_socket_config(self.socket.as_deref())?;
        Ok(())
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

pub(super) fn validate_optional_daemon_config(daemon: Option<&DaemonConfig>) -> Result<()> {
    let Some(daemon) = daemon else {
        return Ok(());
    };

    daemon.validate()
}

pub(super) fn validate_command_name(field: &str, value: &str) -> Result<()> {
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
