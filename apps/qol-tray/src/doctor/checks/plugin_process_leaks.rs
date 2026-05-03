use super::super::diagnosis::{ok_outcome, warn_outcome, Diagnosis, FixAction};
use crate::plugins::daemon_tracker::ManagedProcess;

const ID: &str = "plugin_process_leaks";

pub(super) fn check() -> Diagnosis {
    diagnose(crate::plugins::daemon_tracker::leaked_processes())
}

fn diagnose(leaks: Vec<ManagedProcess>) -> Diagnosis {
    if leaks.is_empty() {
        return ok_outcome(ID, "no leaked plugin processes detected".to_string());
    }

    warn_outcome(
        ID,
        format!("leaked plugin processes detected: {}", format_leaks(&leaks)),
        Some(FixAction::KillPluginProcessLeaks),
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
    use crate::doctor::OutcomeStatus;
    use std::path::PathBuf;

    #[test]
    fn diagnose_ok_when_no_leaks() {
        let diagnosis = diagnose(Vec::new());

        assert_eq!(diagnosis.outcome.status, OutcomeStatus::Ok);
        assert!(!diagnosis.outcome.fix_available);
    }

    #[test]
    fn diagnose_warns_with_safe_fix_when_leaks_exist() {
        let diagnosis = diagnose(vec![ManagedProcess {
            pid: 42,
            executable: PathBuf::from("/plugins/plugin-lights/daemon"),
        }]);

        assert_eq!(diagnosis.outcome.status, OutcomeStatus::Warn);
        assert!(diagnosis.outcome.fix_available);
        assert!(diagnosis.outcome.message.contains("42"));
        assert!(diagnosis.outcome.message.contains("plugin-lights"));
    }
}
