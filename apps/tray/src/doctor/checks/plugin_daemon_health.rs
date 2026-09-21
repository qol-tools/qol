use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::plugins::daemon_health::{HealthSnapshot, PluginRuntimeStatus};
use std::path::Path;

const ID: &str = "plugin_daemon_health";

pub(super) struct PluginDaemonHealthCheck;

impl DoctorCheck for PluginDaemonHealthCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Plugin daemon health", CheckCategory::Runtime)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        diagnose(&crate::plugins::daemon_health::default_file_path())
    }
}

fn diagnose(path: &Path) -> CheckReport {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return CheckReport::ok("no daemon health snapshot (qol-tray not running here)");
    };
    let snapshot: HealthSnapshot = match serde_json::from_str(&raw) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return CheckReport::ok(format!("unreadable daemon health snapshot: {error}"))
        }
    };
    if !crate::process_utils::is_pid_alive(snapshot.process_pid as i32) {
        return CheckReport::ok("stale daemon health snapshot (qol-tray not running)");
    }
    let suppressed: Vec<&str> = snapshot
        .plugins
        .iter()
        .filter(|plugin| {
            matches!(
                plugin.status,
                PluginRuntimeStatus::Down {
                    suppressed: true,
                    ..
                }
            )
        })
        .map(|plugin| plugin.plugin_id.as_str())
        .collect();
    if suppressed.is_empty() {
        return CheckReport::ok("no crash-looped plugin daemons");
    }
    CheckReport::warn(
        format!(
            "daemon crash loop suppressed for: {}",
            suppressed.join(", ")
        ),
        ID,
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::daemon_health::PluginHealth;

    const DEAD_PID: u32 = 99_999_999;

    fn snapshot(process_pid: u32, statuses: Vec<(&str, PluginRuntimeStatus)>) -> String {
        serde_json::to_string(&HealthSnapshot {
            tick: 1,
            process_pid,
            plugins: statuses
                .into_iter()
                .map(|(id, status)| PluginHealth {
                    plugin_id: id.to_string(),
                    status,
                })
                .collect(),
            ..HealthSnapshot::default()
        })
        .unwrap()
    }

    #[test]
    fn diagnose_table() {
        let alive = std::process::id();
        let suppressed = PluginRuntimeStatus::Down {
            consecutive_failures: 5,
            suppressed: true,
        };
        let transient = PluginRuntimeStatus::Down {
            consecutive_failures: 1,
            suppressed: false,
        };
        let cases = [
            ("missing file", None, false, ""),
            ("corrupt file", Some("not json".to_string()), false, ""),
            (
                "stale writer pid",
                Some(snapshot(DEAD_PID, vec![("plugin-foo", suppressed.clone())])),
                false,
                "",
            ),
            (
                "suppressed plugin warns",
                Some(snapshot(alive, vec![("plugin-foo", suppressed)])),
                true,
                "plugin-foo",
            ),
            (
                "transient down is ok",
                Some(snapshot(alive, vec![("plugin-foo", transient)])),
                false,
                "",
            ),
        ];
        for (label, contents, expect_warn, expect_in_summary) in cases {
            let tmp = tempfile::TempDir::new().unwrap();
            let path = tmp.path().join("daemon-health.json");
            if let Some(contents) = contents {
                std::fs::write(&path, contents).unwrap();
            }
            let report = diagnose(&path);
            assert_eq!(!report.issues.is_empty(), expect_warn, "{label}");
            if expect_warn {
                assert!(
                    report.summary.contains(expect_in_summary),
                    "{label}: {}",
                    report.summary
                );
            }
        }
    }
}
