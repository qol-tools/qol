use super::DaemonActionDispatch;
use std::path::Path;

const SOCKET_IO_TIMEOUT_MS: u64 = 80;

pub(super) fn dispatch_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    let mut fallback_seen = false;

    for payload in payload_attempts(action_id) {
        match send_payload(endpoint, &payload) {
            DaemonActionDispatch::Handled => return DaemonActionDispatch::Handled,
            DaemonActionDispatch::Error(message) => return DaemonActionDispatch::Error(message),
            DaemonActionDispatch::Fallback => fallback_seen = true,
            DaemonActionDispatch::Unavailable => {}
        }
    }

    if fallback_seen {
        DaemonActionDispatch::Fallback
    } else {
        DaemonActionDispatch::Unavailable
    }
}

fn send_payload(endpoint: &Path, payload: &str) -> DaemonActionDispatch {
    use std::io::{Read, Write};
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let mut stream = match UnixStream::connect(endpoint) {
        Ok(stream) => stream,
        Err(_) => return DaemonActionDispatch::Unavailable,
    };

    let timeout = Duration::from_millis(SOCKET_IO_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.set_read_timeout(Some(timeout));

    if stream.write_all(payload.as_bytes()).is_err() {
        return DaemonActionDispatch::Unavailable;
    }
    let _ = stream.shutdown(Shutdown::Write);

    let mut buffer = [0u8; 128];
    let read_result = stream.read(&mut buffer);
    match read_result {
        Ok(0) => DaemonActionDispatch::Unavailable,
        Ok(n) => super::protocol::parse_response(&buffer[..n]),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
            DaemonActionDispatch::Unavailable
        }
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
            DaemonActionDispatch::Unavailable
        }
        Err(_) => DaemonActionDispatch::Unavailable,
    }
}

fn payload_attempts(action_id: &str) -> Vec<String> {
    let mut payloads = vec![format!("action:{action_id}\n"), format!("{action_id}\n")];
    if action_id == "open" {
        payloads.insert(0, "show".to_string());
    }
    payloads
}
