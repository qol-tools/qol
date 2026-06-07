use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::plugins::daemon_tracker::ManagedProcess;

const ID: &str = "plugin_process_leaks";

pub(super) struct PluginProcessLeaksCheck;

impl DoctorCheck for PluginProcessLeaksCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Plugin process leaks", CheckCategory::Plugins).group(&["dev-loop"])
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        diagnose(crate::plugins::daemon_tracker::leaked_processes())
    }
}

fn diagnose(leaks: Vec<ManagedProcess>) -> CheckReport {
    if leaks.is_empty() {
        return CheckReport::ok("no leaked plugin processes detected".to_string());
    }

    let message = format!("leaked plugin processes detected: {}", format_leaks(&leaks));
    CheckReport::warn(
        message,
        ID,
        vec![FixAction::KillPluginProcessLeaks { processes: leaks }],
    )
}

fn format_leaks(leaks: &[ManagedProcess]) -> String {
    leaks
        .iter()
        .map(|process| format!("{} ({})", process.pid, process.executable.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn diagnose_ok_when_no_leaks() {
        let report = diagnose(Vec::new());
        assert!(report.issues.is_empty(), "Ok report has no issues");
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn diagnose_warns_with_safe_fix_when_leaks_exist() {
        let report = diagnose(vec![ManagedProcess {
            pid: 42,
            executable: PathBuf::from("/plugins/plugin-lights/daemon"),
        }]);

        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.fixes.len(), 1);
        assert!(report.summary.contains("42"));
        assert!(report.summary.contains("plugin-lights"));
        assert!(matches!(
            report.fixes.as_slice(),
            [FixAction::KillPluginProcessLeaks { processes }]
                if processes.len() == 1 && processes[0].pid == 42
        ));
    }
}
