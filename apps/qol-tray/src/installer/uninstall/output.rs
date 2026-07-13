use anyhow::Result;
use std::io::Write;

use super::model::{ActionReport, ActionResult, ReportStatus, UninstallReport};

pub(super) fn print(report: &UninstallReport, json: bool) -> Result<()> {
    if json {
        return print_json(report);
    }
    print_human(report);
    Ok(())
}

fn print_json(report: &UninstallReport) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, report)?;
    writeln!(output)?;
    Ok(())
}

fn print_human(report: &UninstallReport) {
    let title = if report.dry_run {
        "QoL Tray uninstall plan"
    } else {
        match report.status {
            ReportStatus::Complete => "QoL Tray uninstall complete",
            ReportStatus::Partial => "QoL Tray uninstall incomplete",
            ReportStatus::Planned => "QoL Tray uninstall plan",
        }
    };
    println!("{title}");
    println!(
        "User data: {}",
        if report.purge_data {
            "purge"
        } else {
            "preserve"
        }
    );
    for action in &report.actions {
        print_action(action);
    }
    for item in &report.preserved {
        println!(
            "  [preserved] {}: {} ({})",
            item.id.label(),
            item.path.display(),
            item.reason
        );
    }
    for warning in &report.warnings {
        eprintln!("Warning: {warning}");
    }
    if report.dry_run {
        println!("Dry run only; no changes were made.");
    }
}

fn print_action(action: &ActionReport) {
    println!(
        "  [{:>18}] {}: {}",
        result_label(action.result),
        action.id.label(),
        action.target
    );
    if let Some(error) = &action.error {
        eprintln!("    {error}");
    }
}

fn result_label(result: ActionResult) -> &'static str {
    match result {
        ActionResult::Planned => "planned",
        ActionResult::Removed => "removed",
        ActionResult::Updated => "updated",
        ActionResult::Stopped => "stopped",
        ActionResult::AlreadyAbsent => "already absent",
        ActionResult::SkippedUnowned => "skipped unowned",
        ActionResult::SkippedDependency => "skipped dependency",
        ActionResult::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{
        ActionReport, ArtifactId, Operation, PreservedReport, TargetState, REPORT_SCHEMA_VERSION,
    };
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn json_shape_is_stable_and_machine_readable() {
        let report = UninstallReport {
            schema_version: REPORT_SCHEMA_VERSION,
            platform: "linux".to_string(),
            dry_run: true,
            purge_data: false,
            status: ReportStatus::Planned,
            actions: vec![ActionReport {
                id: ArtifactId::Binary,
                operation: Operation::RemoveFile,
                target: "/home/u/.local/bin/qol-tray".to_string(),
                state: TargetState::Present,
                result: ActionResult::Planned,
                error: None,
            }],
            preserved: vec![PreservedReport {
                id: ArtifactId::ConfigDirectory,
                path: PathBuf::from("/home/u/.config/qol-tray"),
                state: TargetState::Present,
                reason: "user config".to_string(),
            }],
            warnings: Vec::new(),
        };

        let value = serde_json::to_value(report).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "schema_version": 1,
                "platform": "linux",
                "dry_run": true,
                "purge_data": false,
                "status": "planned",
                "actions": [{
                    "id": "binary",
                    "operation": "remove_file",
                    "target": "/home/u/.local/bin/qol-tray",
                    "state": "present",
                    "result": "planned"
                }],
                "preserved": [{
                    "id": "config_directory",
                    "path": "/home/u/.config/qol-tray",
                    "state": "present",
                    "reason": "user config"
                }],
                "warnings": []
            })
        );
    }
}
