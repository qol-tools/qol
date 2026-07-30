use std::io::{self, ErrorKind};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::broker::{is_same_uid, peer_cred};

pub type LocalListener = std::os::unix::net::UnixListener;
pub type LocalStream = std::os::unix::net::UnixStream;

pub(super) fn bind_listener(path: &Path) -> io::Result<LocalListener> {
    let listener = LocalListener::bind(path)?;
    let permissions = std::fs::Permissions::from_mode(0o600);
    if let Err(error) = std::fs::set_permissions(path, permissions) {
        drop(listener);
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(listener)
}

pub(super) fn authorize_peer(stream: &LocalStream) -> io::Result<()> {
    let credential = peer_cred(stream)?;
    if is_same_uid(&credential) {
        return Ok(());
    }
    Err(io::Error::new(
        ErrorKind::PermissionDenied,
        "local IPC peer belongs to a different user",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Write};

    #[test]
    fn listener_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.sock");

        let _listener = bind_listener(&path).unwrap();

        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn current_user_peer_is_authorized() {
        let (server, _client) = LocalStream::pair().unwrap();

        assert!(authorize_peer(&server).is_ok());
    }

    #[test]
    fn bounded_line_does_not_wait_for_newline_after_limit() {
        let (mut writer, reader) = LocalStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            writer.write_all(&vec![b'a'; crate::local_ipc::MAX_MESSAGE_BYTES + 1])
        });
        let mut reader = BufReader::new(reader);

        let result = crate::local_ipc::read_line(&mut reader);
        writer.join().unwrap().unwrap();

        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);
    }
}
