use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use serde_json::Value;
use std::path::{Path, PathBuf};

const ID: &str = "plugin_config_shadows";

pub(super) struct PluginConfigShadowsCheck;

impl DoctorCheck for PluginConfigShadowsCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Plugin config shadows", CheckCategory::Plugins)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let context = match build_context() {
            Ok(context) => context,
            Err(error) => return CheckReport::error(error.to_string(), ID),
        };
        diagnose(collect_findings(&context.roots, &context.canonical_root))
    }
}

struct Context {
    roots: Vec<PathBuf>,
    canonical_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Finding {
    plugin_id: String,
    shadow_path: PathBuf,
    canonical_path: PathBuf,
}

fn build_context() -> anyhow::Result<Context> {
    let plugins_dir = crate::paths::plugins_dir()?;
    let canonical_root = plugins_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("plugins directory has no parent"))?;
    Ok(Context {
        roots: qol_config::config_roots(),
        canonical_root,
    })
}

fn diagnose(findings: Vec<Finding>) -> CheckReport {
    if findings.is_empty() {
        return CheckReport::ok("no plugin config shadows detected");
    }
    CheckReport::warn(
        format_message(&findings),
        ID,
        findings
            .iter()
            .map(|finding| FixAction::ArchivePluginConfigShadow {
                path: finding.shadow_path.clone(),
                backup_path: backup_path_for(&finding.shadow_path),
            })
            .collect(),
    )
}

fn collect_findings(roots: &[PathBuf], canonical_root: &Path) -> Vec<Finding> {
    let canonical_plugins_dir = canonical_root.join("plugins");
    let mut findings = Vec::new();
    for root in roots_before_canonical(roots, canonical_root) {
        findings.extend(collect_root_findings(&root, &canonical_plugins_dir));
    }
    findings.sort_by(|a, b| {
        a.plugin_id
            .cmp(&b.plugin_id)
            .then_with(|| a.shadow_path.cmp(&b.shadow_path))
    });
    findings
}

fn roots_before_canonical(roots: &[PathBuf], canonical_root: &Path) -> Vec<PathBuf> {
    let mut before = Vec::new();
    for root in roots {
        if same_path(root, canonical_root) {
            return before;
        }
        if !before.iter().any(|existing| same_path(existing, root)) {
            before.push(root.clone());
        }
    }
    before
}

fn collect_root_findings(root: &Path, canonical_plugins_dir: &Path) -> Vec<Finding> {
    let plugins_dir = root.join("plugins");
    let Ok(entries) = std::fs::read_dir(&plugins_dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|plugin_dir| finding_for_plugin_dir(&plugin_dir, canonical_plugins_dir))
        .collect()
}

fn finding_for_plugin_dir(plugin_dir: &Path, canonical_plugins_dir: &Path) -> Option<Finding> {
    if !is_regular_dir(plugin_dir) {
        return None;
    }
    let plugin_id = plugin_dir.file_name()?.to_str()?;
    if !crate::paths::is_safe_path_component(plugin_id) {
        return None;
    }
    let shadow_path = plugin_dir.join("config.json");
    if !is_regular_file(&shadow_path) {
        return None;
    }
    let canonical_path = canonical_plugins_dir.join(plugin_id).join("config.json");
    if same_path(&shadow_path, &canonical_path) || !is_regular_file(&canonical_path) {
        return None;
    }
    read_json_value(&shadow_path)?;
    read_json_value(&canonical_path)?;
    Some(Finding {
        plugin_id: plugin_id.to_string(),
        shadow_path,
        canonical_path,
    })
}

fn read_json_value(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn is_regular_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| !metadata.file_type().is_symlink() && metadata.is_dir())
        .unwrap_or(false)
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| !metadata.file_type().is_symlink() && metadata.is_file())
        .unwrap_or(false)
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let Ok(a) = a.canonicalize() else {
        return false;
    };
    let Ok(b) = b.canonicalize() else {
        return false;
    };
    a == b
}

