use std::io::{self, BufRead, ErrorKind};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use crate::broker::{is_same_uid, peer_cred};

pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

pub fn bind_listener(path: &Path) -> io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    let permissions = std::fs::Permissions::from_mode(0o600);
    if let Err(error) = std::fs::set_permissions(path, permissions) {
        drop(listener);
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(listener)
}

pub fn authorize_peer(stream: &UnixStream) -> io::Result<()> {
    let credential = peer_cred(stream)?;
    if is_same_uid(&credential) {
        return Ok(());
    }
    Err(io::Error::new(
        ErrorKind::PermissionDenied,
        "local IPC peer belongs to a different user",
    ))
}

pub fn read_line<R: BufRead>(reader: &mut R) -> io::Result<Option<String>> {
    let mut bytes = Vec::with_capacity(1024);
    let mut limited = std::io::Read::take(reader, (MAX_MESSAGE_BYTES + 1) as u64);
    let read = limited.read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "local IPC message exceeds 64 KiB",
        ));
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
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
        let (server, _client) = UnixStream::pair().unwrap();

        assert!(authorize_peer(&server).is_ok());
    }

    #[test]
    fn bounded_line_accepts_limit_and_rejects_overflow() {
        let cases = [
            (MAX_MESSAGE_BYTES - 1, true),
            (MAX_MESSAGE_BYTES, false),
            (MAX_MESSAGE_BYTES + 1, false),
        ];
        for (size, accepted) in cases {
            let mut input = vec![b'a'; size];
            input.push(b'\n');
            let mut reader = BufReader::new(input.as_slice());

            let result = read_line(&mut reader);

            assert_eq!(result.is_ok(), accepted, "size={size}");
        }
    }

    #[test]
    fn bounded_line_does_not_wait_for_newline_after_limit() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer
            .write_all(&vec![b'a'; MAX_MESSAGE_BYTES + 1])
            .unwrap();
        let mut reader = BufReader::new(reader);

        let result = read_line(&mut reader);

        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);
    }
}
