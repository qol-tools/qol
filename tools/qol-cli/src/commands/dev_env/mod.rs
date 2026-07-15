pub(crate) mod registry;
pub(crate) mod resources;

use crate::commands::emu;
use crate::workspace::repo_root;
use anyhow::{Context, Result};
use registry::{EnvironmentDefinition, LocalConfig, ResolvedEnvironment};
use std::path::PathBuf;
use std::process::Command;

const HOST_SESSION_VARIABLES: [&str; 14] = [
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "SESSION_MANAGER",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_DESKTOP",
    "XDG_SESSION_ID",
    "XDG_SESSION_TYPE",
    "SSH_AUTH_SOCK",
    "PULSE_SERVER",
    "PIPEWIRE_REMOTE",
];

pub(crate) fn discover() -> Result<Vec<ResolvedEnvironment>> {
    let root = repo_root()?;
    let definitions = registry::discover_definitions(&root)?;
    let config_path = config_path().context("could not determine dev environment config path")?;
    let config = with_defaults(registry::load_local_config(&config_path)?)?;
    registry::resolve_definitions(definitions, &config, backend_supported)
}

pub(crate) fn find(id: &str) -> Result<Option<ResolvedEnvironment>> {
    Ok(discover()?
        .into_iter()
        .find(|environment| environment.definition.id == id))
}

pub(crate) fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|root| root.join("qol").join("dev-envs.toml"))
}

pub(crate) fn clear_host_session(command: &mut Command) {
    for variable in HOST_SESSION_VARIABLES {
        command.env_remove(variable);
    }
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

pub(crate) fn with_defaults(mut config: LocalConfig) -> Result<LocalConfig> {
    let root = repo_root()?;
    if config.image_root.is_none() {
        config.image_root = emu::emu_dir();
    }
    if config.run_root.is_none() {
        config.run_root = Some(root.join("target/qol-env"));
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clears_every_host_session_variable() {
        let mut command = Command::new("qol");
        for variable in HOST_SESSION_VARIABLES {
            command.env(variable, "inherited");
        }

        clear_host_session(&mut command);

        let removed = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            removed,
            HOST_SESSION_VARIABLES
                .into_iter()
                .map(str::to_string)
                .collect()
        );
    }
}
