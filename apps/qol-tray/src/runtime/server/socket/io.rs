use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde::Serialize;

use super::super::shared::SharedState;

const IO_TIMEOUT_MS: u64 = 50;

pub(super) fn prepare_stream(stream: UnixStream) -> Option<(BufReader<UnixStream>, UnixStream)> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(IO_TIMEOUT_MS)));
    let writer = stream.try_clone().ok()?;
    Some((BufReader::new(stream), writer))
}

pub(super) fn read_request(reader: &mut BufReader<UnixStream>) -> Option<String> {
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return None;
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

pub(super) fn write_flushed_json_line<T: Serialize>(writer: &mut UnixStream, value: &T) -> bool {
    if !write_json_line(writer, value) {
        return false;
    }
    writer.flush().is_ok()
}

pub(super) fn write_json_line<T: Serialize>(writer: &mut UnixStream, value: &T) -> bool {
    let Ok(json) = serde_json::to_string(value) else {
        return false;
    };

    if writer.write_all(json.as_bytes()).is_err() {
        return false;
    }

    writer.write_all(b"\n").is_ok()
}

pub(super) fn write_state(writer: &mut UnixStream, shared: &SharedState) {
    let _ = write_json_line(writer, &shared.build_state());
}
