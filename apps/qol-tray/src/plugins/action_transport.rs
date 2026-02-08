use std::path::Path;

pub enum DaemonActionDispatch {
    Handled,
    Fallback,
    Unavailable,
}

pub fn dispatch_daemon_action(socket_path: &Path, action_id: &str) -> DaemonActionDispatch {
    if action_id.is_empty() {
        return DaemonActionDispatch::Fallback;
    }
    dispatch_daemon_action_impl(socket_path, action_id)
}

#[cfg(unix)]
fn dispatch_daemon_action_impl(socket_path: &Path, action_id: &str) -> DaemonActionDispatch {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let mut stream = match UnixStream::connect(socket_path) {
        Ok(stream) => stream,
        Err(_) => return DaemonActionDispatch::Unavailable,
    };

    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));

    let payload = format!("action:{action_id}\n");
    if stream.write_all(payload.as_bytes()).is_err() {
        return DaemonActionDispatch::Unavailable;
    }

    let mut buffer = [0u8; 128];
    let read_result = stream.read(&mut buffer);
    match read_result {
        Ok(0) => DaemonActionDispatch::Handled,
        Ok(n) => parse_response(&buffer[..n]),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => DaemonActionDispatch::Handled,
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => DaemonActionDispatch::Handled,
        Err(_) => DaemonActionDispatch::Handled,
    }
}

#[cfg(not(unix))]
fn dispatch_daemon_action_impl(_socket_path: &Path, _action_id: &str) -> DaemonActionDispatch {
    DaemonActionDispatch::Unavailable
}

#[cfg(unix)]
fn parse_response(bytes: &[u8]) -> DaemonActionDispatch {
    let raw = match std::str::from_utf8(bytes) {
        Ok(value) => value.trim().to_ascii_lowercase(),
        Err(_) => return DaemonActionDispatch::Handled,
    };
    if raw.is_empty() || raw == "ok" || raw == "handled" {
        return DaemonActionDispatch::Handled;
    }
    if raw == "fallback" {
        return DaemonActionDispatch::Fallback;
    }
    DaemonActionDispatch::Handled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn parse_response_cases() {
        let cases = [
            (b"ok\n".as_slice(), DaemonActionDispatch::Handled),
            (b"handled".as_slice(), DaemonActionDispatch::Handled),
            (b"fallback\n".as_slice(), DaemonActionDispatch::Fallback),
            (b"".as_slice(), DaemonActionDispatch::Handled),
            (b"weird".as_slice(), DaemonActionDispatch::Handled),
        ];

        for (input, expected) in cases {
            let got = parse_response(input);
            assert_eq!(
                std::mem::discriminant(&got),
                std::mem::discriminant(&expected)
            );
        }
    }
}
