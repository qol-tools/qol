use std::io::{self, BufRead, Read};

use serde::Serialize;
use zeroize::Zeroizing;

use super::MAX_MESSAGE_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretMessageError {
    TooLarge,
    Encode,
}

pub fn encode_secret_json(
    value: &impl Serialize,
) -> Result<Zeroizing<Vec<u8>>, SecretMessageError> {
    let mut bytes = Zeroizing::new(vec![0; MAX_MESSAGE_BYTES]);
    let mut cursor = io::Cursor::new(&mut bytes[..MAX_MESSAGE_BYTES - 1]);
    if let Err(error) = serde_json::to_writer(&mut cursor, value) {
        return Err(if error.is_io() {
            SecretMessageError::TooLarge
        } else {
            SecretMessageError::Encode
        });
    }
    let length = cursor.position() as usize;
    bytes.truncate(length);
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn read_secret_line(reader: &mut impl BufRead) -> io::Result<Option<Zeroizing<String>>> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_MESSAGE_BYTES + 1));
    let mut limited = Read::take(reader, (MAX_MESSAGE_BYTES + 1) as u64);
    if limited.read_until(b'\n', &mut bytes)? == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "local IPC message exceeds 64 KiB",
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid local IPC text"))?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    Ok(Some(Zeroizing::new(text.to_owned())))
}

pub struct SecretReader<R> {
    reader: R,
    buffer: Zeroizing<[u8; 8192]>,
    start: usize,
    end: usize,
}

impl<R> SecretReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Zeroizing::new([0; 8192]),
            start: 0,
            end: 0,
        }
    }
}

impl<R: Read> BufRead for SecretReader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.start == self.end {
            self.end = self.reader.read(&mut self.buffer[..])?;
            self.start = 0;
        }
        Ok(&self.buffer[self.start..self.end])
    }

    fn consume(&mut self, amount: usize) {
        self.start = self.start.saturating_add(amount).min(self.end);
    }
}

impl<R: Read> Read for SecretReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let bytes = self.fill_buf()?;
        let count = bytes.len().min(output.len());
        output[..count].copy_from_slice(&bytes[..count]);
        self.consume(count);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_reader_and_writer_preserve_limits_and_sanitize_invalid_input() {
        for length in [
            MAX_MESSAGE_BYTES - 1,
            MAX_MESSAGE_BYTES,
            MAX_MESSAGE_BYTES + 1,
        ] {
            let bytes = vec![b'x'; length];
            let mut reader = SecretReader::new(bytes.as_slice());
            assert_eq!(
                read_secret_line(&mut reader).is_ok(),
                length <= MAX_MESSAGE_BYTES,
                "{length}"
            );
        }
        let mut reader = SecretReader::new(b"synthetic-canary\xff\n".as_slice());
        let error = read_secret_line(&mut reader).unwrap_err().to_string();
        assert_eq!(error, "invalid local IPC text");
        for (length, valid) in [
            (MAX_MESSAGE_BYTES - 3, true),
            (MAX_MESSAGE_BYTES - 2, false),
        ] {
            let value = "x".repeat(length);
            let result = encode_secret_json(&value);
            assert_eq!(result.is_ok(), valid, "{length}");
        }
    }
}
