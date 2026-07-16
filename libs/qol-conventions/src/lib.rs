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
pub const RUNTIME_DIR_PATH: &str = "/tmp/qol-tray";
pub const RUNTIME_PIDS_DIR_PATH: &str = "/tmp/qol-tray/pids";

pub const ENV_STATE_SOCKET: &str = "QOL_TRAY_STATE_SOCKET";
pub const ENV_INSTALL_ID: &str = "QOL_TRAY_INSTALL_ID";
pub const ENV_PLUGIN_ID: &str = "QOL_TRAY_PLUGIN_ID";
pub const ENV_DAEMON_SOCKET: &str = "QOL_TRAY_DAEMON_SOCKET";
pub const ENV_DAEMON_REPLACE_EXISTING: &str = "QOL_TRAY_DAEMON_REPLACE_EXISTING";
pub const ENV_DAEMON_LISTENER_FD: &str = "QOL_TRAY_DAEMON_LISTENER_FD";
pub const ENV_DAEMON_PORT_FD: &str = "QOL_TRAY_DAEMON_PORT_FD";
pub const ENV_THEME_ACCENT: &str = "QOL_TRAY_THEME_ACCENT";
pub const ENV_THEME_NAME: &str = "QOL_TRAY_THEME_NAME";

pub const ENV_DEV_GENERATION_MODE: &str = "QOL_DEV_GENERATION_MODE";
pub const ENV_DEV_GENERATION_ID: &str = "QOL_DEV_GENERATION_ID";
pub const ENV_DEV_READY_FILE: &str = "QOL_DEV_READY_FILE";
pub const ENV_DEV_UI_PORT: &str = "QOL_DEV_UI_PORT";
pub const ENV_DEV_ROLLING_RESTART: &str = "QOL_DEV_ROLLING_RESTART";
pub const DEV_GENERATION_MODE_SHADOW: &str = "shadow";
pub const SHUTDOWN_ROUTE: &str = "/shutdown";
pub const DEV_RESTART_PREBUILT_ROUTE: &str = dev_routes::RESTART_PREBUILT;
pub const DEV_PROMOTE_GENERATION_ROUTE: &str = dev_routes::PROMOTE_GENERATION;

pub mod dev_routes {
    pub const ENABLED: &str = "/dev/enabled";
    pub const RELOAD: &str = "/dev/reload";
    pub const RELOAD_PLUGIN: &str = "/dev/reload/{plugin_id}";
    pub const RECOMPILE_SELF: &str = "/dev/recompile-self";
    pub const RESTART_PREBUILT: &str = "/dev/restart-prebuilt";
    pub const PROMOTE_GENERATION: &str = "/dev/promote-generation";
    pub const WORKTREES: &str = "/dev/worktrees";
    pub const ACTIVE_WORKTREE: &str = "/dev/active-worktree";
    pub const PLUGIN_HEALTH: &str = "/dev/plugin-health";
    pub const LINKS: &str = "/dev/links";
    pub const LINK: &str = "/dev/links/{id}";
    pub const LOG_CONTROLS: &str = "/dev/log-controls";
    pub const LOG_CONTROL: &str = "/dev/log-controls/{id}";
    pub const CORE_LOG_CONTROLS: &str = "/dev/core-log-controls";
    pub const CORE_LOG_CONTROL: &str = "/dev/core-log-controls/{section}";
    pub const DISCOVER: &str = "/dev/discover";
    pub const DISCOVERY_STATE: &str = "/dev/discovery-state";
    pub const BUILD_STATE: &str = "/dev/build-state";
    pub const PLUGIN_CPU: &str = "/dev/plugin-cpu";
    pub const PLUGIN_CPU_MONITORING: &str = "/dev/plugin-cpu/monitoring";
    pub const TOOLING_GH_ACCOUNT: &str = "/dev/tooling-gh-account";
    pub const RUNTIME_GPUI: &str = "/dev/runtime/gpui";
    pub const MOCK_CHECK_UPDATE: &str = "/dev/mock-check-update";
    pub const MOCK_TARGETS: &str = "/dev/mock-targets";
    pub const MOCK_TARGETS_START: &str = "/dev/mock-targets/start";
    pub const MOCK_TARGETS_STOP: &str = "/dev/mock-targets/stop";
    pub const MOCK_PLUGIN_BUILD: &str = "/dev/mock-plugin-build";
    pub const MOCK_PLUGIN_BUILD_STOP: &str = "/dev/mock-plugin-build/stop";
    pub const MOCK_SELF_RECOMPILE: &str = "/dev/mock-self-recompile";
    pub const MOCK_SELF_RECOMPILE_STOP: &str = "/dev/mock-self-recompile/stop";
    pub const MOCK_SELF_UPDATE: &str = "/dev/mock-self-update";
    pub const MOCK_SELF_UPDATE_STOP: &str = "/dev/mock-self-update/stop";
    pub const UPDATE_FIXTURE: &str = "/dev/update-fixture.tar.gz";
    pub const TEST_SELF_UPDATE: &str = "/dev/test-self-update";

