use std::path::{Path, PathBuf};

use crate::config::ServerConfig;

pub(super) const RULE_NAME: &str = "qol PointZ";
const RULE_DESCRIPTION: &str = "Lets phones find and control this PC through qol PointZ";
const OWNERSHIP_FILE: &str = "firewall-rule.json";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum AllowPlan {
    Add,
    Repair { recorded: String },
    Refuse,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RemovePlan {
    Nothing,
    Forget,
    Delete { recorded: String },
}

pub(super) fn allow_plan(recorded: Option<&str>, existing: Option<&str>) -> AllowPlan {
    match (recorded, existing) {
        (_, None) => AllowPlan::Add,
        (Some(recorded), Some(existing)) if same_program(recorded, existing) => AllowPlan::Repair {
            recorded: recorded.to_string(),
        },
        _ => AllowPlan::Refuse,
    }
}

pub(super) fn remove_plan(recorded: Option<&str>, existing: Option<&str>) -> RemovePlan {
    match (recorded, existing) {
        (None, _) => RemovePlan::Nothing,
        (Some(_), None) => RemovePlan::Forget,
        (Some(recorded), Some(_)) => RemovePlan::Delete {
            recorded: recorded.to_string(),
        },
    }
}

pub(super) fn same_program(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

fn quotable(program: &str) -> bool {
    !program.contains('"') && !program.ends_with('\\')
}

pub(super) fn ports() -> String {
    format!(
        "{},{}",
        ServerConfig::DISCOVERY_PORT,
        ServerConfig::COMMAND_PORT
    )
}

fn settings(program: &str, profiles: &str) -> String {
    format!(
        r#"dir=in action=allow protocol=UDP localport={} profile={profiles} program="{program}""#,
        ports()
    )
}

pub(super) fn add_arguments(program: &str, profiles: &str) -> String {
    format!(
        r#"advfirewall firewall add rule name="{RULE_NAME}" {} enable=yes description="{RULE_DESCRIPTION}""#,
        settings(program, profiles)
    )
}

pub(super) fn repair_arguments(recorded: &str, program: &str, profiles: &str) -> String {
    format!(
        r#"advfirewall firewall set rule name="{RULE_NAME}" program="{recorded}" new enable=yes {}"#,
        settings(program, profiles)
    )
}

pub(super) fn delete_arguments(recorded: &str) -> String {
    format!(r#"advfirewall firewall delete rule name="{RULE_NAME}" program="{recorded}""#)
}

pub(super) struct Ownership {
    path: PathBuf,
}

impl Ownership {
    pub(super) fn in_data_dir(data_dir: &Path) -> Self {
        Self::at(
            data_dir
                .join("plugins")
                .join(env!("QOL_PLUGIN_ID"))
                .join(OWNERSHIP_FILE),
        )
    }

    pub(super) fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn program(&self) -> Result<Option<String>, String> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("could not read {}: {error}", self.path.display())),
        };
        let record: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            format!("{} is not a firewall record: {error}", self.path.display())
        })?;
        match (record["rule"].as_str(), record["program"].as_str()) {
            (Some(RULE_NAME), Some(program)) if quotable(program) => Ok(Some(program.to_string())),
            _ => Err(format!("{} is not a firewall record", self.path.display())),
        }
    }

    pub(super) fn record(&self, program: &str) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        let record = serde_json::json!({ "rule": RULE_NAME, "program": program });
        std::fs::write(&self.path, record.to_string())
            .map_err(|error| format!("could not write {}: {error}", self.path.display()))
    }

    pub(super) fn forget(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("could not remove {}: {error}", self.path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = r"C:\Users\Ada\AppData\Local\qol\qol-pointz.exe";
    const MOVED: &str = r"D:\Portable\qol-pointz.exe";

    #[test]
    fn allow_adds_a_missing_rule_and_repairs_only_the_rule_pointz_recorded() {
        let shouted = PROGRAM.to_uppercase();
        let cases = [
            (None, None, AllowPlan::Add),
            (Some(PROGRAM), None, AllowPlan::Add),
            (
                Some(PROGRAM),
                Some(PROGRAM),
                AllowPlan::Repair {
                    recorded: PROGRAM.into(),
                },
            ),
            (
                Some(PROGRAM),
                Some(shouted.as_str()),
                AllowPlan::Repair {
                    recorded: PROGRAM.into(),
                },
            ),
            (None, Some(PROGRAM), AllowPlan::Refuse),
            (None, Some(""), AllowPlan::Refuse),
            (Some(PROGRAM), Some(MOVED), AllowPlan::Refuse),
            (Some(PROGRAM), Some(""), AllowPlan::Refuse),
        ];
        for (recorded, existing, expected) in cases {
            assert_eq!(
                allow_plan(recorded, existing),
                expected,
                "{recorded:?} {existing:?}"
            );
        }
    }

    #[test]
    fn remove_deletes_only_a_recorded_rule() {
        let cases = [
            (None, None, RemovePlan::Nothing),
            (None, Some(PROGRAM), RemovePlan::Nothing),
            (Some(PROGRAM), None, RemovePlan::Forget),
            (
                Some(PROGRAM),
                Some(MOVED),
                RemovePlan::Delete {
                    recorded: PROGRAM.into(),
                },
            ),
            (
                Some(PROGRAM),
                Some(PROGRAM),
                RemovePlan::Delete {
                    recorded: PROGRAM.into(),
                },
            ),
        ];
        for (recorded, existing, expected) in cases {
            assert_eq!(
                remove_plan(recorded, existing),
                expected,
                "{recorded:?} {existing:?}"
            );
        }
    }

    #[test]
    fn netsh_scopes_every_rule_change_to_the_pointz_program() {
        let add = add_arguments(PROGRAM, "private");
        assert!(add.starts_with(r#"advfirewall firewall add rule name="qol PointZ" dir=in"#));
        assert!(add.contains(&format!("localport={}", ports())));
        assert!(add.contains("protocol=UDP"));
        assert!(add.contains(&format!(r#"program="{PROGRAM}""#)));
        let repair = repair_arguments(PROGRAM, MOVED, "private");
        assert!(repair.starts_with(&format!(
            r#"advfirewall firewall set rule name="qol PointZ" program="{PROGRAM}" new enable=yes"#
        )));
        assert!(repair.ends_with(&format!(r#"program="{MOVED}""#)));
        assert!(!repair.contains("description="));
        assert_eq!(
            delete_arguments(PROGRAM),
            format!(r#"advfirewall firewall delete rule name="qol PointZ" program="{PROGRAM}""#)
        );
    }

    #[test]
    fn the_ownership_record_round_trips_and_forgets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ownership = Ownership::in_data_dir(dir.path());
        assert_eq!(ownership.program(), Ok(None));
        ownership.record(PROGRAM).unwrap();
        assert_eq!(ownership.program(), Ok(Some(PROGRAM.to_string())));
        ownership.forget().unwrap();
        assert_eq!(ownership.program(), Ok(None));
        assert_eq!(ownership.forget(), Ok(()));
    }

    #[test]
    fn a_record_for_another_rule_or_an_unquotable_program_is_not_ownership() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(OWNERSHIP_FILE);
        let cases = [
            serde_json::json!({ "rule": "Other", "program": PROGRAM }).to_string(),
            serde_json::json!({ "rule": RULE_NAME }).to_string(),
            serde_json::json!({ "rule": RULE_NAME, "program": r#"C:\x.exe" name="Other"# })
                .to_string(),
            serde_json::json!({ "rule": RULE_NAME, "program": r"C:\qol\" }).to_string(),
            "not json".to_string(),
        ];
        for content in cases {
            std::fs::write(&path, &content).unwrap();
            assert!(Ownership::at(path.clone()).program().is_err(), "{content}");
        }
    }
}
