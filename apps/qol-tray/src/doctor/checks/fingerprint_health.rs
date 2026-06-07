use super::super::diagnosis::FixAction;
#[cfg(test)]
use super::super::diagnosis::FixApplicability;
use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, Severity,
};
use crate::plugins::registry::Entry;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

const ID: &str = "fingerprint_health";
const FINGERPRINTS_REL_PATH: &str = "dev/build-fingerprints.json";

pub(super) struct FingerprintHealthCheck;

impl DoctorCheck for FingerprintHealthCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Fingerprint health", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, ctx: &DoctorContext) -> CheckReport {
        let path = ctx.config_dir().join(FINGERPRINTS_REL_PATH);
        match file_state(&path) {
            FileState::Absent => CheckReport::ok("no build fingerprints recorded yet"),
            FileState::Corrupt(reason) => corrupt_report(&path, &reason),
            FileState::Valid(fingerprints) => diagnose_consistency(ctx, &fingerprints),
        }
    }
}

#[derive(Deserialize)]
struct FingerprintFile {
    #[serde(default)]
    fingerprints: HashMap<String, String>,
}

enum FileState {
    Absent,
    Corrupt(String),
    Valid(HashMap<String, String>),
}

fn file_state(path: &Path) -> FileState {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return FileState::Absent,
        Err(error) => return FileState::Corrupt(format!("unreadable: {error}")),
    };
    if content.trim().is_empty() {
        return FileState::Absent;
    }
    match serde_json::from_str::<FingerprintFile>(&content) {
        Ok(parsed) => FileState::Valid(parsed.fingerprints),
        Err(error) => FileState::Corrupt(error.to_string()),
    }
}

fn diagnose_consistency(
    ctx: &DoctorContext,
    fingerprints: &HashMap<String, String>,
) -> CheckReport {
    let registry = match ctx.registry() {
        Ok(registry) => registry,
        Err(error) => return CheckReport::ok(format!("could not read plugin registry: {error}")),
    };
    let registry_ids: BTreeSet<String> = registry
        .entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    let fingerprint_ids: BTreeSet<String> = fingerprints.keys().cloned().collect();
    let entry_by_id: HashMap<&str, &Entry> = registry
        .entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();

    let findings = classify(&fingerprint_ids, &registry_ids, &|id| {
        entry_by_id
            .get(id)
            .is_some_and(|entry| binary_missing(entry))
    });
    report_from_findings(&findings)
}

struct Findings {
    orphans: Vec<String>,
    phantoms: Vec<String>,
}

fn classify(
    fingerprint_ids: &BTreeSet<String>,
    registry_ids: &BTreeSet<String>,
    binary_missing: &dyn Fn(&str) -> bool,
) -> Findings {
    let mut orphans = Vec::new();
    let mut phantoms = Vec::new();
    for id in fingerprint_ids {
        if !registry_ids.contains(id) {
            orphans.push(id.clone());
        } else if binary_missing(id) {
            phantoms.push(id.clone());
        }
    }
    Findings { orphans, phantoms }
}

fn report_from_findings(findings: &Findings) -> CheckReport {
    if findings.orphans.is_empty() && findings.phantoms.is_empty() {
        return CheckReport::ok("build fingerprints are consistent with the registry");
    }
    let mut report = CheckReport {
        summary: summarize(findings),
        issues: Vec::new(),
        advice: Vec::new(),
        fixes: Vec::new(),
    };
    if !findings.orphans.is_empty() {
        report.issues.push(DoctorIssue::new(
            "orphan_fingerprints",
            Severity::Warn,
            format!(
                "orphan build fingerprints with no registry entry: {}",
                findings.orphans.join(", ")
            ),
        ));
        report.fixes.push(FixAction::PruneOrphanFingerprints {
            ids: findings.orphans.clone(),
        });
    }
    if !findings.phantoms.is_empty() {
        report.issues.push(DoctorIssue::new(
            "phantom_fingerprints",
            Severity::Warn,
            format!(
                "fingerprinted plugins whose binary is missing: {}",
                findings.phantoms.join(", ")
            ),
        ));
        report.advice.push(format!(
            "rebuild via `qol dev` or the in-app Recompile button: {}",
            findings.phantoms.join(", ")
        ));
    }
    report
}

fn summarize(findings: &Findings) -> String {
    format!(
        "fingerprint integrity issues: {} orphan, {} phantom",
        findings.orphans.len(),
        findings.phantoms.len()
    )
}

fn corrupt_report(path: &Path, reason: &str) -> CheckReport {
    let mut report = CheckReport::error(
        format!("build fingerprints file is corrupt: {reason}"),
        "corrupt_fingerprints",
    );
    report.advice.push(format!(
        "remove {} and rebuild; it is regenerated on the next build",
        path.display()
    ));
    report
}

