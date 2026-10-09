use qol_host_fixes::policy::{
    read_journal, JournalState, PolicyError, ResidencyOwnerId, RestoreOutcome,
};

use crate::monitor::grant::{GrantError, I2cGrantState, RevokeOutcome, GRANT_OWNER};

pub(crate) fn grant() -> Result<(), GrantError> {
    qol_host_fixes::udev::grant(&owner()?).map_err(from_host_fixes)
}

pub(crate) fn revoke() -> Result<RevokeOutcome, GrantError> {
    match qol_host_fixes::udev::revoke(&owner()?).map_err(from_host_fixes)? {
        RestoreOutcome::Restored | RestoreOutcome::DeletedZeroMutation => {
            Ok(RevokeOutcome::Restored)
        }
        RestoreOutcome::NothingToRestore => Ok(RevokeOutcome::NothingToRestore),
    }
}

pub(crate) fn state() -> I2cGrantState {
    match read_journal(qol_host_fixes::udev::UDEV_UACCESS_POLICY_ID) {
        Ok(None) => I2cGrantState::None,
        Ok(Some(journal)) => match journal.state {
            JournalState::Active => {
                let owner = journal
                    .owners
                    .iter()
                    .map(|granted| granted.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                I2cGrantState::Active { owner }
            }
            JournalState::Preparing => I2cGrantState::Preparing,
            JournalState::Releasing => I2cGrantState::Releasing,
            JournalState::ReleaseFailed => I2cGrantState::ReleaseFailed,
        },
        Err(error) => I2cGrantState::Unreadable {
            message: format!("{error:#}"),
        },
    }
}

fn owner() -> Result<ResidencyOwnerId, GrantError> {
    ResidencyOwnerId::parse(GRANT_OWNER).map_err(|error| GrantError::Other {
        message: format!("invalid grant owner id: {error}"),
    })
}

fn from_host_fixes(error: anyhow::Error) -> GrantError {
    if let Some(policy_error) = error.downcast_ref::<PolicyError>() {
        return match policy_error {
            PolicyError::Busy { detail, .. } => GrantError::Busy {
                detail: detail.clone(),
            },
            PolicyError::RuleConflict {
                path,
                expected_sha256,
                actual_sha256,
                ..
            } => GrantError::RuleConflict {
                path: path.clone(),
                expected_sha256: expected_sha256.clone(),
                actual_sha256: actual_sha256.clone(),
            },
            PolicyError::PlatformUnsupported { .. } => {
                GrantError::unsupported("udev uaccess grants are not implemented on this platform")
            }
            other => GrantError::Other {
                message: other.to_string(),
            },
        };
    }
    GrantError::Other {
        message: format!("{error:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_error_maps_the_live_policy_error_variants() {
        let busy = from_host_fixes(anyhow::Error::new(PolicyError::Busy {
            policy: "udev-i2c-uaccess".into(),
            detail: "already active".into(),
        }));
        match busy {
            GrantError::Busy { detail } => assert_eq!(detail, "already active"),
            other => panic!("expected Busy, got {other:?}"),
        }

        let conflict = from_host_fixes(anyhow::Error::new(PolicyError::RuleConflict {
            policy: "udev-i2c-uaccess".into(),
            path: "/etc/udev/rules.d/90-qol-i2c-uaccess.rules".into(),
            expected_sha256: "a".repeat(64),
            actual_sha256: "b".repeat(64),
        }));
        match conflict {
            GrantError::RuleConflict {
                path,
                expected_sha256,
                actual_sha256,
            } => {
                assert_eq!(path, "/etc/udev/rules.d/90-qol-i2c-uaccess.rules");
                assert_eq!(expected_sha256, "a".repeat(64));
                assert_eq!(actual_sha256, "b".repeat(64));
            }
            other => panic!("expected RuleConflict, got {other:?}"),
        }

        let unsupported = from_host_fixes(anyhow::Error::new(PolicyError::PlatformUnsupported {
            policy: "udev-i2c-uaccess".into(),
        }));
        assert!(matches!(unsupported, GrantError::Unsupported { .. }));
    }

    #[test]
    fn grant_error_maps_unrelated_errors_to_other() {
        let error = from_host_fixes(anyhow::anyhow!("udevadm exploded"));
        assert_eq!(
            error.to_string(),
            "udevadm exploded",
            "unrelated errors must surface verbatim"
        );
    }
}
