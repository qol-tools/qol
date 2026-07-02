use crate::plugins::manifest::{DaemonConfig, NamedPort, PortProtocol};
use crate::plugins::Plugin;
use std::net::{TcpListener, UdpSocket};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::process::Command;

// Plugins whose daemon control socket is pre-bound by qol-tray and handed off
// as an already-open fd, instead of the daemon binding it itself. Migrating a
// plugin here is a one-line addition once its daemon adopts the inherited-fd
// fallback in qol_plugin_daemon::daemon::bind_listener.
const MIGRATED_PLUGINS: &[&str] = &[
    "plugin-alt-tab",
    "plugin-cli-sessions",
    "plugin-keyremap",
    "plugin-launcher",
    "plugin-lights",
    "plugin-os-themes",
    "plugin-pointz",
    "qol-shot",
];

#[derive(Debug)]
pub(in crate::plugins) struct DaemonListener {
    unix: UnixListener,
    port: Option<TcpListener>,
    extra: Vec<(String, ExtraSocket)>,
}

#[derive(Debug)]
enum ExtraSocket {
    Tcp(TcpListener),
    Udp(UdpSocket),
}

impl AsRawFd for ExtraSocket {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            ExtraSocket::Tcp(listener) => listener.as_raw_fd(),
            ExtraSocket::Udp(socket) => socket.as_raw_fd(),
        }
    }
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
    let unix = match bind_reclaiming_stale_socket(&socket_path) {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!(
                "Failed to pre-bind daemon listener for plugin {} at {:?}: {}. The daemon will bind its own socket instead.",
                plugin.id,
                socket_path,
                error
            );
            return None;
        }
    };
    let port = daemon_config
        .port
        .and_then(|port| bind_primary_port(plugin, port));
    let extra = daemon_config
        .extra_ports
        .iter()
        .filter_map(|named_port| bind_extra_port(plugin, named_port))
        .collect();
    Some(DaemonListener { unix, port, extra })
}

fn bind_primary_port(plugin: &Plugin, port: u16) -> Option<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log::warn!(
                "Failed to pre-bind daemon port for plugin {} on port {}: {}. The daemon will bind its own socket instead.",
                plugin.id,
                port,
                error
            );
            None
        }
    }
}

fn bind_reclaiming_stale_socket(socket_path: &std::path::Path) -> std::io::Result<UnixListener> {
    match UnixListener::bind(socket_path) {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if crate::plugins::action_transport::daemon_listener_reachable(socket_path) {
                return Err(error);
            }
            let _ = std::fs::remove_file(socket_path);
            UnixListener::bind(socket_path)
        }
        Err(error) => Err(error),
    }
}

fn bind_extra_port(plugin: &Plugin, named_port: &NamedPort) -> Option<(String, ExtraSocket)> {
    let bound = match named_port.protocol {
        PortProtocol::Tcp => {
            TcpListener::bind(("127.0.0.1", named_port.port)).map(ExtraSocket::Tcp)
        }
        PortProtocol::Udp => UdpSocket::bind(("0.0.0.0", named_port.port)).map(ExtraSocket::Udp),
    };
    match bound {
        Ok(socket) => Some((named_port.name.clone(), socket)),
        Err(error) => {
            log::warn!(
                "Failed to pre-bind daemon port '{}' for plugin {} on port {}: {}. The daemon will bind its own socket instead.",
                named_port.name,
                plugin.id,
                named_port.port,
                error
            );
            None
        }
    }
}

pub(super) fn apply_to_command(daemon_listener: &DaemonListener, command: &mut Command) {
    set_inheritable_fd(
        command,
        qol_conventions::ENV_DAEMON_LISTENER_FD.to_string(),
        daemon_listener.unix.as_raw_fd(),
    );
    if let Some(port) = &daemon_listener.port {
        set_inheritable_fd(
            command,
            qol_conventions::ENV_DAEMON_PORT_FD.to_string(),
            port.as_raw_fd(),
        );
    }
    for (name, socket) in &daemon_listener.extra {
        let env_name = format!(
            "{}_{}",
            qol_conventions::ENV_DAEMON_PORT_FD,
            name.to_uppercase()
        );
        set_inheritable_fd(command, env_name, socket.as_raw_fd());
    }
}

