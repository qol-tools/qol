use crate::policy::{
    new_session_id, read_journal, restore_journal, write_journal_durable, JournalPayload,
    JournalState, PolicyError, PolicyJournal, ResidencyOwnerId, RestoreOutcome,
    JOURNAL_SCHEMA_VERSION,
};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

mod platform;

pub const UDEV_UACCESS_POLICY_ID: &str = "udev-i2c-uaccess";
pub const RULE_FILE_NAME: &str = "90-qol-i2c-uaccess.rules";
pub const RULE_CONTENT: &str = "# qol host fixes: grant uaccess to i2c display adapters (reversible)\nSUBSYSTEM==\"i2c-dev\", ATTRS{class}==\"0x030000\", TAG+=\"uaccess\"\n";

pub fn rule_path() -> PathBuf {
    platform::rules_dir().join(RULE_FILE_NAME)
}

pub fn grant(owner: &ResidencyOwnerId) -> Result<()> {
    if let Some(journal) = read_journal(UDEV_UACCESS_POLICY_ID)? {
        if journal.state == JournalState::Active {
            return Err(PolicyError::Busy {
                policy: UDEV_UACCESS_POLICY_ID.to_string(),
                detail: "the uaccess grant is already active; revoke it first".to_string(),
            }
            .into());
        }
    }
    write_journal_durable(&active_journal(owner)?)
        .with_context(|| format!("failed to record the `{UDEV_UACCESS_POLICY_ID}` grant"))?;
    platform::grant(&rule_path(), RULE_CONTENT)
        .with_context(|| format!("failed to apply the `{UDEV_UACCESS_POLICY_ID}` grant"))
}

pub fn revoke(owner: &ResidencyOwnerId) -> Result<RestoreOutcome> {
    let Some(journal) = read_journal(UDEV_UACCESS_POLICY_ID)? else {
        return Ok(RestoreOutcome::NothingToRestore);
    };
    if !journal.owners.iter().any(|granted| granted == owner) {
        return Err(PolicyError::Busy {
            policy: UDEV_UACCESS_POLICY_ID.to_string(),
            detail: "the caller is not an owner of the active grant".to_string(),
        }
        .into());
    }
    restore_journal(UDEV_UACCESS_POLICY_ID)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UdevUaccessPayload {
    pub rule_path: PathBuf,
    pub rule_sha256: String,
    pub rule_content: String,
}

impl JournalPayload for UdevUaccessPayload {
    fn policy_id(&self) -> &'static str {
        UDEV_UACCESS_POLICY_ID
    }

    fn has_staged_path(&self) -> bool {
        false
    }

    fn has_staged_identity(&self) -> bool {
        false
    }

    fn has_active_fingerprint(&self) -> bool {
        true
    }

    fn rendered_hash(&self) -> Result<String> {
        Ok(self.rule_sha256.clone())
    }

    fn validate_payload(&self, policy: &str) -> Result<()> {
        validate_payload(policy, self)
    }

    fn recorded_mutations(&self) -> usize {
        1
    }

    fn restore(&self, _policy: &str) -> Result<()> {
        platform::restore_rule(&self.rule_path, &self.rule_content)
            .with_context(|| format!("failed to restore the `{UDEV_UACCESS_POLICY_ID}` grant"))
    }
}

pub(crate) fn validate_payload(policy: &str, payload: &UdevUaccessPayload) -> Result<()> {
    if policy != UDEV_UACCESS_POLICY_ID {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: "the payload is not a udev-uaccess payload".to_string(),
        }
        .into());
    }
    if payload.rule_path != rule_path() {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: "the rule path must be the canonical qol rule path".to_string(),
        }
        .into());
    }
    if payload.rule_content.is_empty() || payload.rule_content.len() > 1024 {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: "the rule content must be non-empty and bounded".to_string(),
        }
        .into());
    }
    if payload.rule_sha256 != sha256_hex(&payload.rule_content) {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: "the rule checksum does not match the recorded rule content".to_string(),
        }
        .into());
    }
    Ok(())
}

