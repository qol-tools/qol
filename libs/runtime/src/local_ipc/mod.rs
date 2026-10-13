use std::ffi::OsStr;
use std::io::{self, BufRead, ErrorKind};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

mod platform;
mod secret;
pub use secret::{encode_secret_json, read_secret_line, SecretMessageError, SecretReader};

pub use platform::{is_socket_file, peer_pid, LocalListener, LocalStream};

pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

pub const MAX_SOCKET_PATH_BYTES: usize = platform::MAX_SOCKET_PATH_BYTES;

const SHORT_NAME_DIGEST_BYTES: usize = 4;

pub fn socket_path(dir: &Path, file_name: &OsStr) -> PathBuf {
    let path = dir.join(file_name);
    if path.as_os_str().len() <= MAX_SOCKET_PATH_BYTES {
        return path;
    }
    let digest = Sha256::digest(file_name.as_encoded_bytes());
    let short: String = digest[..SHORT_NAME_DIGEST_BYTES]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    dir.join(format!("{short}.sock"))
}

pub fn bind_listener(path: &Path) -> io::Result<LocalListener> {
    platform::bind_listener(path)
}

pub fn authorize_peer(stream: &LocalStream) -> io::Result<()> {
    platform::authorize_peer(stream)
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
    use std::io::BufReader;

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
    fn a_socket_name_that_does_not_fit_shortens_to_a_stable_digest() {
        if MAX_SOCKET_PATH_BYTES == usize::MAX {
            return;
        }
        let dir = PathBuf::from("d".repeat(MAX_SOCKET_PATH_BYTES - 20));
        let fitting = OsStr::new("qol-a.sock");
        let long = OsStr::new("qol-window-actions.sock");
        let other = OsStr::new("qol-window-actions-2.sock");

        assert_eq!(socket_path(&dir, fitting), dir.join(fitting));
        let short = socket_path(&dir, long);
        let name = short.file_name().unwrap().to_str().unwrap();
        assert_eq!(short.parent(), Some(dir.as_path()));
        assert_eq!(name.len(), 13, "{name}");
        assert!(name.ends_with(".sock"), "{name}");
        assert!(
            name[..8].chars().all(|digit| digit.is_ascii_hexdigit()),
            "{name}"
        );
        assert!(short.as_os_str().len() <= MAX_SOCKET_PATH_BYTES);
        assert_eq!(socket_path(&dir, long), short);
        assert_ne!(socket_path(&dir, other), short);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_shortened_socket_path_binds_where_the_full_name_cannot() {
        let tmp = tempfile::tempdir().unwrap();
        let long = OsStr::new("qol-window-actions.sock");
        let padding = MAX_SOCKET_PATH_BYTES - tmp.path().as_os_str().len() - 20;
        let dir = tmp.path().join("d".repeat(padding));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(dir.join(long).as_os_str().len() > MAX_SOCKET_PATH_BYTES);

        let socket = socket_path(&dir, long);

        let listener = bind_listener(&socket)
            .unwrap_or_else(|error| panic!("{} did not bind: {error}", socket.display()));
        drop(listener);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn only_a_bound_socket_counts_as_a_socket_file() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s");
        let _listener = bind_listener(&socket).unwrap();
        let plain = dir.path().join("plain");
        std::fs::write(&plain, "x").unwrap();

        assert!(is_socket_file(&socket));
        assert!(!is_socket_file(&plain));
        assert!(!is_socket_file(dir.path()));
        assert!(!is_socket_file(&dir.path().join("missing")));
    }

    #[test]
    fn bounded_line_rejects_unterminated_overflow() {
        let input = vec![b'a'; MAX_MESSAGE_BYTES + 1];
        let mut reader = BufReader::new(input.as_slice());

        let result = read_line(&mut reader);

        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);
    }
}
