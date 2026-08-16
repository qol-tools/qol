use anyhow::{Context, Result};
use qol_host_fixes::policy::nvidia::NVIDIA_POLICY_ID;
use qol_host_fixes::policy::{
    acquire_policy_lock, restore_journal, ResidentPolicy, RestoreOutcome,
};
use qol_host_fixes::udev::UDEV_UACCESS_POLICY_ID;

pub(crate) const RESTORE_ORDER: [&str; 2] = [UDEV_UACCESS_POLICY_ID, NVIDIA_POLICY_ID];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreEntry {
    Restored {
        policy: &'static str,
        outcome: RestoreOutcome,
    },
    Failed {
        policy: &'static str,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    entries: Vec<RestoreEntry>,
}

impl RestoreReport {
    pub fn entries(&self) -> &[RestoreEntry] {
        &self.entries
    }

    pub fn succeeded(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| matches!(entry, RestoreEntry::Restored { .. }))
    }
}

pub fn restore_all() -> RestoreReport {
    restore_all_with(&RESTORE_ORDER, restore_one)
}

fn restore_one(policy: &'static str) -> Result<RestoreOutcome> {
    let resident = ResidentPolicy::from_id(policy)?;
    let _guard = acquire_policy_lock(&resident)?;
    restore_journal(policy).with_context(|| format!("residency restore failed for `{policy}`"))
}

