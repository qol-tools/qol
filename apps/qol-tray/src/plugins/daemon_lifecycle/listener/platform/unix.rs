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
    "plugin-ide-checkout",
    "plugin-keyremap",
    "plugin-launcher",
    "plugin-lights",
    "plugin-os-themes",
    "plugin-pointz",
    "qol-shot",
];

#[derive(Debug)]
pub(in crate::plugins) struct DaemonListener {
    unix: Option<UnixListener>,
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

pub(in crate::plugins::daemon_lifecycle) fn bind_for_plugin(
    plugin: &Plugin,
    daemon_config: &DaemonConfig,
) -> Option<DaemonListener> {
    if !MIGRATED_PLUGINS.contains(&plugin.id.as_str()) {
        return None;
    }
    let unix = daemon_config
        .socket
        .as_deref()
        .and_then(|socket| bind_unix_socket(plugin, socket));
    let port = daemon_config
        .port
        .and_then(|port| bind_primary_port(plugin, port));
    let extra = daemon_config
        .extra_ports
        .iter()
        .filter_map(|named_port| bind_extra_port(plugin, named_port))
        .collect::<Vec<_>>();

    if unix.is_none() && port.is_none() && extra.is_empty() {
        return None;
    }
    Some(DaemonListener { unix, port, extra })
}

// Revalidates a listener retained from the previous spawn. A daemon exit
// can leave the unix socket path unlinked out from under the retained fd
// (nothing can connect to it anymore), and pre-binds that lost their
// first attempt - typically to the predecessor generation still holding
// the port - deserve another try once that generation is gone.
pub(in crate::plugins::daemon_lifecycle) fn refresh_for_respawn(
    mut listener: DaemonListener,
    plugin: &Plugin,
    daemon_config: &DaemonConfig,
) -> Option<DaemonListener> {
    if listener.unix.is_some() {
        let path_gone = daemon_config
            .socket
            .as_deref()
            .map(crate::dev_generation::daemon_socket_path)
            .is_some_and(|path| !path.exists());
        if path_gone {
            return None;
        }
    } else if let Some(socket) = daemon_config.socket.as_deref() {
        listener.unix = bind_unix_socket(plugin, socket);
    }
    if listener.port.is_none() {
        listener.port = daemon_config
            .port
            .and_then(|port| bind_primary_port(plugin, port));
    }
    for named_port in &daemon_config.extra_ports {
        if listener
            .extra
            .iter()
            .any(|(name, _)| name == &named_port.name)
        {
            continue;
        }
        if let Some(bound) = bind_extra_port(plugin, named_port) {
            listener.extra.push(bound);
        }
    }
    Some(listener)
}

fn bind_unix_socket(plugin: &Plugin, socket: &str) -> Option<UnixListener> {
    let socket_path = crate::dev_generation::daemon_socket_path(socket);
    if let Some(parent) = socket_path.parent() {
        if let Err(error) = qol_fs::create_private_dir(parent) {
            log::warn!(
                "Failed to prepare private daemon socket directory for plugin {} at {:?}: {}",
                plugin.id,
                parent,
                error
            );
            return None;
        }
    }
    match bind_reclaiming_stale_socket(&socket_path) {
        Ok(listener) => Some(listener),
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
    match qol_runtime::local_ipc::bind_listener(socket_path) {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if crate::plugins::action_transport::daemon_listener_reachable(socket_path) {
                return Err(error);
            }
            let _ = std::fs::remove_file(socket_path);
            qol_runtime::local_ipc::bind_listener(socket_path)
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

pub(in crate::plugins::daemon_lifecycle) fn apply_to_command(
    daemon_listener: &DaemonListener,
    command: &mut Command,
) {
    if let Some(unix) = &daemon_listener.unix {
        set_inheritable_fd(
            command,
            qol_conventions::ENV_DAEMON_LISTENER_FD.to_string(),
            unix.as_raw_fd(),
        );
    }
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
        let _env = crate::test_support::env_lock().blocking_lock();
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

    // Regression test: plugin-ide-checkout declares only a top-level `port`
    // in plugin.toml, no `socket` at all. bind_for_plugin must not treat the
    // unix socket as a prerequisite gate for binding anything else.
    #[test]
    fn bind_for_plugin_binds_the_port_even_without_a_socket() {
        let plugin = plugin_with_id("plugin-alt-tab");
        let daemon_config = DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: None,
            port: Some(0),
            extra_ports: Vec::new(),
        };

        let bound = bind_for_plugin(&plugin, &daemon_config);

        let bound = bound.expect("a declared port alone must still be pre-bound");
        assert!(bound.unix.is_none());
        assert!(bound.port.is_some());
    }

    #[test]
    fn refresh_for_respawn_drops_a_listener_whose_socket_path_was_unlinked() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let plugin = plugin_with_id("plugin-alt-tab");
        let socket_path = format!("/tmp/qol-ln-unlinked-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);
        let daemon_config = socket_daemon_config(&socket_path);
        let effective_path = crate::dev_generation::daemon_socket_path(&socket_path);
        let bound = bind_for_plugin(&plugin, &daemon_config).unwrap();
        std::fs::remove_file(&effective_path).unwrap();

        let refreshed = refresh_for_respawn(bound, &plugin, &daemon_config);

        assert!(
            refreshed.is_none(),
            "a retained fd whose socket path was unlinked serves a socket no \
                 path resolves to; it must be dropped so the respawn rebinds fresh"
        );
    }

    #[test]
    fn refresh_for_respawn_keeps_a_listener_whose_socket_path_is_intact() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let plugin = plugin_with_id("plugin-alt-tab");
        let socket_path = format!("/tmp/qol-ln-intact-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);
        let daemon_config = socket_daemon_config(&socket_path);
        let bound = bind_for_plugin(&plugin, &daemon_config).unwrap();

        let refreshed = refresh_for_respawn(bound, &plugin, &daemon_config);

        let refreshed = refreshed.expect("an intact listener must be retained");
        assert!(refreshed.unix.is_some());
        let _ = std::fs::remove_file(&socket_path);
    }

    #[test]
    fn refresh_for_respawn_binds_ports_that_failed_the_first_time() {
        let _env = crate::test_support::env_lock().blocking_lock();
        let plugin = plugin_with_id("plugin-alt-tab");
        let socket_path = format!("/tmp/qol-ln-ports-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);
        let daemon_config = DaemonConfig {
            enabled: true,
            command: "any".to_string(),
            socket: Some(socket_path.clone()),
            port: Some(0),
            extra_ports: vec![NamedPort {
                name: "discovery".to_string(),
                port: 0,
                protocol: PortProtocol::Udp,
            }],
        };
        let effective_path = crate::dev_generation::daemon_socket_path(&socket_path);
        if let Some(parent) = effective_path.parent() {
            qol_fs::create_private_dir(parent).unwrap();
        }
        let unix = qol_runtime::local_ipc::bind_listener(&effective_path).unwrap();
        let partial = DaemonListener {
            unix: Some(unix),
            port: None,
            extra: Vec::new(),
        };

        let refreshed = refresh_for_respawn(partial, &plugin, &daemon_config);

        let refreshed = refreshed.expect("a partial listener must be retained and completed");
        assert!(
            refreshed.port.is_some(),
            "a declared port that lost its first bind (e.g. to the predecessor \
                 generation) must be re-attempted on respawn"
        );
        assert_eq!(
            refreshed.extra.len(),
            1,
            "missing extra ports must be re-attempted"
        );
        assert_eq!(refreshed.extra[0].0, "discovery");
        let _ = std::fs::remove_file(&effective_path);
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
        let socket_path = format!("/tmp/qol-ln-primary-{}.sock", std::process::id());
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
            unix: Some(unix),
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
            unix: Some(unix),
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
            unix: Some(UnixListener::bind(dir.path().join(name)).unwrap()),
            port: None,
            extra: Vec::new(),
        }
    }

    #[test]
    fn apply_to_command_publishes_the_listener_fd_env_var() {
        use std::ffi::OsStr;

        let dir = tempfile::TempDir::new().unwrap();
        let listener = bound_listener(&dir, "env-var.sock");
        let expected_fd = listener.unix.as_ref().unwrap().as_raw_fd().to_string();
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
        let fd = listener.unix.as_ref().unwrap().as_raw_fd();
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
        let fd = listener.unix.as_ref().unwrap().as_raw_fd();
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
