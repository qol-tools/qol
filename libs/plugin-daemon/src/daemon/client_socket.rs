use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{DaemonConfig, SocketSource};

const MANIFEST_FILE_NAME: &str = "plugin.toml";

pub(super) fn resolve(config: &DaemonConfig) -> Option<PathBuf> {
    super::socket_path(config).or_else(|| manifest_assigned_socket_path(config))
}

fn manifest_assigned_socket_path(config: &DaemonConfig) -> Option<PathBuf> {
    let declared = declared_socket_name(config)?;
    qol_runtime::plugin_socket::assigned_socket_path(&declared)
}

fn declared_socket_name(config: &DaemonConfig) -> Option<String> {
    if let SocketSource::Path(path) = &config.socket {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .map(ToString::to_string);
    }
    read_manifest_socket_name_from(manifest_dir()?.as_path())
}

fn manifest_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("QOL_TRAY_PLUGIN_DIR") {
        return Some(PathBuf::from(value));
    }
    let exe = std::env::current_exe().ok()?;
    exe.parent().map(Path::to_path_buf)
}

fn read_manifest_socket_name_from(dir: &Path) -> Option<String> {
    let raw = read_manifest_text(dir)?;
    let manifest: ManifestLite = toml::from_str(&raw).ok()?;
    manifest.daemon?.socket
}

const MANIFEST_SEARCH_DEPTH: usize = 4;

fn read_manifest_text(dir: &Path) -> Option<String> {
    for candidate in manifest_candidates(dir) {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return Some(text);
        }
    }
    None
}

fn manifest_candidates(dir: &Path) -> Vec<PathBuf> {
    dir.ancestors()
        .take(MANIFEST_SEARCH_DEPTH)
        .map(|ancestor| ancestor.join(MANIFEST_FILE_NAME))
        .collect()
}

#[derive(Deserialize)]
struct ManifestLite {
    daemon: Option<DaemonSection>,
}

#[derive(Deserialize)]
struct DaemonSection {
    #[serde(default)]
    socket: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &Path, body: &str) {
        let mut file = std::fs::File::create(dir.join(MANIFEST_FILE_NAME)).unwrap();
        file.write_all(body.as_bytes()).unwrap();
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    const SAMPLE_MANIFEST: &str = r#"[plugin]
id = "qol-sample"
uid = "u"
name = "Sample"
description = ""
version = "1.0.0"

[runtime]
command = "qol-sample"

[daemon]
enabled = true
command = "qol-sample"
socket = "/tmp/qol-sample.sock"

[menu]
label = "Sample"
items = []
"#;

    #[test]
    fn reads_the_declared_socket_name_from_the_manifest_in_the_plugin_dir() {
        let dir = TempDir::new().unwrap();
        write_manifest(
            dir.path(),
            r#"[plugin]
id = "qol-sample"
uid = "u"
name = "Sample"
description = ""
version = "1.0.0"

[runtime]
command = "qol-sample"

[daemon]
enabled = true
command = "qol-sample"
socket = "/tmp/qol-sample.sock"

[menu]
label = "Sample"
items = []
"#,
        );

        let name = read_manifest_socket_name_from(dir.path());

        assert_eq!(name.as_deref(), Some("/tmp/qol-sample.sock"));
    }

    #[test]
    fn returns_none_when_the_manifest_has_no_daemon_socket() {
        let dir = TempDir::new().unwrap();
        write_manifest(
            dir.path(),
            r#"[plugin]
id = "qol-sample"
uid = "u"
name = "Sample"
description = ""
version = "1.0.0"

[runtime]
command = "qol-sample"

[menu]
label = "Sample"
items = []
"#,
        );

        let name = read_manifest_socket_name_from(dir.path());

        assert!(name.is_none(), "expected no socket, got {name:?}");
    }

    #[test]
    fn returns_none_when_the_manifest_is_absent() {
        let dir = TempDir::new().unwrap();

        let name = read_manifest_socket_name_from(dir.path());

        assert!(name.is_none());
    }

    #[test]
    fn falls_back_to_the_parent_dir_when_the_exe_sits_in_a_target_subfolder() {
        let dir = TempDir::new().unwrap();
        write_manifest(dir.path(), SAMPLE_MANIFEST);
        let nested = dir.path().join("target").join("debug");
        std::fs::create_dir_all(&nested).unwrap();

        let name = read_manifest_socket_name_from(&nested);

        assert_eq!(name.as_deref(), Some("/tmp/qol-sample.sock"));
    }

    #[test]
    fn env_required_without_env_resolves_from_the_plugin_dir_manifest() {
        let _lock = env_lock();
        let dir = TempDir::new().unwrap();
        write_manifest(dir.path(), SAMPLE_MANIFEST);
        std::env::remove_var(qol_conventions::ENV_DAEMON_SOCKET);
        std::env::set_var("QOL_TRAY_PLUGIN_DIR", dir.path());
        let config = DaemonConfig {
            socket: SocketSource::EnvRequired,
            support_replace_existing: false,
        };

        let path = resolve(&config);
        std::env::remove_var("QOL_TRAY_PLUGIN_DIR");

        let path = path.expect("manifest-based fallback must find a socket path");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("qol-sample.sock")
        );
        assert_eq!(
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("sockets"),
            "fallback must place the socket under the runtime sockets dir, not /tmp"
        );
    }

    #[test]
    fn env_required_with_env_prefers_the_env_over_the_manifest() {
        let _lock = env_lock();
        let dir = TempDir::new().unwrap();
        write_manifest(dir.path(), SAMPLE_MANIFEST);
        std::env::set_var(qol_conventions::ENV_DAEMON_SOCKET, "/tmp/env-chosen.sock");
        std::env::set_var("QOL_TRAY_PLUGIN_DIR", dir.path());
        let config = DaemonConfig {
            socket: SocketSource::EnvRequired,
            support_replace_existing: false,
        };

        let path = resolve(&config);
        std::env::remove_var(qol_conventions::ENV_DAEMON_SOCKET);
        std::env::remove_var("QOL_TRAY_PLUGIN_DIR");

        assert_eq!(path, Some(PathBuf::from("/tmp/env-chosen.sock")));
    }

    #[test]
    fn env_required_without_env_returns_none_when_no_manifest_is_findable() {
        let _lock = env_lock();
        let dir = TempDir::new().unwrap();
        std::env::remove_var(qol_conventions::ENV_DAEMON_SOCKET);
        std::env::set_var("QOL_TRAY_PLUGIN_DIR", dir.path());
        let config = DaemonConfig {
            socket: SocketSource::EnvRequired,
            support_replace_existing: false,
        };

        let path = resolve(&config);
        std::env::remove_var("QOL_TRAY_PLUGIN_DIR");

        assert!(path.is_none(), "no manifest means no fallback: {path:?}");
    }
}
