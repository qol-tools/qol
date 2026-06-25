use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, Severity,
};
use crate::plugins::manifest::PluginManifest;
use std::collections::BTreeMap;

const ID: &str = "plugin_port_collisions";

pub(super) struct PluginPortCollisionsCheck;

impl DoctorCheck for PluginPortCollisionsCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Plugin port collisions", CheckCategory::Plugins)
    }

    fn run(&self, ctx: &DoctorContext) -> CheckReport {
        let registry = match ctx.registry() {
            Ok(r) => r,
            Err(e) => return CheckReport::ok(format!("could not load registry: {e}")),
        };
        let ports: Vec<(String, u16)> = registry
            .entries
            .iter()
            .filter_map(|entry| {
                let manifest_path = entry.active.path.join("plugin.toml");
                let content = std::fs::read_to_string(&manifest_path).ok()?;
                let manifest: PluginManifest = toml::from_str(&content).ok()?;
                let port = manifest.daemon?.port?;
                Some((entry.id.clone(), port))
            })
            .collect();
        report_from_ports(&ports)
    }
}

fn collisions(ports: &[(String, u16)]) -> Vec<(u16, Vec<String>)> {
    let mut by_port: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for (id, port) in ports {
        by_port.entry(*port).or_default().push(id.clone());
    }
    by_port
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(port, mut ids)| {
            ids.sort();
            (port, ids)
        })
        .collect()
}

fn report_from_ports(ports: &[(String, u16)]) -> CheckReport {
    let collided = collisions(ports);
    if collided.is_empty() {
        return CheckReport::ok(format!(
            "{} plugin daemon port(s), no collisions",
            ports.len()
        ));
    }
    let issues: Vec<DoctorIssue> = collided
        .iter()
        .map(|(port, ids)| DoctorIssue {
            code: ID,
            severity: Severity::Warn,
            message: format!("port {port} claimed by {}", ids.join(", ")),
            evidence: ids.clone(),
        })
        .collect();
    CheckReport {
        summary: format!("{} plugin daemon port collision(s)", collided.len()),
        issues,
        advice: vec!["give each plugin a unique [daemon] port in its plugin.toml".to_string()],
        fixes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(pairs: &[(&str, u16)]) -> Vec<(String, u16)> {
        pairs.iter().map(|(id, p)| (id.to_string(), *p)).collect()
    }

    #[test]
    fn no_ports_yields_ok_without_issues_or_fixes() {
        let report = report_from_ports(&[]);
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
        assert_eq!(report.summary, "0 plugin daemon port(s), no collisions");
    }

    #[test]
    fn unique_ports_yield_ok_without_issues() {
        let report = report_from_ports(&ports(&[("plugin-a", 42710), ("plugin-b", 42720)]));
        assert!(report.issues.is_empty(), "unique ports must not warn");
        assert!(report.advice.is_empty());
        assert_eq!(report.summary, "2 plugin daemon port(s), no collisions");
    }

    #[test]
    fn duplicate_port_emits_one_warn_listing_both_plugins_and_never_a_fix() {
        let report = report_from_ports(&ports(&[
            ("plugin-b", 42710),
            ("plugin-a", 42710),
            ("plugin-c", 42720),
        ]));
        assert_eq!(report.issues.len(), 1, "one issue per collided port");
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert_eq!(report.issues[0].code, ID);
        assert_eq!(
            report.issues[0].message, "port 42710 claimed by plugin-a, plugin-b",
            "collided ids are sorted and listed"
        );
        assert!(report.fixes.is_empty(), "collisions are never auto-fixable");
        assert!(
            !report.advice.is_empty(),
            "collisions carry remediation advice"
        );
    }

    #[test]
    fn collisions_groups_each_shared_port_separately() {
        let result = collisions(&ports(&[
            ("plugin-a", 42710),
            ("plugin-b", 42710),
            ("plugin-c", 42720),
            ("plugin-d", 42720),
            ("plugin-e", 42730),
        ]));
        assert_eq!(
            result,
            vec![
                (42710, vec!["plugin-a".to_string(), "plugin-b".to_string()]),
                (42720, vec!["plugin-c".to_string(), "plugin-d".to_string()]),
            ],
            "only shared ports surface, sorted by port then id"
        );
    }
}
