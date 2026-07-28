use std::io::{self, BufRead, ErrorKind};
use std::path::Path;

mod platform;

pub use platform::{LocalListener, LocalStream};

pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

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
    fn bounded_line_rejects_unterminated_overflow() {
        let input = vec![b'a'; MAX_MESSAGE_BYTES + 1];
        let mut reader = BufReader::new(input.as_slice());

        let result = read_line(&mut reader);

        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);
    }
}
