//! Cross-process constants shared by qol-tray, qol-cli, and every plugin.
//!
//! This crate is the single source of truth for the well-known values that
//! several independent processes must agree on. A literal-guard (run in CI and
//! as a pre-commit hook) forbids these values from appearing anywhere else, so
//! they cannot drift.

pub const DEFAULT_PORT: u16 = 42700;
pub const LOCAL_HOST: &str = "127.0.0.1";

pub const STATE_SOCKET_PATH: &str = "/tmp/qol-tray-state.sock";
pub const TRACE_LOG_PATH: &str = "/tmp/qol-altmon.log";

pub const ENV_STATE_SOCKET: &str = "QOL_TRAY_STATE_SOCKET";
pub const ENV_PLUGIN_ID: &str = "QOL_TRAY_PLUGIN_ID";
pub const ENV_DAEMON_SOCKET: &str = "QOL_TRAY_DAEMON_SOCKET";
pub const ENV_DAEMON_REPLACE_EXISTING: &str = "QOL_TRAY_DAEMON_REPLACE_EXISTING";

pub fn local_base_url() -> String {
    format!("http://{LOCAL_HOST}:{DEFAULT_PORT}")
}

pub fn settings_url(plugin_id: &str) -> String {
    format!("http://{LOCAL_HOST}:{DEFAULT_PORT}/plugins/{plugin_id}/")
}

const RESERVED_PLUGIN_IDS: &[&str] = &["plugin-template"];

pub fn is_reserved_plugin_id(id: &str) -> bool {
    RESERVED_PLUGIN_IDS.contains(&id)
}

pub mod build;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_uses_the_default_port() {
        assert_eq!(local_base_url(), "http://127.0.0.1:42700");
    }

    #[test]
    fn settings_url_targets_the_plugin_namespace() {
        assert_eq!(
            settings_url("plugin-foo"),
            "http://127.0.0.1:42700/plugins/plugin-foo/"
        );
    }

    #[test]
    fn only_template_is_a_reserved_plugin_id() {
        let cases = [
            ("plugin-template", true),
            ("plugin-foo", false),
            ("plugin-bar", false),
            ("", false),
        ];
        for (id, expected) in cases {
            assert_eq!(is_reserved_plugin_id(id), expected, "id: {id}");
        }
    }
}