fn restore_all_with(
    order: &[&'static str],
    mut restore: impl FnMut(&'static str) -> Result<RestoreOutcome>,
) -> RestoreReport {
    let mut entries = Vec::with_capacity(order.len());
    for &policy in order {
        let entry = match restore(policy) {
            Ok(outcome) => {
                trace_restore(policy, outcome_as_str(outcome), None);
                RestoreEntry::Restored { policy, outcome }
            }
            Err(error) => {
                let reason = qol_runtime::probe::token(&format!("{error:#}"));
                trace_restore(policy, "failed", Some(&reason));
                RestoreEntry::Failed {
                    policy,
                    error: format!("{error:#}"),
                }
            }
        };
        entries.push(entry);
    }
    RestoreReport { entries }
}

fn outcome_as_str(outcome: RestoreOutcome) -> &'static str {
    match outcome {
        RestoreOutcome::NothingToRestore => "nothing_to_restore",
        RestoreOutcome::Restored => "restored",
        RestoreOutcome::DeletedZeroMutation => "deleted_zero_mutation",
    }
}

fn trace_restore(policy: &str, outcome: &str, reason: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        match reason {
            Some(reason) => qol_runtime::probe!(
                "RESIDENT_POLICY_RESTORE",
                "policy={policy} outcome={outcome} reason={reason}"
            ),
            None => qol_runtime::probe!(
                "RESIDENT_POLICY_RESTORE",
                "policy={policy} outcome={outcome}"
            ),
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (policy, outcome, reason);
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn restore_all_runs_udev_before_nvidia_when_both_journals_are_present() {
        let mut calls = Vec::new();
        let report = restore_all_with(&RESTORE_ORDER, |policy| {
            calls.push(policy);
            Ok(RestoreOutcome::Restored)
        });
        assert_eq!(calls, RESTORE_ORDER.to_vec());
        assert!(report.succeeded());
        assert_eq!(
            report.entries(),
            &[
                RestoreEntry::Restored {
                    policy: UDEV_UACCESS_POLICY_ID,
                    outcome: RestoreOutcome::Restored,
                },
                RestoreEntry::Restored {
                    policy: NVIDIA_POLICY_ID,
                    outcome: RestoreOutcome::Restored,
                },
            ]
        );
    }

    #[test]
    fn restore_all_still_runs_nvidia_when_the_udev_restore_fails_first() {
        let mut calls = Vec::new();
        let report = restore_all_with(&RESTORE_ORDER, |policy| {
            calls.push(policy);
            if policy == UDEV_UACCESS_POLICY_ID {
                Err(anyhow!("injected udev restore failure"))
            } else {
                Ok(RestoreOutcome::Restored)
            }
        });
        assert_eq!(calls, RESTORE_ORDER.to_vec());
        assert!(!report.succeeded());
        assert_eq!(
            report.entries(),
            &[
                RestoreEntry::Failed {
                    policy: UDEV_UACCESS_POLICY_ID,
                    error: "injected udev restore failure".to_string(),
                },
                RestoreEntry::Restored {
                    policy: NVIDIA_POLICY_ID,
                    outcome: RestoreOutcome::Restored,
                },
            ]
        );
    }

    #[test]
    fn restore_all_is_a_noop_when_neither_policy_has_a_journal() {
        let mut calls = Vec::new();
        let report = restore_all_with(&RESTORE_ORDER, |policy| {
            calls.push(policy);
            Ok(RestoreOutcome::NothingToRestore)
        });
        assert_eq!(calls, RESTORE_ORDER.to_vec());
        assert!(report.succeeded());
        assert!(report.entries().iter().all(|entry| matches!(
            entry,
            RestoreEntry::Restored {
                outcome: RestoreOutcome::NothingToRestore,
                ..
            }
        )));
    }

    #[test]
    fn restore_all_reports_each_policy_failure_with_its_own_error() {
        let report = restore_all_with(&RESTORE_ORDER, |policy| {
            Err(anyhow!("injected failure for {policy}"))
        });
        assert!(!report.succeeded());
        assert_eq!(
            report.entries(),
            &[
                RestoreEntry::Failed {
                    policy: UDEV_UACCESS_POLICY_ID,
                    error: "injected failure for udev-i2c-uaccess".to_string(),
                },
                RestoreEntry::Failed {
                    policy: NVIDIA_POLICY_ID,
                    error: "injected failure for nvidia-driver-version-pin".to_string(),
                },
            ]
        );
    }

    #[cfg(all(feature = "sandbox", target_os = "linux"))]
    #[test]
    fn restore_all_restores_real_journals_udev_before_nvidia() {
        use qol_host_fixes::policy::nvidia::{
            render_fragment, sha256_hex, ActiveFileFingerprint, NvidiaPayload, PackageEntry,
        };
        use qol_host_fixes::policy::{
            new_session_id, write_journal_durable, JournalState, PolicyJournal, PolicyPayload,
            ResidencyOwnerId, JOURNAL_SCHEMA_VERSION,
        };
        use qol_host_fixes::udev::{rule_path, UdevUaccessPayload, RULE_CONTENT};
        use std::os::unix::fs::PermissionsExt;

        let _serial = serialized_sandbox_tests();
        let dir = tempfile::tempdir().unwrap();
        let journal_dir = dir.path().join("journal");
        std::fs::create_dir_all(&journal_dir).unwrap();
        std::env::set_var("QOL_POLICY_JOURNAL_DIR", &journal_dir);
        let fragment = dir.path().join("90qol-nvidia-driver.pref");
        std::env::set_var("QOL_RESIDENT_FRAGMENT_PATH", &fragment);
        let rules_dir = dir.path().join("rules.d");
        std::fs::create_dir_all(&rules_dir).unwrap();
        std::env::set_var("QOL_UDEV_RULES_DIR", &rules_dir);
        let dev_dir = dir.path().join("dev");
        std::fs::create_dir_all(&dev_dir).unwrap();
        std::env::set_var("QOL_UDEV_I2C_DEV_DIR", &dev_dir);
        std::env::set_var("QOL_UDEV_SEAT_USER", "fakeuser");

        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        for name in ["udevadm", "getfacl", "setfacl"] {
            let shim = bin_dir.join(name);
            std::fs::write(&shim, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&shim, PermissionsExt::from_mode(0o755)).unwrap();
        }
        let previous_path = std::env::var_os("PATH");
        let shimmed_path = format!(
            "{}:{}",
            bin_dir.display(),
            previous_path
                .as_deref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        std::env::set_var("PATH", &shimmed_path);
        let _restore_env = EnvRestore { previous_path };

        let owner = ResidencyOwnerId::parse("owner-a").unwrap();
        let entries = vec![PackageEntry {
            package: "nvidia-driver-560".to_string(),
            version: "560.35.03-0ubuntu1".to_string(),
        }];
        let resource_identity = format!("{NVIDIA_POLICY_ID}:{}", "a".repeat(32));
        let rendered_fragment = render_fragment(&entries, &resource_identity);
        let rendered_sha256 = sha256_hex(&rendered_fragment);
        let nvidia = PolicyJournal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            policy: NVIDIA_POLICY_ID.to_string(),
            owners: vec![owner.clone()],
            state: JournalState::Active,
            created_unix_ms: 1,
            session_id: new_session_id().unwrap(),
            content_sha256: String::new(),
            payload: PolicyPayload::Nvidia(NvidiaPayload {
                entries,
                expected_module_version: "580.159.02".to_string(),
                resource_identity,
                staged_path: None,
                staged_identity: None,
                active_fingerprint: Some(ActiveFileFingerprint {
                    dev: 1,
                    ino: 1,
                    rendered_sha256: rendered_sha256.clone(),
                    mode: 0o100644,
                    uid: unsafe { libc::geteuid() },
                    gid: unsafe { libc::getegid() },
                    ctime_sec: 1,
                    ctime_nsec: 1,
                }),
                rendered_sha256,
            }),
            failure: None,
            journal_file_identity: None,
        };
        write_journal_durable(&nvidia).unwrap();

        let rule_file = rule_path();
        std::fs::write(&rule_file, RULE_CONTENT.as_bytes()).unwrap();
        let udev = PolicyJournal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            policy: UDEV_UACCESS_POLICY_ID.to_string(),
            owners: vec![owner],
            state: JournalState::Active,
            created_unix_ms: 1,
            session_id: new_session_id().unwrap(),
            content_sha256: String::new(),
            payload: PolicyPayload::UdevUaccess(UdevUaccessPayload {
                rule_path: rule_file.clone(),
                rule_sha256: sha256_hex(RULE_CONTENT),
                rule_content: RULE_CONTENT.to_string(),
                rule_applied: true,
            }),
            failure: None,
            journal_file_identity: None,
        };
        write_journal_durable(&udev).unwrap();

        let report = restore_all();
        assert!(report.succeeded(), "{report:?}");
        assert_eq!(
            report.entries(),
            &[
                RestoreEntry::Restored {
                    policy: UDEV_UACCESS_POLICY_ID,
                    outcome: RestoreOutcome::Restored,
                },
                RestoreEntry::Restored {
                    policy: NVIDIA_POLICY_ID,
                    outcome: RestoreOutcome::Restored,
                },
            ]
        );
        assert!(
            !rule_file.exists(),
            "the owned uaccess rule must be removed by the restore"
        );
        assert_eq!(
            std::fs::read(&fragment).unwrap(),
            rendered_fragment.as_bytes(),
            "the owned nvidia pin fragment must be written back by the restore"
        );
        assert!(
            !journal_dir
                .join("qol-resident-policy-udev-i2c-uaccess.json")
                .exists(),
            "the udev journal must be removed after a successful restore"
        );
        assert!(
            !journal_dir
                .join("qol-resident-policy-nvidia-driver-version-pin.json")
                .exists(),
            "the nvidia journal must be removed after a successful restore"
        );
    }

    #[cfg(all(feature = "sandbox", target_os = "linux"))]
    #[test]
    fn restore_one_refuses_while_the_policy_lock_is_held() {
        use qol_host_fixes::policy::{acquire_policy_lock, ResidentPolicy};
        let _serial = serialized_sandbox_tests();
        std::env::set_var("QOL_POLICY_LOCK_RETRY_WINDOW_MS", "50");
        let policy = ResidentPolicy::from_id(UDEV_UACCESS_POLICY_ID).unwrap();
        let _held = acquire_policy_lock(&policy).unwrap();
        let error = restore_one(UDEV_UACCESS_POLICY_ID).unwrap_err();
        std::env::remove_var("QOL_POLICY_LOCK_RETRY_WINDOW_MS");
        assert!(
            format!("{error:#}").contains("another process holds"),
            "the restore must take the policy lock like grant/revoke: {error:#}"
        );
    }

    #[cfg(all(feature = "sandbox", target_os = "linux"))]
    fn serialized_sandbox_tests() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[cfg(all(feature = "sandbox", target_os = "linux"))]
    struct EnvRestore {
        previous_path: Option<std::ffi::OsString>,
    }

    #[cfg(all(feature = "sandbox", target_os = "linux"))]
    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for name in [
                "QOL_POLICY_JOURNAL_DIR",
                "QOL_RESIDENT_FRAGMENT_PATH",
                "QOL_UDEV_RULES_DIR",
                "QOL_UDEV_I2C_DEV_DIR",
                "QOL_UDEV_SEAT_USER",
            ] {
                std::env::remove_var(name);
            }
            match &self.previous_path {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
        }
    }
}