fn active_journal(owner: &ResidencyOwnerId) -> Result<PolicyJournal> {
    Ok(PolicyJournal {
        schema_version: JOURNAL_SCHEMA_VERSION,
        policy: UDEV_UACCESS_POLICY_ID.to_string(),
        owners: vec![owner.clone()],
        state: JournalState::Active,
        created_unix_ms: now_unix_ms(),
        session_id: new_session_id()?,
        content_sha256: String::new(),
        payload: crate::policy::PolicyPayload::UdevUaccess(UdevUaccessPayload {
            rule_path: rule_path(),
            rule_sha256: sha256_hex(RULE_CONTENT),
            rule_content: RULE_CONTENT.to_string(),
        }),
        failure: None,
        journal_file_identity: None,
    })
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_unix_ms() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_millis() as u64,
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::test_support;

    fn owner() -> ResidencyOwnerId {
        ResidencyOwnerId::parse("owner-a").unwrap()
    }

    fn test_journal() -> PolicyJournal {
        PolicyJournal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            policy: UDEV_UACCESS_POLICY_ID.to_string(),
            owners: vec![owner()],
            state: JournalState::Active,
            created_unix_ms: 1,
            session_id: new_session_id().unwrap(),
            content_sha256: String::new(),
            payload: crate::policy::PolicyPayload::UdevUaccess(UdevUaccessPayload {
                rule_path: rule_path(),
                rule_sha256: sha256_hex(RULE_CONTENT),
                rule_content: RULE_CONTENT.to_string(),
            }),
            failure: None,
            journal_file_identity: None,
        }
    }

    #[cfg(target_os = "linux")]
    struct TestEnv {
        _guard: std::sync::MutexGuard<'static, ()>,
        _dir: tempfile::TempDir,
        previous_path: Option<std::ffi::OsString>,
        log_path: PathBuf,
    }

    #[cfg(target_os = "linux")]
    impl TestEnv {
        fn new() -> Self {
            let _guard = test_support::serialized();
            test_support::reset_dir();
            let dir = tempfile::tempdir().unwrap();
            let rules_dir = dir.path().join("rules.d");
            std::fs::create_dir_all(&rules_dir).unwrap();
            std::env::set_var("QOL_UDEV_RULES_DIR", &rules_dir);
            let shim_dir = dir.path().join("bin");
            std::fs::create_dir_all(&shim_dir).unwrap();
            let log_path = dir.path().join("udevadm.log");
            std::env::set_var("QOL_UDEVADM_LOG", &log_path);
            let shim = shim_dir.join("udevadm");
            std::fs::write(
                &shim,
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$QOL_UDEVADM_LOG\"\n",
            )
            .unwrap();
            let mut permissions = std::fs::metadata(&shim).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
            std::fs::set_permissions(&shim, permissions).unwrap();
            let previous_path = std::env::var_os("PATH");
            let path = format!(
                "{}:{}",
                shim_dir.display(),
                previous_path
                    .as_deref()
                    .map(|path| path.to_string_lossy())
                    .unwrap_or_default()
            );
            std::env::set_var("PATH", path);
            Self {
                _guard,
                _dir: dir,
                previous_path,
                log_path,
            }
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for TestEnv {
        fn drop(&mut self) {
            match &self.previous_path {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
            std::env::remove_var("QOL_UDEV_RULES_DIR");
            std::env::remove_var("QOL_UDEVADM_LOG");
        }
    }

    #[test]
    fn rule_content_is_the_ddcutil_style_uaccess_rule() {
        let rule = "SUBSYSTEM==\"i2c-dev\", ATTRS{class}==\"0x030000\", TAG+=\"uaccess\"\n";
        assert!(RULE_CONTENT.contains(rule), "{RULE_CONTENT:?}");
        assert!(RULE_CONTENT.ends_with('\n'));
        assert_eq!(RULE_FILE_NAME, "90-qol-i2c-uaccess.rules");
        assert!(
            rule_path().ends_with(RULE_FILE_NAME),
            "{}",
            rule_path().display()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_writes_the_rule_reloads_triggers_and_journals() {
        let env = TestEnv::new();
        grant(&owner()).unwrap();
        assert_eq!(
            std::fs::read(rule_path()).unwrap(),
            RULE_CONTENT.as_bytes(),
            "the rule file must carry the exact rendered content"
        );
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert!(log.contains("control --reload"), "{log}");
        assert!(log.contains("trigger"), "{log}");
        assert!(
            log.find("control --reload").unwrap() < log.find("trigger").unwrap(),
            "the reload must precede the trigger"
        );
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        assert_eq!(journal.owners, vec![owner()]);
        let crate::policy::PolicyPayload::UdevUaccess(payload) = journal.payload else {
            panic!("the journal must carry a udev-uaccess payload");
        };
        assert_eq!(payload.rule_path, rule_path());
        assert_eq!(payload.rule_sha256, sha256_hex(RULE_CONTENT));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_refuses_an_already_active_grant() {
        let _env = TestEnv::new();
        grant(&owner()).unwrap();
        let error = grant(&owner()).unwrap_err();
        assert!(format!("{error:#}").contains("already active"), "{error:#}");
    }

    #[test]
    fn udev_journal_round_trips_through_the_durable_write() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let saved = test_journal();
        write_journal_durable(&saved).unwrap();
        let read = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(read.policy, saved.policy);
        assert_eq!(read.state, saved.state);
        assert_eq!(read.owners, saved.owners);
        assert_eq!(read.payload, saved.payload);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn restore_removes_the_rule_reloads_and_clears_the_journal() {
        let env = TestEnv::new();
        grant(&owner()).unwrap();
        assert!(rule_path().exists());
        let outcome = restore_journal(UDEV_UACCESS_POLICY_ID).unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        assert!(!rule_path().exists(), "the rule must be removed");
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none(),
            "the restored journal must be cleared"
        );
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert!(log.contains("control --reload"), "{log}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn restore_preserves_a_modified_rule_and_fails_closed() {
        let _env = TestEnv::new();
        grant(&owner()).unwrap();
        std::fs::write(rule_path(), b"operator rule\n").unwrap();
        let error = restore_journal(UDEV_UACCESS_POLICY_ID).unwrap_err();
        assert!(
            format!("{error:#}").contains("refusing to remove a modified rule"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(rule_path()).unwrap(),
            b"operator rule\n",
            "the operator-modified rule must be preserved"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_some(),
            "the journal must survive a refused restore"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn revoke_removes_the_rule_and_clears_the_journal() {
        let _env = TestEnv::new();
        grant(&owner()).unwrap();
        let outcome = revoke(&owner()).unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        assert!(!rule_path().exists());
        assert!(read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none());
    }

    #[test]
    fn revoke_without_a_journal_reports_nothing_to_restore() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        assert_eq!(revoke(&owner()).unwrap(), RestoreOutcome::NothingToRestore);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn revoke_refuses_a_caller_who_is_not_an_owner() {
        let _env = TestEnv::new();
        grant(&owner()).unwrap();
        let other = ResidencyOwnerId::parse("owner-b").unwrap();
        let error = revoke(&other).unwrap_err();
        assert!(format!("{error:#}").contains("not an owner"), "{error:#}");
        assert!(rule_path().exists(), "the rule must survive the refusal");
    }

    #[test]
    fn payload_validation_rejects_a_forged_rule_path() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let mut payload = test_journal().payload;
        let crate::policy::PolicyPayload::UdevUaccess(udev_payload) = &mut payload else {
            panic!("the journal must carry a udev-uaccess payload");
        };
        udev_payload.rule_path = PathBuf::from("/tmp/forged");
        let error = udev_payload
            .validate_payload(UDEV_UACCESS_POLICY_ID)
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("canonical qol rule path"),
            "{error:#}"
        );
    }

    #[test]
    fn payload_validation_rejects_a_checksum_mismatch() {
        let _guard = test_support::serialized();
        test_support::reset_dir();
        let mut payload = test_journal().payload;
        let crate::policy::PolicyPayload::UdevUaccess(udev_payload) = &mut payload else {
            panic!("the journal must carry a udev-uaccess payload");
        };
        udev_payload.rule_sha256 = "f".repeat(64);
        let error = udev_payload
            .validate_payload(UDEV_UACCESS_POLICY_ID)
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("does not match the recorded rule content"),
            "{error:#}"
        );
    }
}
