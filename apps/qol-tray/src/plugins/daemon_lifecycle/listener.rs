use crate::plugins::manifest::DaemonConfig;
use crate::plugins::Plugin;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::process::Command;

// Plugins whose daemon control socket is pre-bound by qol-tray and handed off
// as an already-open fd, instead of the daemon binding it itself. Migrating a
// plugin here is a one-line addition once its daemon adopts the inherited-fd
// fallback in qol_plugin_daemon::daemon::bind_listener.
const MIGRATED_PLUGINS: &[&str] = &[];

#[derive(Debug)]
pub(in crate::plugins) struct DaemonListener {
    listener: UnixListener,
}

pub(super) fn bind_for_plugin(
    plugin: &Plugin,
    daemon_config: &DaemonConfig,
) -> Option<DaemonListener> {
    if !MIGRATED_PLUGINS.contains(&plugin.id.as_str()) {
        return None;
    }
    let socket = daemon_config.socket.as_deref()?;
    let socket_path = crate::dev_generation::daemon_socket_path(socket);
    match UnixListener::bind(&socket_path) {
        Ok(listener) => Some(DaemonListener { listener }),
        Err(error) => {
            log::warn!(
                "Failed to pre-bind daemon listener for plugin {} at {:?}: {}. The daemon will bind its own socket instead.",
                plugin.id,
                socket_path,
                error
            );
            None
        }
    }
}

pub(super) fn apply_to_command(daemon_listener: &DaemonListener, command: &mut Command) {
    let fd = daemon_listener.listener.as_raw_fd();
    command.env(qol_conventions::ENV_DAEMON_LISTENER_FD, fd.to_string());
    unsafe {
        command.pre_exec(move || clear_cloexec(fd));
    }
}

fn clear_cloexec(fd: RawFd) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let cleared = unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) };
    if cleared < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::PluginManifest;
    use crate::plugins::PluginId;

    fn minimal_plugin() -> Plugin {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
id = "plugin-listener-test"
name = "Foo"
description = ""
version = "1.0.0"

[menu]
label = "Foo"
items = []
"#,
        )
        .unwrap();
        Plugin::new(
            PluginId::new("plugin-listener-test"),
            manifest,
            std::path::PathBuf::new(),
        )
    }

    fn socket_daemon_config(socket: &str) -> DaemonConfig {
        DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: Some(socket.to_string()),
            port: None,
        }
    }

    #[test]
    fn bind_for_plugin_returns_none_when_not_migrated() {
        let plugin = minimal_plugin();
        let daemon_config = socket_daemon_config("plugin-listener-test.sock");

        assert!(
            bind_for_plugin(&plugin, &daemon_config).is_none(),
            "an empty MIGRATED_PLUGINS allowlist must never pre-bind"
        );
    }

    #[test]
    fn bind_for_plugin_returns_none_when_daemon_has_no_socket() {
        let plugin = minimal_plugin();
        let daemon_config = DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: None,
            port: None,
        };

        assert!(bind_for_plugin(&plugin, &daemon_config).is_none());
    }

    fn bound_listener(dir: &tempfile::TempDir, name: &str) -> DaemonListener {
        DaemonListener {
            listener: UnixListener::bind(dir.path().join(name)).unwrap(),
        }
    }

    #[test]
    fn apply_to_command_publishes_the_listener_fd_env_var() {
        use std::ffi::OsStr;

        let dir = tempfile::TempDir::new().unwrap();
        let listener = bound_listener(&dir, "env-var.sock");
        let expected_fd = listener.listener.as_raw_fd().to_string();
        let mut command = Command::new("/bin/true");

        apply_to_command(&listener, &mut command);

        let entry = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(qol_conventions::ENV_DAEMON_LISTENER_FD));
        let (_, value) = entry.expect("fd env var must be set");
        assert_eq!(value, Some(OsStr::new(expected_fd.as_str())));
    }

    // Mirrors lifeline_handoff.rs's own test style for the same class of
    // problem: verify the cloexec flag transition directly via fcntl rather
    // than spawning a real child and probing fd inheritance by number. Raw fd
    // numbers get reused across this binary's ~1000+ parallel tests, so a
    // shell-redirect-based inheritance check is flaky (a coincidentally
    // reused fd number in the child can false-positive).
    fn cloexec_is_set(fd: RawFd) -> bool {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(flags >= 0, "fd {fd} must be open");
        flags & libc::FD_CLOEXEC != 0
    }

    #[test]
    fn clear_cloexec_flips_the_close_on_exec_flag() {
        let dir = tempfile::TempDir::new().unwrap();
        let listener = bound_listener(&dir, "cloexec.sock");
        let fd = listener.listener.as_raw_fd();
        assert!(cloexec_is_set(fd), "std sockets start close-on-exec");

        clear_cloexec(fd).unwrap();

        assert!(
            !cloexec_is_set(fd),
            "clear_cloexec must make the fd inheritable across exec"
        );
    }

    #[test]
    fn apply_to_command_does_not_touch_the_parents_own_cloexec_flag() {
        let dir = tempfile::TempDir::new().unwrap();
        let listener = bound_listener(&dir, "parent-untouched.sock");
        let fd = listener.listener.as_raw_fd();
        let mut command = Command::new("/bin/true");

        apply_to_command(&listener, &mut command);

        assert!(
            cloexec_is_set(fd),
            "cloexec must only be cleared on the forked child's copy via pre_exec, \
             never on the parent's own fd table -- otherwise every other \
             Command::spawn from this process would also leak this fd"
        );
    }
}
