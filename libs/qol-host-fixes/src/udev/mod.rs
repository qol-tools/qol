use crate::policy::{
    new_session_id, read_journal, restore_journal, write_journal_durable, JournalPayload,
    JournalState, PolicyError, PolicyJournal, ResidencyOwnerId, ResidentPolicy, RestoreOutcome,
    JOURNAL_SCHEMA_VERSION,
};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

mod platform;

pub const UDEV_UACCESS_POLICY_ID: &str = "udev-i2c-uaccess";
pub const RULE_FILE_NAME: &str = "90-qol-i2c-uaccess.rules";
pub const RULE_CONTENT: &str = "# qol host fixes: grant uaccess to i2c display adapters (reversible)\nSUBSYSTEM==\"i2c-dev\", ATTRS{class}==\"0x030000\", TAG+=\"uaccess\"\n";

pub fn rule_path() -> PathBuf {
    platform::rules_dir().join(RULE_FILE_NAME)
}

#[cfg(any(test, feature = "sandbox"))]
fn crash_point(point: &str) -> Result<()> {
    if std::env::var("QOL_RESIDENT_CRASH_POINT").as_deref() == Ok(point) {
        unsafe { libc::abort() };
    }
    Ok(())
}

#[cfg(not(any(test, feature = "sandbox")))]
fn crash_point(_point: &str) -> Result<()> {
    Ok(())
}

pub fn grant(owner: &ResidencyOwnerId) -> Result<()> {
    let policy = ResidentPolicy::from_id(UDEV_UACCESS_POLICY_ID)?;
    let _guard = crate::policy::lock::acquire(&policy)?;
    crate::policy::recover_stage::<crate::policy::PolicyPayload>(UDEV_UACCESS_POLICY_ID)?;
    match read_journal(UDEV_UACCESS_POLICY_ID)? {
        Some(journal) => match journal.state {
            JournalState::Active => {
                let rule_content = plan_content(&journal.payload)?;
                match rule_file_state(rule_content)? {
                    RuleFileState::Canonical => Err(PolicyError::Busy {
                        policy: UDEV_UACCESS_POLICY_ID.to_string(),
                        detail: "the uaccess grant is already active; revoke it first".to_string(),
                    }
                    .into()),
                    RuleFileState::Missing => {
                        platform::grant(&rule_path(), rule_content).with_context(|| {
                            format!("failed to apply the `{UDEV_UACCESS_POLICY_ID}` grant")
                        })?;
                        Ok(())
                    }
                    RuleFileState::Foreign { actual_sha256 } => {
                        Err(rule_conflict_error(rule_content, &actual_sha256).into())
                    }
                }
            }
            JournalState::Preparing => {
                let rule_content = plan_content(&journal.payload)?;
                ensure_rule_writable(rule_content)?;
                apply_and_flip(rule_content, &journal.session_id)
            }
            _ => Err(PolicyError::Busy {
                policy: UDEV_UACCESS_POLICY_ID.to_string(),
                detail: "the uaccess grant is mid-release; revoke it first".to_string(),
            }
            .into()),
        },
        None => {
            ensure_rule_writable(RULE_CONTENT)?;
            let journal = preparing_journal(owner)?;
            write_journal_durable(&journal).with_context(|| {
                format!("failed to record the `{UDEV_UACCESS_POLICY_ID}` grant")
            })?;
            apply_and_flip(RULE_CONTENT, &journal.session_id)
        }
    }
}

fn ensure_rule_writable(rule_content: &str) -> Result<()> {
    match rule_file_state(rule_content)? {
        RuleFileState::Foreign { actual_sha256 } => {
            Err(rule_conflict_error(rule_content, &actual_sha256).into())
        }
        RuleFileState::Missing | RuleFileState::Canonical => Ok(()),
    }
}

fn rule_conflict_error(expected: &str, actual_sha256: &str) -> PolicyError {
    PolicyError::RuleConflict {
        policy: UDEV_UACCESS_POLICY_ID.to_string(),
        path: rule_path().display().to_string(),
        expected_sha256: sha256_hex(expected),
        actual_sha256: actual_sha256.to_string(),
    }
}

