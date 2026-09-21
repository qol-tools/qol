use super::super::diagnosis::FixAction;
use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, Severity,
};
use std::path::PathBuf;

const ID: &str = "orphan_plugin_configs";

pub(super) struct OrphanPluginConfigsCheck;

impl DoctorCheck for OrphanPluginConfigsCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Orphan plugin configs", CheckCategory::Plugins)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let orphans = crate::config_drain::orphan_config_paths();
        report_from_orphans(&orphans)
    }
}

fn report_from_orphans(orphans: &[(String, PathBuf)]) -> CheckReport {
    if orphans.is_empty() {
        return CheckReport::ok("no orphan plugin config files");
    }
    let summary = format!(
        "found {} orphan plugin config file(s) outside the host store",
        orphans.len()
    );
    let detail = orphans
        .iter()
        .map(|(id, path)| format!("{id} -> {}", path.display()))
        .collect::<Vec<_>>()
        .join(", ");
    let issue = DoctorIssue {
        code: ID,
        severity: Severity::Warn,
        message: format!("orphan plugin config files: {detail}"),
        evidence: orphans
            .iter()
            .map(|(_, path)| path.display().to_string())
            .collect(),
    };
    CheckReport {
        summary,
        issues: vec![issue],
        advice: Vec::new(),
        fixes: vec![FixAction::DrainOrphanPluginConfigs],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_drain::classify_orphans;
    use crate::doctor::diagnosis::FixApplicability;

    #[test]
    fn empty_orphans_yields_ok_with_no_fix() {
        let report = report_from_orphans(&[]);
        assert!(report.issues.is_empty(), "no issues for clean state");
        assert!(report.fixes.is_empty(), "no fixes for clean state");
        assert_eq!(report.summary, "no orphan plugin config files");
    }

    #[test]
    fn non_empty_orphans_emit_one_warn_and_one_drain_fix() {
        let orphans = vec![
            (
                "plugin-foo".to_string(),
                PathBuf::from("/home/u/.config/qol-tray/plugins/plugin-foo/config.json"),
            ),
            (
                "plugin-bar".to_string(),
                PathBuf::from("/home/u/.local/share/qol-tray/plugins/plugin-bar/config.json"),
            ),
        ];
        let report = report_from_orphans(&orphans);
        assert_eq!(report.issues.len(), 1, "exactly one warn issue");
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert_eq!(report.issues[0].code, ID);
        assert!(
            report.issues[0].message.contains("plugin-foo")
                && report.issues[0].message.contains("plugin-bar"),
            "issue lists both plugin ids: {}",
            report.issues[0].message
        );
        assert!(matches!(
            report.fixes.as_slice(),
            [FixAction::DrainOrphanPluginConfigs]
        ));
        assert_eq!(
            report.fixes[0].applicability(),
            FixApplicability::SafeAutomatic,
            "drain fix must be safe-automatic"
        );
    }

    #[test]
    fn classify_orphans_keeps_only_files_outside_plugins_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        std::fs::create_dir_all(plugins_dir.join("plugin-a")).unwrap();
        let inside = plugins_dir.join("plugin-a").join("config.json");
        std::fs::write(&inside, "{}").unwrap();

        let outside_dir = tmp.path().join("data").join("plugins").join("plugin-a");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let outside = outside_dir.join("config.json");
        std::fs::write(&outside, "{}").unwrap();

        let missing = tmp.path().join("data2").join("plugins").join("ghost.json");

        let candidates = vec![inside.clone(), outside.clone(), missing.clone()];
        let kept = classify_orphans(&candidates, &plugins_dir);
        assert_eq!(
            kept,
            vec![outside.clone()],
            "only the existing outside-plugins-dir path survives"
        );
    }

    #[test]
    fn classify_orphans_empty_inputs_yield_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        let kept = classify_orphans(&[], &plugins_dir);
        assert!(kept.is_empty());
    }
}
