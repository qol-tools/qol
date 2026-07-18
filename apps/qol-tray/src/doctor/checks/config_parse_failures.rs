use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, Severity,
};
use std::path::PathBuf;

const ID: &str = "config_parse_failures";

pub(super) struct ConfigParseFailuresCheck;

impl DoctorCheck for ConfigParseFailuresCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Plugin config parse failures", CheckCategory::Plugins)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        report_from_markers(&collect_markers(&qol_config::config_roots()))
    }
}

struct ParseFailureMarker {
    plugin_id: String,
    marker_path: PathBuf,
    error: String,
}

fn collect_markers(roots: &[PathBuf]) -> Vec<ParseFailureMarker> {
    let mut markers = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root.join("plugins")) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let marker_path =
                qol_config::parse_error_marker_path(&entry.path().join("config.json"));
            let Ok(error) = std::fs::read_to_string(&marker_path) else {
                continue;
            };
            markers.push(ParseFailureMarker {
                plugin_id: entry.file_name().to_string_lossy().into_owned(),
                marker_path,
                error: error.trim().to_string(),
            });
        }
    }
    markers
}

fn report_from_markers(markers: &[ParseFailureMarker]) -> CheckReport {
    if markers.is_empty() {
        return CheckReport::ok("all plugin configs parse");
    }
    let summary = format!(
        "{} plugin config(s) failed to parse; those daemons run compiled defaults",
        markers.len()
    );
    let issues = markers
        .iter()
        .map(|marker| DoctorIssue {
            code: ID,
            severity: Severity::Warn,
            message: format!("{}: {}", marker.plugin_id, marker.error),
            evidence: vec![marker.marker_path.display().to_string()],
        })
        .collect();
    CheckReport {
        summary,
        issues,
        advice: vec![
            "fix the config file or re-save the plugin's settings; the daemon clears the marker on its next successful load".to_string(),
        ],
        fixes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_markers_finds_marker_files_across_roots() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root_a = tmp.path().join("a");
        let root_b = tmp.path().join("b");
        let foo = root_a.join("plugins").join("plugin-foo");
        let bar = root_b.join("plugins").join("plugin-bar");
        let clean = root_a.join("plugins").join("plugin-clean");
        for dir in [&foo, &bar, &clean] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(foo.join("config.json.parse-error"), "invalid type\n").unwrap();
        std::fs::write(bar.join("config.json.parse-error"), "expected value").unwrap();
        std::fs::write(clean.join("config.json"), "{}").unwrap();

        let mut markers = collect_markers(&[root_a, root_b, tmp.path().join("missing")]);
        markers.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));

        let found: Vec<(&str, &str)> = markers
            .iter()
            .map(|marker| (marker.plugin_id.as_str(), marker.error.as_str()))
            .collect();
        assert_eq!(
            found,
            vec![
                ("plugin-bar", "expected value"),
                ("plugin-foo", "invalid type"),
            ]
        );
    }

    #[test]
    fn report_reflects_marker_presence() {
        let clean = report_from_markers(&[]);
        assert!(clean.issues.is_empty(), "no issues without markers");
        assert!(clean.fixes.is_empty(), "advice-only check has no fixes");

        let markers = [ParseFailureMarker {
            plugin_id: "plugin-foo".to_string(),
            marker_path: PathBuf::from("/a/b/plugins/plugin-foo/config.json.parse-error"),
            error: "invalid type".to_string(),
        }];
        let report = report_from_markers(&markers);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert!(
            report.issues[0].message.contains("plugin-foo")
                && report.issues[0].message.contains("invalid type"),
            "issue names the plugin and error: {}",
            report.issues[0].message
        );
        assert!(
            report.summary.contains("compiled defaults"),
            "summary states the consequence: {}",
            report.summary
        );
        assert!(report.fixes.is_empty(), "advice-only check has no fixes");
    }
}
