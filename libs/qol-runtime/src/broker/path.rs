//! Per-uid socket bind path and permission-checked bind helper.
//!
//! See `docs/adr/RUNTIME-1-af-unix-broker-with-peer-cred-auth-and-pull-pane-f.md`
//! and the security plan card 06 / card 11.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

/// Errors surfaced by `bind_broker_listener`.
///
/// Each variant pins a distinct refusal reason so a daemon can decide
/// whether to retry, surface a banner to the user, or abort startup.
#[derive(Debug)]
pub enum BrokerPathError {
    /// `parent` exists but its mode bits include any access for group or
    /// world (`mode & 0o077 != 0`). The broker refuses to bind rather
    /// than tighten the parent's bits silently, because doing so could
    /// race a concurrent reader.
    ParentPermissive { parent: PathBuf, mode: u32 },
    /// I/O failure during socket-dir setup, socket bind, or chmod.
    Io(std::io::Error),
}

impl std::fmt::Display for BrokerPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
            _ => None,
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
pub fn broker_socket_path_for_uid(uid: u32, xdg_runtime_dir: Option<&str>) -> PathBuf {
    if let Some(xdg) = xdg_runtime_dir {
        let mut p = PathBuf::from(xdg);
        // Embed the uid in a per-user subdir so the path remains
        // distinct from any sibling tenant's path even if XDG is
        // misconfigured to be shared.
        p.push(format!("qol-runtime-{uid}"));
        p.push("broker.sock");
        p
    } else {
        PathBuf::from(format!("/tmp/qol-runtime-{uid}.sock"))
    }
}

/// Resolve the broker socket path for the current uid using the process
/// environment ($XDG_RUNTIME_DIR or fallback).
pub fn broker_socket_path() -> PathBuf {
    let uid = current_uid();
    let xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    broker_socket_path_for_uid(uid, xdg.as_deref())
}

fn current_uid() -> u32 {
    // SAFETY: getuid() always succeeds.
    unsafe { libc::getuid() as u32 }
}

/// Bind a `UnixListener` at `sock` with the AF_UNIX security invariant:
/// parent dir mode 0700, socket file mode 0600, stale socket files
/// unlinked before bind. If the parent dir already exists with any bit
/// set in `mode & 0o077`, refuse with `ParentPermissive`.
pub fn bind_broker_listener(sock: &Path) -> Result<UnixListener, BrokerPathError> {
    let parent = sock.parent().ok_or_else(|| {
        BrokerPathError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "broker socket path has no parent",
        ))
    })?;

    ensure_parent_dir(parent)?;

    // Remove a stale socket (or any leftover file) before binding.
    // bind(2) returns EADDRINUSE if the path exists, so unlink first.
    match std::fs::remove_file(sock) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(BrokerPathError::Io(e)),
    }

    let listener = UnixListener::bind(sock).map_err(BrokerPathError::Io)?;
    std::fs::set_permissions(sock, std::fs::Permissions::from_mode(0o600))
        .map_err(BrokerPathError::Io)?;
    Ok(listener)
}

fn ensure_parent_dir(parent: &Path) -> Result<(), BrokerPathError> {
    match std::fs::metadata(parent) {
        Ok(meta) => {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(BrokerPathError::ParentPermissive {
                    parent: parent.to_path_buf(),
                    mode,
                });
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(parent).map_err(BrokerPathError::Io)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(BrokerPathError::Io)?;
            Ok(())
        }
        Err(e) => Err(BrokerPathError::Io(e)),
    }
}
