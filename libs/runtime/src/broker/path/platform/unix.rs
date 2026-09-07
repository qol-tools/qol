use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::super::BrokerPathError;

pub type BrokerListener = std::os::unix::net::UnixListener;

pub(super) fn broker_socket_path_for_uid(
    uid: u32,
    xdg_runtime_dir: Option<&str>,
) -> Result<PathBuf, BrokerPathError> {
    if let Some(xdg) = xdg_runtime_dir {
        let mut path = PathBuf::from(xdg);
        path.push(format!("qol-runtime-{uid}"));
        path.push("broker.sock");
        return Ok(path);
    }
    Ok(PathBuf::from(format!("/tmp/qol-runtime-{uid}.sock")))
}

pub(super) fn broker_socket_path() -> Result<PathBuf, BrokerPathError> {
    let uid = unsafe { libc::getuid() } as u32;
    let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
    broker_socket_path_for_uid(uid, xdg_runtime_dir.as_deref())
}

pub(super) fn bind_broker_listener(socket: &Path) -> Result<BrokerListener, BrokerPathError> {
    let parent = socket.parent().ok_or_else(|| {
        BrokerPathError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "broker socket path has no parent",
        ))
    })?;

    ensure_parent_dir(parent)?;

    match std::fs::remove_file(socket) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(BrokerPathError::Io(error)),
    }

    let listener = BrokerListener::bind(socket).map_err(BrokerPathError::Io)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .map_err(BrokerPathError::Io)?;
    Ok(listener)
}

fn ensure_parent_dir(parent: &Path) -> Result<(), BrokerPathError> {
    match std::fs::metadata(parent) {
        Ok(metadata) => {
            let mode = metadata.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(BrokerPathError::ParentPermissive {
                    parent: parent.to_path_buf(),
                    mode,
                });
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(parent).map_err(BrokerPathError::Io)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(BrokerPathError::Io)?;
            Ok(())
        }
        Err(error) => Err(BrokerPathError::Io(error)),
    }
}
