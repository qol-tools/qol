use qol_host_fixes::policy::nvidia::PolicyStatusView;
use qol_host_fixes::policy::{PolicyState, ResidentPolicy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PolicyIntent {
    None,
    Preparing,
    Active { expected_module_version: String },
    Drifted { expected_module_version: String },
    MissingFragment,
    Releasing,
    ReleaseFailed,
    Unavailable,
}

impl PolicyIntent {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Preparing => "preparing",
            Self::Active { .. } => "active",
            Self::Drifted { .. } => "drifted",
            Self::MissingFragment => "missing-fragment",
            Self::Releasing => "releasing",
            Self::ReleaseFailed => "release-failed",
            Self::Unavailable => "unavailable",
        }
    }

    pub(crate) fn expected_module_version(&self) -> Option<&str> {
        match self {
            Self::Active {
                expected_module_version,
            }
            | Self::Drifted {
                expected_module_version,
            } => Some(expected_module_version.as_str()),
            _ => None,
        }
    }
}

pub(crate) fn query() -> PolicyIntent {
    from_query(ResidentPolicy::nvidia().status())
}

fn from_query(status: anyhow::Result<PolicyStatusView>) -> PolicyIntent {
    match status {
        Ok(view) => from_status(&view),
        Err(_) => PolicyIntent::Unavailable,
    }
}

fn from_status(view: &PolicyStatusView) -> PolicyIntent {
    let expected_module_version = view.expected_module_version.clone();
    match view.state {
        PolicyState::Absent | PolicyState::Unjournaled => PolicyIntent::None,
        PolicyState::Preparing => PolicyIntent::Preparing,
        PolicyState::Active => match expected_module_version {
            Some(expected_module_version) => PolicyIntent::Active {
                expected_module_version,
            },
            None => PolicyIntent::Unavailable,
        },
        PolicyState::Drifted => match expected_module_version {
            Some(expected_module_version) => PolicyIntent::Drifted {
                expected_module_version,
            },
            None => PolicyIntent::Unavailable,
        },
        PolicyState::MissingFragment => PolicyIntent::MissingFragment,
        PolicyState::Releasing => PolicyIntent::Releasing,
        PolicyState::ReleaseFailed => PolicyIntent::ReleaseFailed,
    }
}

