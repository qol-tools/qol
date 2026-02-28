use super::super::DaemonActionDispatch;
use qol_runtime::protocol::DaemonRequest;
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

const SOCKET_IO_TIMEOUT_MS: u64 = 80;

pub(super) fn dispatch_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    let mut stream = match UnixStream::connect(endpoint) {
        Ok(s) => s,
        Err(_) => return DaemonActionDispatch::Unavailable,
    };

    let timeout = Duration::from_millis(SOCKET_IO_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.set_read_timeout(Some(timeout));

    let request = DaemonRequest {
        action: action_id.to_string(),
    };
    let Ok(mut payload) = serde_json::to_string(&request) else {
        return DaemonActionDispatch::Unavailable;
    };
    payload.push('\n');

    if stream.write_all(payload.as_bytes()).is_err() {
        return DaemonActionDispatch::Unavailable;
    }
    let _ = stream.shutdown(Shutdown::Write);

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return DaemonActionDispatch::Unavailable,
        Ok(_) => {}
    }

    super::super::protocol::parse_response(line.trim())
}
