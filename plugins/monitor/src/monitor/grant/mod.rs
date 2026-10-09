use std::fmt;

mod platform;

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
        platform::grant()
    }

    fn revoke(&self) -> Result<RevokeOutcome, GrantError> {
        platform::revoke()
    }

    fn state(&self) -> I2cGrantState {
        platform::state()
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
