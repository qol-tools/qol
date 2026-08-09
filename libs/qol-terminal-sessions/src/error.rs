use std::fmt::{Display, Formatter};

use crate::{BackendId, SessionBinding, SessionId, SpawnSurface};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityError {
    message: String,
}

impl IdentityError {
    pub(crate) fn component(kind: &'static str, value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            message: format!("{kind} `{value}` contains unsupported characters"),
        }
    }

    pub(crate) fn binding(value: &str) -> Self {
        Self {
            message: format!("terminal binding `{value}` has an invalid format"),
        }
    }

    pub(crate) fn root_pid(value: i32) -> Self {
        Self {
            message: format!("terminal root process `{value}` must be positive"),
        }
    }
}

impl Display for IdentityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for IdentityError {}

#[derive(Debug)]
pub enum TerminalError {
    BackendUnavailable {
        backend: BackendId,
        source: std::io::Error,
    },
    CommandFailed {
        backend: BackendId,
        operation: &'static str,
        code: Option<i32>,
        stderr: String,
    },
    InvalidResponse {
        backend: BackendId,
        source: serde_json::Error,
    },
    DuplicateBackend(BackendId),
    UnknownBackend(BackendId),
    TargetMissing(SessionBinding),
    TargetChanged {
        target: SessionId,
        expected_root_pid: i32,
        actual_root_pid: i32,
    },
    Unsupported {
        target: SessionId,
        capability: &'static str,
    },
    SpawnUnsupported {
        backend: BackendId,
        surface: SpawnSurface,
    },
    SpawnFailed {
        backend: BackendId,
        message: String,
    },
}

impl Display for TerminalError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BackendUnavailable { backend, source } => {
                write!(formatter, "terminal backend `{backend}` is unavailable: {source}")
            }
            Self::CommandFailed {
                backend,
                operation,
                code,
                stderr,
            } => write!(
                formatter,
                "terminal backend `{backend}` failed to {operation} (code {code:?}): {stderr}"
            ),
            Self::InvalidResponse { backend, source } => {
                write!(formatter, "terminal backend `{backend}` returned invalid data: {source}")
            }
            Self::DuplicateBackend(backend) => {
                write!(formatter, "terminal backend `{backend}` was registered twice")
            }
            Self::UnknownBackend(backend) => {
                write!(formatter, "terminal backend `{backend}` is not registered")
            }
            Self::TargetMissing(target) => write!(
                formatter,
                "terminal session `{}` is no longer available",
                target.session_id()
            ),
            Self::TargetChanged {
                target,
                expected_root_pid,
                actual_root_pid,
            } => write!(
                formatter,
                "terminal session `{target}` changed process from {expected_root_pid} to {actual_root_pid}"
            ),
            Self::Unsupported { target, capability } => {
                write!(formatter, "terminal session `{target}` does not support {capability}")
            }
            Self::SpawnUnsupported { backend, surface } => write!(
                formatter,
                "terminal backend `{backend}` cannot spawn a {surface} terminal"
            ),
            Self::SpawnFailed { backend, message } => write!(
                formatter,
                "terminal backend `{backend}` failed to spawn a terminal: {message}"
            ),
        }
    }
}

impl std::error::Error for TerminalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BackendUnavailable { source, .. } => Some(source),
            Self::InvalidResponse { source, .. } => Some(source),
            Self::CommandFailed { .. }
            | Self::DuplicateBackend(_)
            | Self::UnknownBackend(_)
            | Self::TargetMissing(_)
            | Self::TargetChanged { .. }
            | Self::Unsupported { .. }
            | Self::SpawnUnsupported { .. }
            | Self::SpawnFailed { .. } => None,
        }
    }
}