pub(crate) fn notification_text(loaded: &str, on_disk: &str, intent: &PolicyIntent) -> String {
    match intent {
        PolicyIntent::Active {
            expected_module_version,
        } if expected_module_version.as_str() == on_disk => {
            format!(
            "A resident policy pins NVIDIA {on_disk} on this host. The kernel still runs {loaded}; \
             reboot to load the pinned module."
        )
        }
        PolicyIntent::Active {
            expected_module_version,
        } => format!(
            "The resident policy pins NVIDIA {expected_module_version} but the on-disk module is \
             {on_disk} while the kernel runs {loaded}; live state no longer matches the policy."
        ),
        PolicyIntent::Drifted { .. } => format!(
            "The resident NVIDIA policy has drifted (see resident-policy status); the kernel runs \
             {loaded} but the on-disk module is {on_disk}."
        ),
        _ => format!(
            "NVIDIA driver updated on disk ({on_disk}) while the kernel still runs {loaded}. \
             New OpenGL apps will fail to start until a reboot loads the matching module."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn view(state: PolicyState, module: Option<&str>) -> PolicyStatusView {
        PolicyStatusView {
            policy: qol_host_fixes::policy::nvidia::NVIDIA_POLICY_ID.to_string(),
            state,
            owners: Vec::new(),
            expected_module_version: module.map(str::to_string),
            detail: String::new(),
        }
    }

    #[cfg(target_os = "linux")]
    fn serialized_env_tests() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn query_emits_only_none_when_no_policy_is_readable() {
        let _serial = serialized_env_tests();
        assert_eq!(query(), PolicyIntent::None);
    }

    #[test]
    fn intent_maps_only_journaled_policy_states() {
        assert_eq!(
            from_status(&view(PolicyState::Absent, None)),
            PolicyIntent::None
        );
        assert_eq!(
            from_status(&view(PolicyState::Unjournaled, None)),
            PolicyIntent::None
        );
        assert_eq!(
            from_status(&view(PolicyState::Preparing, None)),
            PolicyIntent::Preparing
        );
        assert_eq!(
            from_status(&view(PolicyState::Active, Some("580.159.02"))),
            PolicyIntent::Active {
                expected_module_version: "580.159.02".to_string()
            }
        );
        assert_eq!(
            from_status(&view(PolicyState::Drifted, Some("580.159.02"))),
            PolicyIntent::Drifted {
                expected_module_version: "580.159.02".to_string()
            }
        );
        assert_eq!(
            from_status(&view(PolicyState::MissingFragment, None)),
            PolicyIntent::MissingFragment
        );
        assert_eq!(
            from_status(&view(PolicyState::Releasing, None)),
            PolicyIntent::Releasing
        );
        assert_eq!(
            from_status(&view(PolicyState::ReleaseFailed, None)),
            PolicyIntent::ReleaseFailed
        );
    }

    #[test]
    fn active_and_drifted_without_a_proven_expected_version_map_to_unavailable() {
        assert_eq!(
            from_status(&view(PolicyState::Active, None)),
            PolicyIntent::Unavailable
        );
        assert_eq!(
            from_status(&view(PolicyState::Drifted, None)),
            PolicyIntent::Unavailable
        );
    }

    #[test]
    fn a_status_error_maps_to_unavailable() {
        let error = anyhow::anyhow!("residency journal is unreadable");
        assert_eq!(from_query(Err(error)), PolicyIntent::Unavailable);
    }

    #[test]
    fn a_successful_status_query_maps_through_from_status() {
        let success = Ok(view(PolicyState::Active, Some("580.159.02")));
        assert_eq!(
            from_query(success),
            PolicyIntent::Active {
                expected_module_version: "580.159.02".to_string()
            }
        );
    }

    #[cfg(feature = "sandbox")]
    #[test]
    fn query_maps_a_real_status_error_to_unavailable() {
        let _serial = serialized_env_tests();
        let journal_dir =
            std::env::temp_dir().join(format!("qol-policy-journal-file-{}", std::process::id()));
        std::fs::write(&journal_dir, b"not a directory").expect("fixture file");
        std::env::set_var("QOL_POLICY_JOURNAL_DIR", &journal_dir);
        std::env::remove_var("QOL_RESIDENT_FRAGMENT_PATH");
        assert_eq!(query(), PolicyIntent::Unavailable);
        std::env::remove_var("QOL_POLICY_JOURNAL_DIR");
        std::fs::remove_file(&journal_dir).ok();
    }

    #[test]
    fn notification_text_compares_only_the_exact_module_version() {
        let intent = PolicyIntent::Active {
            expected_module_version: "580.159.02".to_string(),
        };
        let intended = notification_text("580.150.00", "580.159.02", &intent);
        assert!(intended.contains("resident policy pins"));
        assert!(intended.contains("reboot"));
        assert!(!intended.contains("fail to start"));

        let divergent = notification_text("580.150.00", "580.200.00", &intent);
        assert!(divergent.contains("no longer matches the policy"));

        let generic = notification_text("580.150.00", "580.200.00", &PolicyIntent::None);
        assert!(generic.contains("fail to start"));
    }

    #[test]
    fn drifted_intent_keeps_the_expected_module_version_for_wording() {
        let drifted = PolicyIntent::Drifted {
            expected_module_version: "580.159.02".to_string(),
        };
        let text = notification_text("580.150.00", "580.173.00", &drifted);
        assert!(text.contains("drifted"));
    }

    #[test]
    fn explicit_release_and_unavailable_intents_use_honest_generic_wording() {
        for intent in [
            PolicyIntent::Releasing,
            PolicyIntent::ReleaseFailed,
            PolicyIntent::Unavailable,
        ] {
            let text = notification_text("580.150.00", "580.173.00", &intent);
            assert!(text.contains("fail to start"), "{intent:?}");
            assert!(!text.contains("resident policy"), "{intent:?}");
        }
        assert_eq!(PolicyIntent::Releasing.as_str(), "releasing");
        assert_eq!(PolicyIntent::ReleaseFailed.as_str(), "release-failed");
        assert_eq!(PolicyIntent::Unavailable.as_str(), "unavailable");
    }
}
