use super::DaemonActionDispatch;
use std::path::Path;

pub(super) fn dispatch_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    for payload in super::payload_candidates(action_id) {
        let dispatch = send_payload(endpoint, &payload);
        if !matches!(dispatch, DaemonActionDispatch::Unavailable) {
            return dispatch;
        }
    }
    DaemonActionDispatch::Unavailable
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

    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));

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
