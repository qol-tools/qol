use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use super::cargo_target::workspace_root;
use std::path::Path;
use std::process::Command;

const ID: &str = "single_source_guard";
const SCRIPT_PATH: &str = ".githooks/single-source-guard.mjs";

pub(super) struct SingleSourceGuardCheck;

impl DoctorCheck for SingleSourceGuardCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Single-source guard", CheckCategory::Runtime)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let Some(workspace) = workspace_root() else {
            return CheckReport::ok(
                "workspace root not found; skipping single-source guard".to_string(),
            );
        };
        if !workspace.join(SCRIPT_PATH).is_file() {
            return CheckReport::ok("single-source guard script not found; skipping".to_string());
        }
        report_for(guard_status(&workspace))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum GuardStatus {
    Clean,
    Rejected(String),
    Unavailable(String),
}

fn guard_status(workspace: &Path) -> GuardStatus {
    match Command::new("node")
        .arg(SCRIPT_PATH)
        .current_dir(workspace)
        .output()
    {
        Ok(output) if output.status.success() => GuardStatus::Clean,
        Ok(output) => GuardStatus::Rejected(first_rejection(&output.stderr)),
        Err(error) => GuardStatus::Unavailable(error.to_string()),
    }
}

fn report_for(status: GuardStatus) -> CheckReport {
    match status {
        GuardStatus::Clean => CheckReport::ok("single-source guard is clean".to_string()),
        GuardStatus::Rejected(detail) => CheckReport::warn(
            format!("single-source guard rejected repo state: {detail}"),
            ID,
            Vec::new(),
        ),
        GuardStatus::Unavailable(reason) => CheckReport::ok(format!(
            "node unavailable, skipping single-source guard: {reason}"
        )),
    }
}

fn first_rejection(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.lines()
        .map(str::trim)
        .find(|line| is_repo_hit(line))
        .or_else(|| text.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("run `node .githooks/single-source-guard.mjs`")
        .to_string()
}

fn is_repo_hit(line: &str) -> bool {
    let Some((path, _)) = line.split_once(':') else {
        return false;
    };
    path.starts_with("apps/")
        || path.starts_with("libs/")
        || path.starts_with("plugins/")
        || path.starts_with("tools/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_is_ok_without_fix() {
        let report = report_for(GuardStatus::Clean);
        assert!(report.issues.is_empty(), "clean must not warn");
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn rejection_warns_without_fix() {
        let report = report_for(GuardStatus::Rejected(
            "apps/qol-tray/src/plugins/daemon_health.rs:167:42700".to_string(),
        ));
        assert_eq!(report.issues.len(), 1);
        assert!(report.fixes.is_empty());
        assert!(
            report
                .summary
                .contains("apps/qol-tray/src/plugins/daemon_health.rs:167"),
            "summary: {}",
            report.summary
        );
    }

    #[test]
    fn unavailable_is_ok_without_fix() {
        let report = report_for(GuardStatus::Unavailable("no node".to_string()));
        assert!(report.issues.is_empty(), "missing node must not block");
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn first_rejection_prefers_repo_hit() {
        let stderr = b"
  single-source guard rejected

  cross-process constants must come from their single source
  offending occurrences:
    apps/qol-tray/src/plugins/daemon_health.rs:167:42700

  fix:
    - host constants: qol_conventions::DEFAULT_PORT
";
        assert_eq!(
            first_rejection(stderr),
            "apps/qol-tray/src/plugins/daemon_health.rs:167:42700"
        );
    }

    #[test]
    fn first_rejection_falls_back_to_first_non_empty_line() {
        assert_eq!(
            first_rejection(b"\n  node failed\n"),
            "node failed".to_string()
        );
    }
}
