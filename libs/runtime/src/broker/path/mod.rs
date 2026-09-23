//! Per-uid socket bind path and permission-checked bind helper.
//!
//! See `docs/adr/RUNTIME-1-af-unix-broker-with-peer-cred-auth-and-pull-pane-f.md`
//! and the security plan card 06 / card 11.

use std::path::{Path, PathBuf};

mod platform;

pub use platform::BrokerListener;

/// Errors surfaced by `bind_broker_listener`.
///
/// Each variant pins a distinct refusal reason so a daemon can decide
/// whether to retry, surface a banner to the user, or abort startup.
#[derive(Debug)]
pub enum BrokerPathError {
    Unsupported,
    /// `parent` exists but its mode bits include any access for group or
    /// world (`mode & 0o077 != 0`). The broker refuses to bind rather
    /// than tighten the parent's bits silently, because doing so could
    /// race a concurrent reader.
    ParentPermissive {
        parent: PathBuf,
        mode: u32,
    },
    /// I/O failure during socket-dir setup, socket bind, or chmod.
    Io(std::io::Error),
}

impl std::fmt::Display for BrokerPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrokerPathError::Unsupported => {
                f.write_str("broker sockets are unavailable on this platform")
            }
            BrokerPathError::ParentPermissive { parent, mode } => write!(
                f,
                "broker socket parent {} has mode {:o}; refusing to bind",
                parent.display(),
                mode
            ),
            BrokerPathError::Io(e) => write!(f, "broker bind I/O error: {e}"),
        }
    }
}

impl std::error::Error for BrokerPathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BrokerPathError::Io(e) => Some(e),
            BrokerPathError::ParentPermissive { .. } | BrokerPathError::Unsupported => None,
        }
    }
}

impl From<std::io::Error> for BrokerPathError {
    fn from(value: std::io::Error) -> Self {
        BrokerPathError::Io(value)
    }
}

/// Resolve the broker socket path for a given uid.
///
/// If `xdg_runtime_dir` is `Some`, the socket lives at
/// `<xdg>/qol-runtime/broker.sock`. Otherwise it falls back to
/// `/tmp/qol-runtime-<uid>.sock`. The uid is always part of the path so
/// two users on the same machine never share a socket file.
pub fn broker_socket_path_for_uid(
    uid: u32,
    xdg_runtime_dir: Option<&str>,
) -> Result<PathBuf, BrokerPathError> {
    platform::broker_socket_path_for_uid(uid, xdg_runtime_dir)
}

/// Resolve the broker socket path for the current uid using the process
/// environment ($XDG_RUNTIME_DIR or fallback).
pub fn broker_socket_path() -> Result<PathBuf, BrokerPathError> {
    platform::broker_socket_path()
}

/// Bind a `UnixListener` at `sock` with the AF_UNIX security invariant:
/// parent dir mode 0700, socket file mode 0600, stale socket files
/// unlinked before bind. If the parent dir already exists with any bit
/// set in `mode & 0o077`, refuse with `ParentPermissive`.
pub fn bind_broker_listener(sock: &Path) -> Result<BrokerListener, BrokerPathError> {
    platform::bind_broker_listener(sock)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::BrokerPathError;

    #[test]
    fn unsupported_error_is_typed_and_source_free() {
        let error = BrokerPathError::Unsupported;
        assert_eq!(
            error.to_string(),
            "broker sockets are unavailable on this platform"
        );
        assert!(error.source().is_none());
    }
}