    pub fn link(id: &str) -> String {
        format!("/dev/links/{id}")
    }

    pub fn api_path(route: &str) -> String {
        format!("/api{route}")
    }
}

/// The `qol-tray-doctor` CLI contract shared between its parser
/// (`apps/qol-tray/src/doctor/cli.rs`) and its callers in `tools/qol-cli`
/// (the dashboard's background/manual probes and the top-level `qol doctor`
/// command). Not covered by the literal-guard above: "check"/"fix" are
/// common English words that also appear as unrelated progress-step labels
/// elsewhere in `tools/qol-cli`, so a repo-wide grep for them would be noisy
/// rather than protective. Usage/error strings and test fixtures may still
/// spell these out literally; only the actual parse/dispatch logic must use
/// these constants.
pub mod doctor_cli {
    pub const ARG_CHECK: &str = "check";
    pub const ARG_FIX: &str = "fix";
    pub const ARG_JSON: &str = "--json";
    pub const ARG_QUICK: &str = "--quick";
    pub const ARG_ID: &str = "--id";
}

pub mod doctor_wire {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum OutcomeStatus {
        Ok,
        Warn,
        Error,
        Crash,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct Outcome {
        pub id: String,
        pub status: OutcomeStatus,
        pub message: String,
        pub fix_available: bool,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
    pub struct Report {
        pub outcomes: Vec<Outcome>,
    }

    impl Report {
        pub fn new(outcomes: Vec<Outcome>) -> Self {
            Self { outcomes }
        }

        pub fn count_ok(&self) -> usize {
            self.count(OutcomeStatus::Ok)
        }

        pub fn count_warn(&self) -> usize {
            self.count(OutcomeStatus::Warn)
        }

        pub fn count_error(&self) -> usize {
            self.count(OutcomeStatus::Error)
        }

        pub fn count_crash(&self) -> usize {
            self.count(OutcomeStatus::Crash)
        }

        pub fn divergence_count(&self) -> usize {
            self.count_warn() + self.count_error() + self.count_crash()
        }

        fn count(&self, status: OutcomeStatus) -> usize {
            self.outcomes
                .iter()
                .filter(|outcome| outcome.status == status)
                .count()
        }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
    pub struct FixReport {
        pub before: Report,
        pub after: Report,
        pub attempted: usize,
        pub applied: usize,
        pub skipped: usize,
        pub failures: Vec<String>,
    }
}

pub mod dev_health {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum PluginRuntimeStatus {
        NotExpected,
        AutostartBlocked,
        OnDemand {
            pid: u32,
        },
        Down {
            consecutive_failures: u32,
            suppressed: bool,
        },
        Probation {
            pid: u32,
            consecutive_failures: u32,
        },
        Stable {
            pid: u32,
        },
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct PluginHealth {
        pub plugin_id: String,
        pub status: PluginRuntimeStatus,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
    pub struct HealthSnapshot {
        #[serde(default)]
        pub tick: u64,
        #[serde(default)]
        pub process_pid: u32,
        #[serde(default)]
        pub role: String,
        #[serde(default)]
        pub bind_port: u16,
        #[serde(default)]
        pub daemon_autostart_held: bool,
        #[serde(default)]
        pub generation_id: Option<String>,
        #[serde(default)]
        pub plugins: Vec<PluginHealth>,
    }
}

pub mod launcher {
    pub const APP_ID: &str = "qol-tray-launcher";
    pub const WINDOW_TITLE: &str = "qol-launcher";
    pub const BOOSTS_FILE_NAME: &str = "qol-launcher-boosts.toml";
    pub const MATCH_MARKERS: &[&str] = &[APP_ID, "plugin-launcher", WINDOW_TITLE, "qol launcher"];
}

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

    #[test]
    fn launcher_match_markers_include_the_window_identity() {
        assert!(launcher::MATCH_MARKERS.contains(&launcher::APP_ID));
        assert!(launcher::MATCH_MARKERS.contains(&launcher::WINDOW_TITLE));
    }

    #[test]
    fn health_status_serde_round_trips_every_variant() {
        use dev_health::PluginRuntimeStatus;

        let cases = [
            (
                PluginRuntimeStatus::NotExpected,
                r#"{"state":"not_expected"}"#,
            ),
            (
                PluginRuntimeStatus::AutostartBlocked,
                r#"{"state":"autostart_blocked"}"#,
            ),
            (
                PluginRuntimeStatus::OnDemand { pid: 12 },
                r#"{"state":"on_demand","pid":12}"#,
            ),
            (
                PluginRuntimeStatus::Down {
                    consecutive_failures: 5,
                    suppressed: true,
                },
                r#"{"state":"down","consecutive_failures":5,"suppressed":true}"#,
            ),
            (
                PluginRuntimeStatus::Probation {
                    pid: 12,
                    consecutive_failures: 1,
                },
                r#"{"state":"probation","pid":12,"consecutive_failures":1}"#,
            ),
            (
                PluginRuntimeStatus::Stable { pid: 12 },
                r#"{"state":"stable","pid":12}"#,
            ),
        ];
        for (status, expected_json) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected_json, "serialize {status:?}");
            let back: PluginRuntimeStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status, "round trip {expected_json}");
        }
    }

