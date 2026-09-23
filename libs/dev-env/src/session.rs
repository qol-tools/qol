use anyhow::{bail, Result};
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

pub fn clear_host_session(command: &mut Command) {
    for variable in HOST_SESSION_VARIABLES {
        command.env_remove(variable);
    }
}

pub fn require_host_session_cleared() -> Result<()> {
    require_host_session_cleared_with(|variable| std::env::var_os(variable).is_some())
}

fn require_host_session_cleared_with(mut is_present: impl FnMut(&str) -> bool) -> Result<()> {
    let inherited = HOST_SESSION_VARIABLES
        .into_iter()
        .filter(|variable| is_present(variable))
        .collect::<Vec<_>>();
    if inherited.is_empty() {
        return Ok(());
    }
    bail!(
        "host session environment was inherited: {}",
        inherited.join(", ")
    )
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

    #[test]
    fn accepts_a_process_without_host_session_variables() {
        require_host_session_cleared_with(|_| false).unwrap();
    }

    #[test]
    fn rejects_inherited_host_session_variables_without_reading_values() {
        let inherited = ["DISPLAY", "DBUS_SESSION_BUS_ADDRESS"]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();

        let error =
            require_host_session_cleared_with(|variable| inherited.contains(variable)).unwrap_err();

        assert_eq!(
            error.to_string(),
            "host session environment was inherited: DISPLAY, DBUS_SESSION_BUS_ADDRESS"
        );
    }
}