fn binary_missing(entry: &Entry) -> bool {
    let manifest_path = entry.active.path.join("plugin.toml");
    let Ok(content) = std::fs::read_to_string(&manifest_path) else {
        return false;
    };
    let Ok(manifest) = toml::from_str::<crate::plugins::manifest::PluginManifest>(&content) else {
        return false;
    };
    if !manifest.plugin.supports_current_platform() {
        return false;
    }
    crate::plugins::validate_execution_contract(&entry.id, &manifest, &entry.active.path).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn orphan_when_fingerprint_id_absent_from_registry() {
        let findings = classify(
            &id_set(&["plugin-a", "plugin-orphan"]),
            &id_set(&["plugin-a"]),
            &|_| false,
        );
        assert_eq!(findings.orphans, vec!["plugin-orphan".to_string()]);
        assert!(findings.phantoms.is_empty());
    }

    #[test]
    fn phantom_when_registered_but_binary_missing() {
        let findings = classify(
            &id_set(&["plugin-a", "plugin-b"]),
            &id_set(&["plugin-a", "plugin-b"]),
            &|id| id == "plugin-b",
        );
        assert!(findings.orphans.is_empty());
        assert_eq!(findings.phantoms, vec!["plugin-b".to_string()]);
    }

    #[test]
    fn orphan_takes_precedence_and_skips_binary_probe() {
        let findings = classify(&id_set(&["plugin-orphan"]), &id_set(&[]), &|_| {
            panic!("binary probe must not run for an orphan with no registry entry")
        });
        assert_eq!(findings.orphans, vec!["plugin-orphan".to_string()]);
        assert!(findings.phantoms.is_empty());
    }

    #[test]
    fn clean_when_all_present_and_built() {
        let findings = classify(
            &id_set(&["plugin-a", "plugin-b"]),
            &id_set(&["plugin-a", "plugin-b"]),
            &|_| false,
        );
        assert!(findings.orphans.is_empty() && findings.phantoms.is_empty());
    }

    #[test]
    fn orphans_emit_safe_prune_fix() {
        let findings = Findings {
            orphans: vec!["plugin-x".into()],
            phantoms: Vec::new(),
        };
        let report = report_from_findings(&findings);
        assert_eq!(report.issues.len(), 1);
        assert!(matches!(
            report.fixes.as_slice(),
            [FixAction::PruneOrphanFingerprints { ids }] if ids == &vec!["plugin-x".to_string()]
        ));
        assert!(
            report
                .fixes
                .iter()
                .all(|fix| fix.applicability() == FixApplicability::SafeAutomatic),
            "prune fix must be safe-automatic"
        );
    }

    #[test]
    fn phantoms_emit_advice_and_never_a_fix() {
        let findings = Findings {
            orphans: Vec::new(),
            phantoms: vec!["plugin-y".into()],
        };
        let report = report_from_findings(&findings);
        assert_eq!(report.issues.len(), 1);
        assert!(
            report.fixes.is_empty(),
            "a phantom must not produce any fix action (no rebuild)"
        );
        assert!(
            !report.advice.is_empty(),
            "a phantom must advise rebuilding via the dev/recompile path"
        );
    }

    #[test]
    fn clean_findings_yield_ok_report() {
        let report = report_from_findings(&Findings {
            orphans: Vec::new(),
            phantoms: Vec::new(),
        });
        assert!(report.issues.is_empty());
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn file_state_detects_absent_valid_and_corrupt() {
        let tmp = tempfile::TempDir::new().unwrap();

        let missing = tmp.path().join("dev/build-fingerprints.json");
        assert!(matches!(file_state(&missing), FileState::Absent));

        let valid = tmp.path().join("valid.json");
        std::fs::write(&valid, r#"{"fingerprints":{"plugin-a":"hash"}}"#).unwrap();
        assert!(matches!(file_state(&valid), FileState::Valid(map) if map.len() == 1));

        let empty_object = tmp.path().join("empty.json");
        std::fs::write(&empty_object, "{}").unwrap();
        assert!(
            matches!(file_state(&empty_object), FileState::Valid(map) if map.is_empty()),
            "an empty object is valid-but-empty, not corrupt"
        );

        let corrupt = tmp.path().join("corrupt.json");
        std::fs::write(&corrupt, "{ not valid json").unwrap();
        assert!(matches!(file_state(&corrupt), FileState::Corrupt(_)));
    }

    #[test]
    fn corrupt_report_is_error_with_advice_and_no_fix() {
        let report = corrupt_report(Path::new("/cfg/dev/build-fingerprints.json"), "boom");
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Error);
        assert!(report.fixes.is_empty());
        assert!(!report.advice.is_empty());
    }
}