fn apply_and_flip(rule_content: &str, session_id: &str) -> Result<()> {
    platform::grant(&rule_path(), rule_content)
        .with_context(|| format!("failed to apply the `{UDEV_UACCESS_POLICY_ID}` grant"))?;
    crash_point("udev-after-rule-apply")?;
    flip_to_active(session_id)?;
    crash_point("udev-after-flip-commit")?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RuleFileState {
    Missing,
    Canonical,
    Foreign { actual_sha256: String },
}

fn rule_file_state(expected: &str) -> Result<RuleFileState> {
    match std::fs::read(rule_path()) {
        Ok(bytes) if bytes == expected.as_bytes() => Ok(RuleFileState::Canonical),
        Ok(bytes) => Ok(RuleFileState::Foreign {
            actual_sha256: sha256_hex(&String::from_utf8_lossy(&bytes)),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RuleFileState::Missing),
        Err(error) => Err(error)
            .with_context(|| format!("failed to read the uaccess rule {}", rule_path().display())),
    }
}

pub fn revoke(owner: &ResidencyOwnerId) -> Result<RestoreOutcome> {
    let policy = ResidentPolicy::from_id(UDEV_UACCESS_POLICY_ID)?;
    let _guard = crate::policy::lock::acquire(&policy)?;
    crate::policy::recover_stage::<crate::policy::PolicyPayload>(UDEV_UACCESS_POLICY_ID)?;
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

fn plan_content(payload: &crate::policy::PolicyPayload) -> Result<&str> {
    match payload {
        crate::policy::PolicyPayload::UdevUaccess(payload) => Ok(&payload.rule_content),
        crate::policy::PolicyPayload::Nvidia(_) => {
            bail!("the `{UDEV_UACCESS_POLICY_ID}` journal does not carry a udev-uaccess payload")
        }
    }
}

fn flip_to_active(expected_session_id: &str) -> Result<()> {
    let mut journal = read_journal(UDEV_UACCESS_POLICY_ID)?.with_context(|| {
        format!("the `{UDEV_UACCESS_POLICY_ID}` journal vanished before the grant commit")
    })?;
    if journal.state != JournalState::Preparing {
        return Err(PolicyError::Busy {
            policy: UDEV_UACCESS_POLICY_ID.to_string(),
            detail: "the uaccess grant journal is not preparing; refusing the update write"
                .to_string(),
        }
        .into());
    }
    if journal.session_id != expected_session_id {
        return Err(PolicyError::Busy {
            policy: UDEV_UACCESS_POLICY_ID.to_string(),
            detail: "the uaccess grant journal session changed while the grant was in flight; refusing the update write".to_string(),
        }
        .into());
    }
    journal.state = JournalState::Active;
    match &mut journal.payload {
        crate::policy::PolicyPayload::UdevUaccess(payload) => payload.rule_applied = true,
        crate::policy::PolicyPayload::Nvidia(_) => {
            bail!("the `{UDEV_UACCESS_POLICY_ID}` journal does not carry a udev-uaccess payload")
        }
    }
    write_journal_durable(&journal)
        .with_context(|| format!("failed to commit the `{UDEV_UACCESS_POLICY_ID}` grant"))
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UdevUaccessPayload {
    pub rule_path: PathBuf,
    pub rule_sha256: String,
    pub rule_content: String,
    #[serde(default = "default_rule_applied")]
    pub rule_applied: bool,
}

fn default_rule_applied() -> bool {
    true
}

impl JournalPayload for UdevUaccessPayload {
    fn policy_id(&self) -> &'static str {
        UDEV_UACCESS_POLICY_ID
    }

    fn has_staged_path(&self) -> bool {
        !self.rule_applied
    }

    fn has_staged_identity(&self) -> bool {
        false
    }

    fn has_active_fingerprint(&self) -> bool {
        self.rule_applied
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
            .with_context(|| format!("failed to restore the `{UDEV_UACCESS_POLICY_ID}` grant"))?;
        crash_point("udev-after-restore-rule")
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

fn preparing_journal(owner: &ResidencyOwnerId) -> Result<PolicyJournal> {
    Ok(PolicyJournal {
        schema_version: JOURNAL_SCHEMA_VERSION,
        policy: UDEV_UACCESS_POLICY_ID.to_string(),
        owners: vec![owner.clone()],
        state: JournalState::Preparing,
        created_unix_ms: now_unix_ms(),
        session_id: new_session_id()?,
        content_sha256: String::new(),
        payload: crate::policy::PolicyPayload::UdevUaccess(UdevUaccessPayload {
            rule_path: rule_path(),
            rule_sha256: sha256_hex(RULE_CONTENT),
            rule_content: RULE_CONTENT.to_string(),
            rule_applied: false,
        }),
        failure: None,
        journal_file_identity: None,
    })
}

pub(crate) fn sha256_hex(content: &str) -> String {
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
                rule_applied: true,
            }),
            failure: None,
            journal_file_identity: None,
        }
    }

    #[cfg(target_os = "linux")]
    struct TestEnv {
        _guard: std::sync::MutexGuard<'static, ()>,
        dir: tempfile::TempDir,
        previous_path: Option<std::ffi::OsString>,
        log_path: PathBuf,
        acl_state_dir: PathBuf,
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
            let acl_state_dir = dir.path().join("acl-state");
            std::fs::create_dir_all(&acl_state_dir).unwrap();
            std::env::set_var("QOL_ACL_STATE_DIR", &acl_state_dir);
            let i2c_dev_dir = dir.path().join("dev");
            std::fs::create_dir_all(&i2c_dev_dir).unwrap();
            std::env::set_var("QOL_UDEV_I2C_DEV_DIR", &i2c_dev_dir);
            let getfacl = shim_dir.join("getfacl");
            std::fs::write(
                &getfacl,
                "#!/bin/sh\nfor last in \"$@\"; do :; done\nname=${last##*/}\n[ -z \"$name\" ] && exit 0\ncat \"$QOL_ACL_STATE_DIR/$name\" 2>/dev/null\nexit 0\n",
            )
            .unwrap();
            let setfacl = shim_dir.join("setfacl");
            std::fs::write(
                &setfacl,
                "#!/bin/sh\nfor last in \"$@\"; do :; done\nname=${last##*/}\n[ -z \"$name\" ] && exit 0\n[ -n \"$QOL_ACL_STICKY\" ] && exit 0\nfile=\"$QOL_ACL_STATE_DIR/$name\"\ncase \"$1\" in\n  -x)\n    user=${2#u:}\n    [ -f \"$file\" ] || exit 0\n    grep -v \"^user:$user:\" \"$file\" > \"$file.tmp\" 2>/dev/null && mv \"$file.tmp\" \"$file\" || rm -f -- \"$file\"\n    ;;\n  -m)\n    printf '%s\\n' \"$2\" >> \"$file\"\n    ;;\nesac\nexit 0\n",
            )
            .unwrap();
            for name in ["getfacl", "setfacl"] {
                let mut permissions = std::fs::metadata(shim_dir.join(name))
                    .unwrap()
                    .permissions();
                std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
                std::fs::set_permissions(shim_dir.join(name), permissions).unwrap();
            }
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
                dir,
                previous_path,
                log_path,
                acl_state_dir,
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
            std::env::remove_var("QOL_UDEV_I2C_DEV_DIR");
            std::env::remove_var("QOL_UDEV_SEAT_USER");
            std::env::remove_var("QOL_ACL_STATE_DIR");
            std::env::remove_var("QOL_ACL_STICKY");
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
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            vec!["control --reload", "trigger --subsystem-match=i2c-dev"],
            "the grant must reload and trigger the exact scoped invocation"
        );
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        assert_eq!(journal.owners, vec![owner()]);
        let crate::policy::PolicyPayload::UdevUaccess(payload) = journal.payload else {
            panic!("the journal must carry a udev-uaccess payload");
        };
        assert_eq!(payload.rule_path, rule_path());
        assert_eq!(payload.rule_sha256, sha256_hex(RULE_CONTENT));
        assert!(
            payload.rule_applied,
            "the committed grant must mark the rule applied"
        );
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
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            vec![
                "control --reload",
                "trigger --subsystem-match=i2c-dev",
                "control --reload",
            ],
            "the restore must reload without ever re-triggering the devices"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_commits_to_active_only_after_the_apply_and_resumes_a_preparing_journal() {
        let env = TestEnv::new();
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "journal-update-rename");
        let error = grant(&owner()).unwrap_err();
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        assert!(
            format!("{error:#}").contains("injected journal-update-rename failure"),
            "{error:#}"
        );
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.state,
            JournalState::Preparing,
            "the apply must not commit before the flip succeeds"
        );
        assert!(
            rule_path().exists(),
            "the rule must already be applied while the journal is preparing"
        );
        grant(&owner()).unwrap();
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert_eq!(
            log.lines().count(),
            4,
            "the resume must re-apply then flip: {log}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn restore_drops_the_seat_user_uaccess_acl_and_reports_full_restore_only_after() {
        let env = TestEnv::new();
        grant(&owner()).unwrap();
        let dev_dir = env.dir.path().join("dev");
        std::fs::create_dir_all(&dev_dir).unwrap();
        std::fs::write(dev_dir.join("i2c-0"), b"node").unwrap();
        std::fs::write(dev_dir.join("i2c-1"), b"node").unwrap();
        std::fs::write(
            env.acl_state_dir.join("i2c-0"),
            b"user:fakeuser:rw-\nuser::rw-\ngroup::r--\nother::r--\n",
        )
        .unwrap();
        std::env::set_var("QOL_UDEV_I2C_DEV_DIR", &dev_dir);
        std::env::set_var("QOL_UDEV_SEAT_USER", "fakeuser");
        let outcome = restore_journal(UDEV_UACCESS_POLICY_ID).unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        assert!(
            !rule_path().exists(),
            "the rule must be removed before the ACL drop"
        );
        assert_eq!(
            std::fs::read(env.acl_state_dir.join("i2c-0")).unwrap(),
            b"user::rw-\ngroup::r--\nother::r--\n",
            "the seat-user uaccess entry must be dropped from the live node"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none(),
            "the journal must be cleared only after the ACL is gone"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn restore_refuses_to_report_full_restore_while_the_uaccess_acl_is_live() {
        let env = TestEnv::new();
        grant(&owner()).unwrap();
        let dev_dir = env.dir.path().join("dev");
        std::fs::create_dir_all(&dev_dir).unwrap();
        std::fs::write(dev_dir.join("i2c-0"), b"node").unwrap();
        std::fs::write(env.acl_state_dir.join("i2c-0"), b"user:fakeuser:rw-\n").unwrap();
        std::env::set_var("QOL_UDEV_I2C_DEV_DIR", &dev_dir);
        std::env::set_var("QOL_UDEV_SEAT_USER", "fakeuser");
        std::env::set_var("QOL_ACL_STICKY", "1");
        let error = restore_journal(UDEV_UACCESS_POLICY_ID).unwrap_err();
        std::env::remove_var("QOL_ACL_STICKY");
        assert!(format!("{error:#}").contains("still live"), "{error:#}");
        assert_eq!(
            std::fs::read(env.acl_state_dir.join("i2c-0")).unwrap(),
            b"user:fakeuser:rw-\n",
            "the live ACL must survive the refused restore"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_some(),
            "the journal must survive while the ACL is live"
        );
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

    #[cfg(target_os = "linux")]
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

    #[cfg(target_os = "linux")]
    fn spawn_udev_probe(envs: &[(&str, &str)]) -> std::process::Output {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg("udev::tests::udev_subprocess_probe")
            .arg("--nocapture");
        command.env("QOL_UDEV_SUBPROCESS", "1");
        for (key, value) in envs {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    #[cfg(target_os = "linux")]
    fn aborted_by_sigabrt(status: std::process::ExitStatus) -> bool {
        use std::os::unix::process::ExitStatusExt;
        status.signal() == Some(libc::SIGABRT)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn udev_subprocess_probe() {
        if std::env::var_os("QOL_UDEV_SUBPROCESS").is_none() {
            return;
        }
        let owner = ResidencyOwnerId::parse("owner-a").unwrap();
        if let Some(ready) = std::env::var_os("QOL_UDEV_PROBE_READY_FILE") {
            std::fs::write(ready, b"ready").unwrap();
        }
        let outcome: Result<(), anyhow::Error> = match std::env::var("QOL_POLICY_PROBE").as_deref()
        {
            Ok("grant") => grant(&owner),
            Ok("revoke") => revoke(&owner).map(|_| ()),
            Ok("grant-then-revoke") => match grant(&owner) {
                Ok(()) => revoke(&owner).map(|_| ()),
                Err(error) => Err(error),
            },
            _ => Err(anyhow::anyhow!("unknown udev probe")),
        };
        if let Err(error) = outcome {
            eprintln!("PROBE_ERROR: {error:#}");
            std::process::exit(7);
        }
        std::process::exit(0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_resumes_after_a_crash_during_the_preparing_journal_write() {
        let env = TestEnv::new();
        let output = spawn_udev_probe(&[
            ("QOL_POLICY_PROBE", "grant"),
            ("QOL_RESIDENT_CRASH_POINT", "after-journal-stage-link"),
        ]);
        assert!(aborted_by_sigabrt(output.status), "{:?}", output.status);
        assert!(
            crate::policy::journal_stage_path(UDEV_UACCESS_POLICY_ID)
                .unwrap()
                .exists(),
            "the crash must leave the linked journal stage behind"
        );
        assert!(
            !rule_path().exists(),
            "the grant must never reach the rule apply before the crash"
        );
        grant(&owner()).unwrap();
        assert_eq!(
            std::fs::read(rule_path()).unwrap(),
            RULE_CONTENT.as_bytes(),
            "the resumed grant must apply the exact rule"
        );
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            vec!["control --reload", "trigger --subsystem-match=i2c-dev"],
            "the resumed grant must reload and trigger exactly once"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_resumes_after_a_crash_between_apply_and_flip() {
        let env = TestEnv::new();
        let output = spawn_udev_probe(&[
            ("QOL_POLICY_PROBE", "grant"),
            ("QOL_RESIDENT_CRASH_POINT", "udev-after-rule-apply"),
        ]);
        assert!(aborted_by_sigabrt(output.status), "{:?}", output.status);
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(
            journal.state,
            JournalState::Preparing,
            "the apply must not commit before the flip"
        );
        assert!(rule_path().exists());
        grant(&owner()).unwrap();
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert_eq!(
            log.lines().count(),
            4,
            "the resume must re-apply then flip: {log}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_resumes_after_a_crash_during_the_flip_write() {
        let env = TestEnv::new();
        let output = spawn_udev_probe(&[
            ("QOL_POLICY_PROBE", "grant"),
            ("QOL_RESIDENT_CRASH_POINT", "after-journal-stage-link:2"),
        ]);
        assert!(aborted_by_sigabrt(output.status), "{:?}", output.status);
        let stage = crate::policy::journal_stage_path(UDEV_UACCESS_POLICY_ID).unwrap();
        assert!(stage.exists(), "the flip crash must leave the stage behind");
        assert!(rule_path().exists(), "the rule must already be applied");
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap_err();
        assert!(
            format!("{journal:#}").contains("recovery stage"),
            "the un-recovered stage must be visible to read-only status"
        );
        grant(&owner()).unwrap();
        assert!(!stage.exists(), "the resume must recover the exact stage");
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert_eq!(
            log.lines().count(),
            4,
            "the resume must re-apply after the interrupted flip: {log}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn revoke_cleans_up_after_a_crash_post_flip_commit() {
        let _env = TestEnv::new();
        let output = spawn_udev_probe(&[
            ("QOL_POLICY_PROBE", "grant"),
            ("QOL_RESIDENT_CRASH_POINT", "udev-after-flip-commit"),
        ]);
        assert!(aborted_by_sigabrt(output.status), "{:?}", output.status);
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        assert!(rule_path().exists());
        let outcome = revoke(&owner()).unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        assert!(!rule_path().exists());
        assert!(read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn revoke_completes_after_a_crash_between_rule_removal_and_journal_removal() {
        let env = TestEnv::new();
        let output = spawn_udev_probe(&[
            ("QOL_POLICY_PROBE", "grant-then-revoke"),
            ("QOL_RESIDENT_CRASH_POINT", "udev-after-restore-rule"),
        ]);
        assert!(aborted_by_sigabrt(output.status), "{:?}", output.status);
        assert!(
            !rule_path().exists(),
            "the rule must already be removed before the crash"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_some(),
            "the journal must survive while the restore is incomplete"
        );
        let outcome = revoke(&owner()).unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        assert!(read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none());
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            vec![
                "control --reload",
                "trigger --subsystem-match=i2c-dev",
                "control --reload",
                "control --reload",
            ],
            "the resumed revoke must reload without ever re-triggering"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn revoke_aborts_a_half_applied_grant_and_leaves_no_state() {
        let env = TestEnv::new();
        std::env::set_var("QOL_RESIDENT_FAIL_NEXT", "journal-update-rename");
        let error = grant(&owner()).unwrap_err();
        std::env::remove_var("QOL_RESIDENT_FAIL_NEXT");
        assert!(
            format!("{error:#}").contains("injected journal-update-rename failure"),
            "{error:#}"
        );
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Preparing);
        let outcome = revoke(&owner()).unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        assert!(
            !rule_path().exists(),
            "the abort must remove the applied rule"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none(),
            "the abort must clear the preparing journal"
        );
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            vec![
                "control --reload",
                "trigger --subsystem-match=i2c-dev",
                "control --reload"
            ],
            "the abort must remove the rule and reload without re-triggering"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_fails_closed_and_preserves_a_corrupt_canonical_journal() {
        let env = TestEnv::new();
        let canonical = crate::policy::journal_path(UDEV_UACCESS_POLICY_ID).unwrap();
        std::fs::write(&canonical, b"this is not json").unwrap();
        let error = grant(&owner()).unwrap_err();
        assert!(
            format!("{error:#}").contains("failed to parse journal entry"),
            "{error:#}"
        );
        assert!(
            !rule_path().exists(),
            "the grant must not apply any rule against an invalid journal"
        );
        assert_eq!(
            std::fs::read(&canonical).unwrap(),
            b"this is not json",
            "the operator bytes must be preserved"
        );
        assert!(
            !env.log_path.exists(),
            "no udevadm invocation may happen against an invalid journal"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_fails_closed_and_preserves_a_truncated_journal() {
        let _env = TestEnv::new();
        let canonical = crate::policy::journal_path(UDEV_UACCESS_POLICY_ID).unwrap();
        std::fs::write(&canonical, b"{\"schema_version\":10,\"policy\":\"udev").unwrap();
        let error = grant(&owner()).unwrap_err();
        assert!(
            format!("{error:#}").contains("failed to parse journal entry"),
            "{error:#}"
        );
        assert!(!rule_path().exists());
        assert_eq!(
            std::fs::read(&canonical).unwrap(),
            b"{\"schema_version\":10,\"policy\":\"udev"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_tampered_checksum_blocks_grant_and_revoke_until_repaired() {
        let _env = TestEnv::new();
        grant(&owner()).unwrap();
        let canonical = crate::policy::journal_path(UDEV_UACCESS_POLICY_ID).unwrap();
        let mut journal: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&canonical).unwrap()).unwrap();
        journal["content_sha256"] = serde_json::Value::String("f".repeat(64));
        std::fs::write(&canonical, serde_json::to_vec(&journal).unwrap()).unwrap();
        let error = grant(&owner()).unwrap_err();
        assert!(
            format!("{error:#}").contains("content checksum mismatch"),
            "{error:#}"
        );
        let error = revoke(&owner()).unwrap_err();
        assert!(
            format!("{error:#}").contains("content checksum mismatch"),
            "{error:#}"
        );
        assert!(
            rule_path().exists(),
            "the host grant must survive the refused operations"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).is_err(),
            "the tampered journal stays visible as invalid"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_operator_stage_fragment_fails_closed_and_is_preserved() {
        let env = TestEnv::new();
        let stage = crate::policy::journal_stage_path(UDEV_UACCESS_POLICY_ID).unwrap();
        std::fs::write(&stage, b"interrupted write").unwrap();
        let error = grant(&owner()).unwrap_err();
        assert!(
            format!("{error:#}").contains("failed to parse the journal recovery stage"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&stage).unwrap(),
            b"interrupted write",
            "the operator stage fragment must be preserved"
        );
        assert!(!rule_path().exists());
        assert!(!env.log_path.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_and_revoke_serialize_across_processes_through_the_policy_lock() {
        let env = TestEnv::new();
        let policy = ResidentPolicy::from_id(UDEV_UACCESS_POLICY_ID).unwrap();
        let ready = env.dir.path().join("probe-ready");
        let _ = std::fs::remove_file(&ready);

        let lock = crate::policy::lock::try_acquire(&policy).unwrap();
        let mut grant_probe = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("udev::tests::udev_subprocess_probe")
            .arg("--nocapture")
            .env("QOL_UDEV_SUBPROCESS", "1")
            .env("QOL_POLICY_PROBE", "grant")
            .env("QOL_UDEV_PROBE_READY_FILE", &ready)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(ready.exists(), "the grant probe must signal readiness");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            grant_probe.try_wait().unwrap().is_none(),
            "the grant must block while another process holds the policy lock"
        );
        drop(lock);
        let status = grant_probe.wait().unwrap();
        assert!(status.success(), "{status:?}");
        assert!(
            rule_path().exists(),
            "the blocked grant must complete after the lock frees"
        );
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);

        let _ = std::fs::remove_file(&ready);
        let lock = crate::policy::lock::try_acquire(&policy).unwrap();
        let mut revoke_probe = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("udev::tests::udev_subprocess_probe")
            .arg("--nocapture")
            .env("QOL_UDEV_SUBPROCESS", "1")
            .env("QOL_POLICY_PROBE", "revoke")
            .env("QOL_UDEV_PROBE_READY_FILE", &ready)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(ready.exists(), "the revoke probe must signal readiness");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            revoke_probe.try_wait().unwrap().is_none(),
            "the revoke must block while another process holds the policy lock"
        );
        drop(lock);
        let status = revoke_probe.wait().unwrap();
        assert!(status.success(), "{status:?}");
        assert!(
            !rule_path().exists(),
            "the blocked revoke must complete after the lock frees"
        );
        assert!(read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn revoke_fails_closed_while_the_uaccess_acl_is_live_and_retries_cleanly() {
        let env = TestEnv::new();
        grant(&owner()).unwrap();
        let dev_dir = env.dir.path().join("dev");
        std::fs::create_dir_all(&dev_dir).unwrap();
        std::fs::write(dev_dir.join("i2c-0"), b"node").unwrap();
        std::fs::write(
            env.acl_state_dir.join("i2c-0"),
            b"user:fakeuser:rw-\nuser::rw-\ngroup::r--\nother::r--\n",
        )
        .unwrap();
        std::env::set_var("QOL_UDEV_I2C_DEV_DIR", &dev_dir);
        std::env::set_var("QOL_UDEV_SEAT_USER", "fakeuser");
        std::env::set_var("QOL_ACL_STICKY", "1");
        let error = revoke(&owner()).unwrap_err();
        std::env::remove_var("QOL_ACL_STICKY");
        assert!(format!("{error:#}").contains("still live"), "{error:#}");
        assert_eq!(
            std::fs::read(env.acl_state_dir.join("i2c-0")).unwrap(),
            b"user:fakeuser:rw-\nuser::rw-\ngroup::r--\nother::r--\n",
            "the live ACL must survive the refused revoke"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_some(),
            "the journal must survive while the ACL is live"
        );
        assert!(
            !rule_path().exists(),
            "the rule is removed before the ACL sweep, so the retry is idempotent"
        );
        let outcome = revoke(&owner()).unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        assert_eq!(
            std::fs::read(env.acl_state_dir.join("i2c-0")).unwrap(),
            b"user::rw-\ngroup::r--\nother::r--\n",
            "the retried revoke must drop the seat-user entry"
        );
        assert!(read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_stale_grant_temp_left_by_a_killed_script_is_inert_and_preserved() {
        let _env = TestEnv::new();
        let stale = rule_path().with_file_name(format!("{RULE_FILE_NAME}.qol-99999"));
        std::fs::write(&stale, b"half-written rule").unwrap();
        grant(&owner()).unwrap();
        assert_eq!(
            std::fs::read(&stale).unwrap(),
            b"half-written rule",
            "a stale temp from a killed script is indistinguishable from an operator file and must be preserved"
        );
        assert!(
            !stale
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".rules"),
            "the stale temp name must not match udev's .rules extension"
        );
        assert_eq!(
            std::fs::read(rule_path()).unwrap(),
            RULE_CONTENT.as_bytes(),
            "the fresh grant must still apply the exact rule"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_resumes_an_active_journal_whose_rule_vanished() {
        let env = TestEnv::new();
        grant(&owner()).unwrap();
        std::fs::remove_file(rule_path()).unwrap();
        grant(&owner()).unwrap();
        assert_eq!(
            std::fs::read(rule_path()).unwrap(),
            RULE_CONTENT.as_bytes(),
            "the resumed grant must re-apply the exact rule"
        );
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            vec![
                "control --reload",
                "trigger --subsystem-match=i2c-dev",
                "control --reload",
                "trigger --subsystem-match=i2c-dev",
            ],
            "the resume must reload and trigger exactly once more"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_refuses_an_active_journal_whose_rule_was_modified_and_preserves_it() {
        let env = TestEnv::new();
        grant(&owner()).unwrap();
        std::fs::write(rule_path(), b"operator rule\n").unwrap();
        let error = grant(&owner()).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("expected sha256"), "{message}");
        assert!(message.contains("actual sha256"), "{message}");
        assert!(message.contains("remove or restore"), "{message}");
        assert_eq!(
            std::fs::read(rule_path()).unwrap(),
            b"operator rule\n",
            "the modified rule must survive the refused resume"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_some(),
            "the active journal must survive the refused resume"
        );
        assert_eq!(
            std::fs::read_to_string(&env.log_path)
                .unwrap()
                .lines()
                .count(),
            2,
            "the refused resume must not re-run udevadm"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_refuses_to_overwrite_a_modified_rule_and_preserves_it() {
        let env = TestEnv::new();
        std::fs::write(rule_path(), b"operator rule\n").unwrap();
        let error = grant(&owner()).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("expected sha256"), "{message}");
        assert!(message.contains("actual sha256"), "{message}");
        assert!(message.contains("remove or restore"), "{message}");
        assert!(
            message.contains(rule_path().display().to_string().as_str()),
            "{message}"
        );
        assert_eq!(
            std::fs::read(rule_path()).unwrap(),
            b"operator rule\n",
            "the operator-modified rule must be preserved byte-for-byte"
        );
        assert!(
            !env.log_path.exists(),
            "no udevadm invocation may happen against a foreign rule"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_none(),
            "a refused fresh grant must not leave a journal behind"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_over_an_identical_existing_rule_is_idempotent() {
        let env = TestEnv::new();
        std::fs::write(rule_path(), RULE_CONTENT.as_bytes()).unwrap();
        grant(&owner()).unwrap();
        assert_eq!(
            std::fs::read(rule_path()).unwrap(),
            RULE_CONTENT.as_bytes(),
            "the idempotent grant must leave the exact rule in place"
        );
        let journal = read_journal(UDEV_UACCESS_POLICY_ID).unwrap().unwrap();
        assert_eq!(journal.state, JournalState::Active);
        let log = std::fs::read_to_string(&env.log_path).unwrap();
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            vec!["control --reload", "trigger --subsystem-match=i2c-dev"],
            "the idempotent grant must still reload and trigger once"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn restore_names_the_acl_package_when_getfacl_and_setfacl_are_missing() {
        let env = TestEnv::new();
        grant(&owner()).unwrap();
        std::env::set_var("QOL_UDEV_SEAT_USER", "fakeuser");
        let bin_dir = env.dir.path().join("bare-bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        for tool in ["sh", "rm", "cmp"] {
            let resolved = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("command -v {tool}"))
                .output()
                .unwrap();
            let target = String::from_utf8(resolved.stdout)
                .unwrap()
                .trim()
                .to_string();
            assert!(!target.is_empty(), "{tool} must exist on the host");
            std::os::unix::fs::symlink(&target, bin_dir.join(tool)).unwrap();
        }
        let udevadm = bin_dir.join("udevadm");
        std::fs::write(
            &udevadm,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$QOL_UDEVADM_LOG\"\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&udevadm).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        std::fs::set_permissions(&udevadm, permissions).unwrap();
        let saved_path = std::env::var_os("PATH");
        std::env::set_var("PATH", &bin_dir);
        let error = restore_journal(UDEV_UACCESS_POLICY_ID).unwrap_err();
        std::env::set_var("PATH", saved_path.unwrap_or_default());
        let message = format!("{error:#}");
        assert!(message.contains("getfacl"), "{message}");
        assert!(message.contains("setfacl"), "{message}");
        assert!(message.contains("acl package"), "{message}");
        assert!(
            !rule_path().exists(),
            "the rule is removed before the ACL sweep, so the retry is idempotent"
        );
        assert!(
            read_journal(UDEV_UACCESS_POLICY_ID).unwrap().is_some(),
            "the journal must survive the refused restore"
        );
    }
}