    #[test]
    fn doctor_wire_reports_round_trip_with_typed_counts() {
        use doctor_wire::{FixReport, Outcome, OutcomeStatus, Report};

        let report = Report::new(vec![
            Outcome {
                id: "healthy".to_string(),
                status: OutcomeStatus::Ok,
                message: "healthy".to_string(),
                fix_available: false,
            },
            Outcome {
                id: "drift".to_string(),
                status: OutcomeStatus::Warn,
                message: "drifted".to_string(),
                fix_available: true,
            },
            Outcome {
                id: "broken".to_string(),
                status: OutcomeStatus::Error,
                message: "broken".to_string(),
                fix_available: false,
            },
            Outcome {
                id: "panic".to_string(),
                status: OutcomeStatus::Crash,
                message: "panicked".to_string(),
                fix_available: false,
            },
        ]);
        assert_eq!(report.count_ok(), 1);
        assert_eq!(report.count_warn(), 1);
        assert_eq!(report.count_error(), 1);
        assert_eq!(report.count_crash(), 1);
        assert_eq!(report.divergence_count(), 3);

        let fix_report = FixReport {
            before: report.clone(),
            after: Report::new(vec![report.outcomes[0].clone()]),
            attempted: 1,
            applied: 1,
            skipped: 0,
            failures: Vec::new(),
        };
        let json = serde_json::to_string(&fix_report).unwrap();
        let decoded: FixReport = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, fix_report);
    }
}