fn set_inheritable_fd(command: &mut Command, env_name: String, fd: RawFd) {
    command.env(env_name, fd.to_string());
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
        plugin_with_id("plugin-listener-test")
    }

    fn plugin_with_id(id: &str) -> Plugin {
        let manifest: PluginManifest = toml::from_str(&format!(
            r#"
[plugin]
id = "{id}"
name = "Foo"
description = ""
version = "1.0.0"

[menu]
label = "Foo"
items = []
"#
        ))
        .unwrap();
        Plugin::new(PluginId::new(id), manifest, std::path::PathBuf::new())
    }

    fn socket_daemon_config(socket: &str) -> DaemonConfig {
        DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: Some(socket.to_string()),
            port: None,
            extra_ports: Vec::new(),
        }
    }

    #[test]
    fn bind_for_plugin_returns_none_when_not_migrated() {
        let plugin = minimal_plugin();
        let daemon_config = socket_daemon_config("plugin-listener-test.sock");

        assert!(
            bind_for_plugin(&plugin, &daemon_config).is_none(),
            "a plugin id absent from MIGRATED_PLUGINS must never pre-bind"
        );
    }

    #[test]
    fn bind_for_plugin_binds_for_a_migrated_plugin() {
        // A migrated plugin id but a throwaway test-only socket name -
        // never the plugin's real declared socket - so this can't collide
        // with an actual running daemon on this machine.
        let plugin = plugin_with_id("plugin-alt-tab");
        let socket_path = format!("/tmp/qol-listener-test-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);
        let daemon_config = socket_daemon_config(&socket_path);

        let bound = bind_for_plugin(&plugin, &daemon_config);

        assert!(
            bound.is_some(),
            "plugin-alt-tab is in MIGRATED_PLUGINS and must be pre-bound"
        );
        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn bind_for_plugin_returns_none_when_daemon_has_no_socket() {
        let plugin = minimal_plugin();
        let daemon_config = DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: None,
            port: None,
            extra_ports: Vec::new(),
        };

        assert!(bind_for_plugin(&plugin, &daemon_config).is_none());
    }

    #[test]
    fn bind_reclaiming_stale_socket_removes_a_dead_leftover_and_rebinds() {
        let path = std::path::PathBuf::from(format!(
            "/tmp/qol-listener-test-stale-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let _dead_listener = UnixListener::bind(&path).unwrap();
            // dropped here: the socket file is left behind, but nothing is
            // listening on it anymore - exactly a leftover from a crashed
            // or force-killed prior generation.
        }
        assert!(path.exists(), "stale socket file must still be present");

        let listener = bind_reclaiming_stale_socket(&path);

        assert!(
            listener.is_ok(),
            "a dead leftover socket must be reclaimed, not treated as in-use"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bind_reclaiming_stale_socket_refuses_to_steal_a_live_listener() {
        let path = std::path::PathBuf::from(format!(
            "/tmp/qol-listener-test-live-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let _live_listener = UnixListener::bind(&path).unwrap();

        let listener = bind_reclaiming_stale_socket(&path);

        assert!(
            listener.is_err(),
            "a genuinely live listener must never be stolen out from under it"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bind_extra_port_binds_a_udp_socket() {
        let plugin = minimal_plugin();
        let named_port = NamedPort {
            name: "test-udp".to_string(),
            port: 0,
            protocol: PortProtocol::Udp,
        };

        let bound = bind_extra_port(&plugin, &named_port);

        assert!(
            bound.is_some(),
            "an ephemeral UDP port must always be bindable"
        );
        assert_eq!(bound.unwrap().0, "test-udp");
    }

    #[test]
    fn bind_extra_port_binds_a_tcp_socket() {
        let plugin = minimal_plugin();
        let named_port = NamedPort {
            name: "test-tcp".to_string(),
            port: 0,
            protocol: PortProtocol::Tcp,
        };

        let bound = bind_extra_port(&plugin, &named_port);

        assert!(
            bound.is_some(),
            "an ephemeral TCP port must always be bindable"
        );
        assert_eq!(bound.unwrap().0, "test-tcp");
    }

    #[test]
    fn bind_extra_port_returns_none_on_a_genuine_collision() {
        let plugin = minimal_plugin();
        let holder = UdpSocket::bind(("0.0.0.0", 0)).unwrap();
        let taken_port = holder.local_addr().unwrap().port();
        let named_port = NamedPort {
            name: "test-udp-collision".to_string(),
            port: taken_port,
            protocol: PortProtocol::Udp,
        };

        let bound = bind_extra_port(&plugin, &named_port);

        assert!(
            bound.is_none(),
            "a port already held by another socket must not be double-bound"
        );
    }

    #[test]
    fn bind_primary_port_binds_a_tcp_socket() {
        let plugin = minimal_plugin();

        let bound = bind_primary_port(&plugin, 0);

        assert!(bound.is_some(), "an ephemeral TCP port must always bind");
    }

    #[test]
    fn bind_primary_port_returns_none_on_a_genuine_collision() {
        let plugin = minimal_plugin();
        let holder = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let taken_port = holder.local_addr().unwrap().port();

        let bound = bind_primary_port(&plugin, taken_port);

        assert!(
            bound.is_none(),
            "a port already held by another listener must not be double-bound"
        );
    }

    #[test]
    fn bind_for_plugin_includes_the_primary_port_for_a_migrated_plugin() {
        let plugin = plugin_with_id("plugin-alt-tab");
        let socket_path = format!(
            "/tmp/qol-listener-test-primary-port-{}.sock",
            std::process::id()
        );
        let _ = std::fs::remove_file(&socket_path);
        let daemon_config = DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: Some(socket_path.clone()),
            port: Some(0),
            extra_ports: Vec::new(),
        };

        let bound = bind_for_plugin(&plugin, &daemon_config).unwrap();

        assert!(
            bound.port.is_some(),
            "the declared top-level port must be pre-bound"
        );
        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn apply_to_command_publishes_the_primary_port_env_var_without_a_suffix() {
        use std::ffi::OsStr;

        let dir = tempfile::TempDir::new().unwrap();
        let unix = UnixListener::bind(dir.path().join("primary-port-env.sock")).unwrap();
        let port = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let expected_fd = port.as_raw_fd().to_string();
        let listener = DaemonListener {
            unix,
            port: Some(port),
            extra: Vec::new(),
        };
        let mut command = Command::new("/bin/true");

        apply_to_command(&listener, &mut command);

        let entry = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(qol_conventions::ENV_DAEMON_PORT_FD));
        let (_, value) = entry.expect("primary port fd env var must be set");
        assert_eq!(value, Some(OsStr::new(expected_fd.as_str())));
    }

    #[test]
    fn bind_for_plugin_includes_extra_ports_for_a_migrated_plugin() {
        let plugin = plugin_with_id("plugin-alt-tab");
        let socket_path = format!("/tmp/qol-listener-test-extra-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);
        let daemon_config = DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: Some(socket_path.clone()),
            port: None,
            extra_ports: vec![NamedPort {
                name: "discovery".to_string(),
                port: 0,
                protocol: PortProtocol::Udp,
            }],
        };

        let bound = bind_for_plugin(&plugin, &daemon_config).unwrap();

        assert_eq!(
            bound.extra.len(),
            1,
            "the declared extra port must be pre-bound"
        );
        assert_eq!(bound.extra[0].0, "discovery");
        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn apply_to_command_publishes_a_named_env_var_per_extra_port() {
        use std::ffi::OsStr;

        let dir = tempfile::TempDir::new().unwrap();
        let unix = UnixListener::bind(dir.path().join("extra-env.sock")).unwrap();
        let udp = UdpSocket::bind(("0.0.0.0", 0)).unwrap();
        let expected_fd = udp.as_raw_fd().to_string();
        let listener = DaemonListener {
            unix,
            port: None,
            extra: vec![("discovery".to_string(), ExtraSocket::Udp(udp))],
        };
        let mut command = Command::new("/bin/true");

        apply_to_command(&listener, &mut command);

        let expected_name = format!("{}_DISCOVERY", qol_conventions::ENV_DAEMON_PORT_FD);
        let entry = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(&expected_name));
        let (_, value) = entry.expect("named port fd env var must be set");
        assert_eq!(value, Some(OsStr::new(expected_fd.as_str())));
    }

    fn bound_listener(dir: &tempfile::TempDir, name: &str) -> DaemonListener {
        DaemonListener {
            unix: UnixListener::bind(dir.path().join(name)).unwrap(),
            port: None,
            extra: Vec::new(),
        }
    }

    #[test]
    fn apply_to_command_publishes_the_listener_fd_env_var() {
        use std::ffi::OsStr;

        let dir = tempfile::TempDir::new().unwrap();
        let listener = bound_listener(&dir, "env-var.sock");
        let expected_fd = listener.unix.as_raw_fd().to_string();
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
        let fd = listener.unix.as_raw_fd();
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
        let fd = listener.unix.as_raw_fd();
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