fn format_message(findings: &[Finding]) -> String {
    if findings.len() == 1 {
        let finding = &findings[0];
        return format!(
            "{} config is shadowed by {}",
            finding.plugin_id,
            finding.shadow_path.display()
        );
    }
    let plugins = findings
        .iter()
        .map(|finding| finding.plugin_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("plugin configs are shadowed: {plugins}")
}

fn backup_path_for(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.json");
    (0..1000)
        .map(|index| backup_candidate(parent, name, index))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| {
            parent.join(format!("{name}.qol-tray-shadow-{}.bak", std::process::id()))
        })
}

fn backup_candidate(parent: &Path, name: &str, index: usize) -> PathBuf {
    if index == 0 {
        return parent.join(format!("{name}.qol-tray-shadow.bak"));
    }
    parent.join(format!("{name}.qol-tray-shadow-{index}.bak"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn write_json(path: &Path, value: Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    #[test]
    fn detects_active_install_config_shadowing_canonical_keyremap_rules() {
        let tmp = TempDir::new().unwrap();
        let active = tmp.path().join("data/qol-tray/installs/install-123");
        let canonical = tmp.path().join("config/qol-tray");
        let shadow_path = active.join("plugins/plugin-keyremap/config.json");
        let canonical_path = canonical.join("plugins/plugin-keyremap/config.json");
        write_json(
            &shadow_path,
            json!({
                "enabled": true,
                "char_rules": []
            }),
        );
        write_json(
            &canonical_path,
            json!({
                "enabled": true,
                "char_rules": [{
                    "from_mods": ["ralt"],
                    "from_key": "2",
                    "to_char": "@",
                    "global": true
                }]
            }),
        );

        let findings = collect_findings(&[active.clone(), canonical.clone()], &canonical);

        assert_eq!(
            findings,
            vec![Finding {
                plugin_id: "plugin-keyremap".to_string(),
                shadow_path,
                canonical_path,
            }]
        );
        let report = diagnose(findings);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.fixes.len(), 1);
    }

    #[test]
    fn detects_matching_active_install_config_as_future_shadow() {
        let tmp = TempDir::new().unwrap();
        let active = tmp.path().join("data/qol-tray/installs/install-123");
        let canonical = tmp.path().join("config/qol-tray");
        let shadow_path = active.join("plugins/plugin-keyremap/config.json");
        let canonical_path = canonical.join("plugins/plugin-keyremap/config.json");
        let value = json!({ "enabled": true, "char_rules": [] });
        write_json(&shadow_path, value.clone());
        write_json(&canonical_path, value);

        let findings = collect_findings(&[active, canonical.clone()], &canonical);

        assert_eq!(
            findings,
            vec![Finding {
                plugin_id: "plugin-keyremap".to_string(),
                shadow_path,
                canonical_path,
            }]
        );
    }

    #[test]
    fn ignores_active_install_config_when_canonical_config_is_missing() {
        let tmp = TempDir::new().unwrap();
        let active = tmp.path().join("data/qol-tray/installs/install-123");
        let canonical = tmp.path().join("config/qol-tray");
        write_json(
            &active.join("plugins/plugin-keyremap/config.json"),
            json!({ "enabled": true, "char_rules": [] }),
        );

        let findings = collect_findings(&[active, canonical.clone()], &canonical);

        assert!(findings.is_empty(), "findings: {findings:?}");
    }

    #[test]
    fn stops_scanning_once_the_canonical_root_is_reached() {
        let tmp = TempDir::new().unwrap();
        let canonical = tmp.path().join("config/qol-tray");
        let later = tmp.path().join("later/qol-tray");
        write_json(
            &canonical.join("plugins/plugin-keyremap/config.json"),
            json!({ "enabled": true, "char_rules": ["real"] }),
        );
        write_json(
            &later.join("plugins/plugin-keyremap/config.json"),
            json!({ "enabled": true, "char_rules": [] }),
        );

        let findings = collect_findings(&[canonical.clone(), later], &canonical);

        assert!(findings.is_empty(), "findings: {findings:?}");
    }
}
