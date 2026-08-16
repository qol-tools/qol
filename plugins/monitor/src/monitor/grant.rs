use std::fmt;

pub const GRANT_OWNER: &str = "plugin-monitor";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum I2cGrantState {
    Active { owner: String },
    Preparing,
    Releasing,
    ReleaseFailed,
    Unreadable { message: String },
    None,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeOutcome {
    NothingToRestore,
    Restored,
}

#[derive(Debug, Clone)]
pub enum GrantError {
    Busy {
        detail: String,
    },
    RuleConflict {
        path: String,
        expected_sha256: String,
        actual_sha256: String,
    },
    Unsupported {
        reason: String,
    },
    Other {
        message: String,
    },
}

impl GrantError {
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported {
            reason: reason.into(),
        }
    }

    #[cfg(target_os = "linux")]
    fn from_host_fixes(error: anyhow::Error) -> Self {
        use qol_host_fixes::policy::PolicyError;

        if let Some(policy_error) = error.downcast_ref::<PolicyError>() {
            return match policy_error {
                PolicyError::Busy { detail, .. } => Self::Busy {
                    detail: detail.clone(),
                },
                PolicyError::RuleConflict {
                    path,
                    expected_sha256,
                    actual_sha256,
                    ..
                } => Self::RuleConflict {
                    path: path.clone(),
                    expected_sha256: expected_sha256.clone(),
                    actual_sha256: actual_sha256.clone(),
                },
                PolicyError::PlatformUnsupported { .. } => {
                    Self::unsupported("udev uaccess grants are not implemented on this platform")
                }
                other => Self::Other {
                    message: other.to_string(),
                },
            };
        }
        Self::Other {
            message: format!("{error:#}"),
        }
    }
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy { detail } => write!(f, "the i2c uaccess grant is busy: {detail}"),
            Self::RuleConflict {
                path,
                expected_sha256,
                actual_sha256,
            } => write!(
                f,
                "refusing to touch the modified uaccess rule {path} (expected sha256 \
                 {expected_sha256}, actual sha256 {actual_sha256}); remove or restore it, then \
                 retry"
            ),
            Self::Unsupported { reason } => write!(f, "{reason}"),
            Self::Other { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for GrantError {}

pub trait GrantBackend: Send + Sync {
    fn grant(&self) -> Result<(), GrantError>;
    fn revoke(&self) -> Result<RevokeOutcome, GrantError>;
    fn state(&self) -> I2cGrantState;
}

pub struct UdevGrantBackend;

impl GrantBackend for UdevGrantBackend {
    fn grant(&self) -> Result<(), GrantError> {
        Self::grant_inner()
    }

    fn revoke(&self) -> Result<RevokeOutcome, GrantError> {
        Self::revoke_inner()
    }

    fn state(&self) -> I2cGrantState {
        Self::state_inner()
    }
}

#[cfg(target_os = "linux")]
impl UdevGrantBackend {
    fn grant_inner() -> Result<(), GrantError> {
        qol_host_fixes::udev::grant(&owner()?).map_err(GrantError::from_host_fixes)
    }

    fn revoke_inner() -> Result<RevokeOutcome, GrantError> {
        match qol_host_fixes::udev::revoke(&owner()?).map_err(GrantError::from_host_fixes)? {
            qol_host_fixes::policy::RestoreOutcome::Restored
            | qol_host_fixes::policy::RestoreOutcome::DeletedZeroMutation => {
                Ok(RevokeOutcome::Restored)
            }
            qol_host_fixes::policy::RestoreOutcome::NothingToRestore => {
                Ok(RevokeOutcome::NothingToRestore)
            }
        }
    }

    fn state_inner() -> I2cGrantState {
        use qol_host_fixes::policy::{read_journal, JournalState};

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
}

#[cfg(target_os = "linux")]
fn owner() -> Result<qol_host_fixes::policy::ResidencyOwnerId, GrantError> {
    qol_host_fixes::policy::ResidencyOwnerId::parse(GRANT_OWNER).map_err(|error| {
        GrantError::Other {
            message: format!("invalid grant owner id: {error}"),
        }
    })
}

#[cfg(not(target_os = "linux"))]
impl UdevGrantBackend {
    fn grant_inner() -> Result<(), GrantError> {
        Err(GrantError::unsupported("i2c uaccess grants require Linux"))
    }

    fn revoke_inner() -> Result<RevokeOutcome, GrantError> {
        Err(GrantError::unsupported("i2c uaccess grants require Linux"))
    }

    fn state_inner() -> I2cGrantState {
        I2cGrantState::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_error_renders_every_variant() {
        assert_eq!(
            GrantError::Busy {
                detail: "already active".into()
            }
            .to_string(),
            "the i2c uaccess grant is busy: already active"
        );
        assert_eq!(
            GrantError::RuleConflict {
                path: "/etc/udev/rules.d/90-qol-i2c-uaccess.rules".into(),
                expected_sha256: "a".repeat(64),
                actual_sha256: "b".repeat(64),
            }
            .to_string(),
            "refusing to touch the modified uaccess rule /etc/udev/rules.d/90-qol-i2c-uaccess.rules (expected sha256 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, actual sha256 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb); remove or restore it, then retry"
        );
        assert_eq!(
            GrantError::Unsupported {
                reason: "i2c uaccess grants require Linux".into()
            }
            .to_string(),
            "i2c uaccess grants require Linux"
        );
        assert_eq!(
            GrantError::Other {
                message: "boom".into()
            }
            .to_string(),
            "boom"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_error_maps_the_live_policy_error_variants() {
        use qol_host_fixes::policy::PolicyError;

        let busy = GrantError::from_host_fixes(anyhow::Error::new(PolicyError::Busy {
            policy: "udev-i2c-uaccess".into(),
            detail: "already active".into(),
        }));
        match busy {
            GrantError::Busy { detail } => assert_eq!(detail, "already active"),
            other => panic!("expected Busy, got {other:?}"),
        }

        let conflict = GrantError::from_host_fixes(anyhow::Error::new(PolicyError::RuleConflict {
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

        let unsupported =
            GrantError::from_host_fixes(anyhow::Error::new(PolicyError::PlatformUnsupported {
                policy: "udev-i2c-uaccess".into(),
            }));
        assert!(matches!(unsupported, GrantError::Unsupported { .. }));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn grant_error_maps_unrelated_errors_to_other() {
        let error = GrantError::from_host_fixes(anyhow::anyhow!("udevadm exploded"));
        assert_eq!(
            error.to_string(),
            "udevadm exploded",
            "unrelated errors must surface verbatim"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_backend_stubs_are_typed_unsupported() {
        let backend = UdevGrantBackend;
        assert!(matches!(
            backend.grant(),
            Err(GrantError::Unsupported { .. })
        ));
        assert!(matches!(
            backend.revoke(),
            Err(GrantError::Unsupported { .. })
        ));
        assert_eq!(backend.state(), I2cGrantState::Unsupported);
    }
}
