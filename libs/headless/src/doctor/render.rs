use super::{DoctorAggregateReport, DoctorCheckResult, DoctorReport, DoctorStatus};

pub(crate) fn render_report(report: &DoctorReport) -> String {
    let mut lines = vec![format!(
        "{} doctor: {}",
        report.plugin_id,
        report.status.as_str()
    )];

    push_results(&mut lines, &report.checks, "");
    trailing_newline(lines.join("\n"))
}

pub(crate) fn render_aggregate_report(app_id: &str, report: &DoctorAggregateReport) -> String {
    let mut lines = vec![
        format!("{app_id} doctor: {}", report.status.as_str()),
        String::new(),
        format!(
            "Host {}: {}",
            report.host.plugin_id,
            report.host.status.as_str()
        ),
    ];
    push_results(&mut lines, &report.host.checks, "  ");

    for plugin in &report.plugins {
        lines.extend([
            String::new(),
            format!("Plugin {}: {}", plugin.plugin_id, plugin.status.as_str()),
        ]);
        if !plugin.diagnostics.is_empty() {
            lines.push("  Diagnostics:".to_string());
            push_results(&mut lines, &plugin.diagnostics, "    ");
        }
        if let Some(plugin_report) = &plugin.report {
            lines.push(format!(
                "  Report {}: {}",
                plugin_report.plugin_id,
                plugin_report.status.as_str()
            ));
            push_results(&mut lines, &plugin_report.checks, "    ");
        }
    }

    trailing_newline(lines.join("\n"))
}

pub(crate) fn aggregate_exit_code(status: DoctorStatus) -> u8 {
    match status {
        DoctorStatus::Ok => 0,
        DoctorStatus::Warn => 1,
        DoctorStatus::Fail => 2,
    }
}

fn push_results(lines: &mut Vec<String>, results: &[DoctorCheckResult], indent: &str) {
    for result in results {
        lines.push(format!(
            "{indent}[{}] {} - {}",
            result.status.as_str(),
            result.id,
            result.message
        ));
        if let Some(fix) = &result.fix {
            lines.push(format!("{indent}  fix: {fix}"));
        }
    }
}

fn trailing_newline(mut text: String) -> String {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}
