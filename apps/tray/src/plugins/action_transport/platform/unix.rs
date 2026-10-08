use crate::plugins::action_transport::DaemonActionDispatch;
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(10);
type DispatchResult<T> = Result<T, ()>;

pub(super) fn dispatch_action(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    let Ok(payload) = request_payload(action_id, input) else {
        return DaemonActionDispatch::NotSent;
    };
    dispatch_payload(endpoint, payload.as_bytes(), timeout)
}

pub(in crate::plugins::action_transport) fn dispatch_payload(
    endpoint: &Path,
    payload: &[u8],
    timeout: Duration,
) -> DaemonActionDispatch {
    if payload.len() > qol_runtime::local_ipc::MAX_MESSAGE_BYTES {
        return DaemonActionDispatch::NotSent;
    }
    let deadline = Instant::now() + timeout;
    let Ok(mut stream) = connect_stream(endpoint, timeout) else {
        return DaemonActionDispatch::NotSent;
    };
    if write_payload(&mut stream, payload, deadline).is_err() {
        return DaemonActionDispatch::OutcomeUnknown;
    }
    let _ = stream.shutdown(Shutdown::Write);
    let Ok(line) = read_response(stream, deadline) else {
        return DaemonActionDispatch::OutcomeUnknown;
    };
    parse_response(line.trim())
}

pub(super) fn can_connect(endpoint: &Path) -> bool {
    connect_stream(endpoint, DEFAULT_IO_TIMEOUT).is_ok()
}

fn connect_stream(endpoint: &Path, timeout: Duration) -> DispatchResult<UnixStream> {
    let stream = connect_bounded(endpoint, timeout)?;
    qol_runtime::local_ipc::authorize_peer(&stream).map_err(|_| ())?;
    apply_timeout(&stream, timeout);
    Ok(stream)
}

fn connect_bounded(endpoint: &Path, timeout: Duration) -> DispatchResult<UnixStream> {
    use std::os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    };
    let bytes = endpoint.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(());
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        address.sun_len = length as u8;
    }
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(());
    }
    let owned = unsafe { OwnedFd::from_raw_fd(raw) };
    if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(());
    }
    let stream = UnixStream::from(owned);
    stream.set_nonblocking(true).map_err(|_| ())?;
    let result = unsafe {
        libc::connect(
            stream.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length as libc::socklen_t,
        )
    };
    if result != 0 {
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(());
        }
        let mut poll = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        let millis = timeout.as_millis().clamp(1, i32::MAX as u128) as i32;
        if unsafe { libc::poll(&mut poll, 1, millis) } <= 0
            || stream.take_error().map_err(|_| ())?.is_some()
        {
            return Err(());
        }
    }
    stream.set_nonblocking(false).map_err(|_| ())?;
    Ok(stream)
}

fn apply_timeout(stream: &UnixStream, timeout: Duration) {
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.set_read_timeout(Some(timeout));
}

fn request_payload(action_id: &str, input: &serde_json::Value) -> DispatchResult<String> {
    let request = DaemonRequest {
        action: action_id.to_string(),
        input: input.clone(),
    };
    let mut payload = serde_json::to_string(&request).map_err(|_| ())?;
    payload.push('\n');
    if payload.len() > qol_runtime::local_ipc::MAX_MESSAGE_BYTES {
        return Err(());
    }
    Ok(payload)
}

fn write_payload(
    stream: &mut UnixStream,
    mut payload: &[u8],
    deadline: Instant,
) -> DispatchResult<()> {
    while !payload.is_empty() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|value| !value.is_zero())
            .ok_or(())?;
        stream.set_write_timeout(Some(remaining)).map_err(|_| ())?;
        let count = stream.write(payload).map_err(|_| ())?;
        if count == 0 {
            return Err(());
        }
        payload = &payload[count..];
    }
    Ok(())
}

fn read_response(mut stream: UnixStream, deadline: Instant) -> DispatchResult<String> {
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|value| !value.is_zero())
            .ok_or(())?;
        stream.set_read_timeout(Some(remaining)).map_err(|_| ())?;
        let mut chunk = [0; 1024];
        let count = stream.read(&mut chunk).map_err(|_| ())?;
        if count == 0 || bytes.len() + count > qol_runtime::local_ipc::MAX_MESSAGE_BYTES {
            return Err(());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
            if end + 1 != bytes.len() {
                return Err(());
            }
            return String::from_utf8(bytes).map_err(|_| ());
        }
    }
}

fn parse_response(line: &str) -> DaemonActionDispatch {
    if let Ok(response) = serde_json::from_str::<DaemonResponse>(line) {
        return match response {
            DaemonResponse::Handled { data } => DaemonActionDispatch::Handled { payload: data },
            DaemonResponse::Fallback => DaemonActionDispatch::Fallback,
            DaemonResponse::Error { message } => DaemonActionDispatch::Error(message),
            DaemonResponse::NotReady { phase, detail } => {
                DaemonActionDispatch::NotReady { phase, detail }
            }
        };
    }

    let word = line.split_whitespace().next().unwrap_or("");
    match word {
        "handled" if line == "handled" => DaemonActionDispatch::Handled { payload: None },
        "fallback" if line == "fallback" => DaemonActionDispatch::Fallback,
        "error" => {
            DaemonActionDispatch::Error(line.strip_prefix("error").unwrap_or("").trim().to_string())
        }
        _ => DaemonActionDispatch::OutcomeUnknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn dispatch_respects_short_read_timeout_when_listener_does_not_answer() {
        let dir = tempfile::TempDir::new().unwrap();
        let socket_path = dir.path().join("hung-daemon.sock");
        let _listener = UnixListener::bind(&socket_path).unwrap();
        let started = Instant::now();

        let dispatch = dispatch_action(
            &socket_path,
            "ping",
            &serde_json::Value::Null,
            Duration::from_millis(50),
        );

        assert!(matches!(dispatch, DaemonActionDispatch::OutcomeUnknown));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "readiness probes must not inherit the long action timeout"
        );
    }

    #[test]
    fn request_payload_serializes_structured_action_input() {
        let payload =
            request_payload("pair_device", &serde_json::json!({"address": "AA:BB"})).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(payload.trim()).unwrap(),
            serde_json::json!({
                "action": "pair_device",
                "input": {"address": "AA:BB"},
            })
        );
    }

    #[test]
    fn parse_response_cases() {
        let cases = [
            (
                r#"{"status":"handled"}"#,
                DaemonActionDispatch::Handled { payload: None },
            ),
            (r#"{"status":"fallback"}"#, DaemonActionDispatch::Fallback),
            (
                r#"{"status":"error","message":"daemon busy"}"#,
                DaemonActionDispatch::Error("daemon busy".to_string()),
            ),
            (
                r#"{"status":"error","message":""}"#,
                DaemonActionDispatch::Error(String::new()),
            ),
            ("handled", DaemonActionDispatch::Handled { payload: None }),
            ("fallback", DaemonActionDispatch::Fallback),
            (
                "error something broke",
                DaemonActionDispatch::Error("something broke".to_string()),
            ),
            ("", DaemonActionDispatch::OutcomeUnknown),
            ("garbage", DaemonActionDispatch::OutcomeUnknown),
        ];

        for (input, expected) in cases {
            let got = parse_response(input);
            assert_eq!(
                std::mem::discriminant(&got),
                std::mem::discriminant(&expected),
                "input: {:?}",
                input
            );
        }
    }

    #[test]
    fn parse_response_extracts_payload() {
        let input = r#"{"status":"handled","data":{"devices":[{"ieee":"0x123","online":true}]}}"#;
        let got = parse_response(input);
        match got {
            DaemonActionDispatch::Handled {
                payload: Some(value),
            } => {
                assert_eq!(
                    value,
                    serde_json::json!({"devices":[{"ieee":"0x123","online":true}]}),
                    "payload should carry JSON data"
                );
            }
            other => panic!("expected Handled with payload, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_handles_no_payload() {
        let input = r#"{"status":"handled"}"#;
        let got = parse_response(input);
        assert_eq!(got, DaemonActionDispatch::Handled { payload: None });
    }
    #[test]
    fn operation_transport_reply_loss_malformed_and_oversize_are_unknown() {
        for reply in [
            Vec::new(),
            b"{broken}\n".to_vec(),
            b"handled trailing-junk\n".to_vec(),
            vec![b'x'; qol_runtime::local_ipc::MAX_MESSAGE_BYTES + 1],
        ] {
            let root = tempfile::tempdir().unwrap();
            let socket = root.path().join("daemon.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                stream.read_to_string(&mut request).unwrap();
                assert!(request.contains("count"));
                let _ = stream.write_all(&reply);
            });
            assert_eq!(
                dispatch_action(
                    &socket,
                    "count",
                    &serde_json::Value::Null,
                    Duration::from_secs(1)
                ),
                DaemonActionDispatch::OutcomeUnknown
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn operation_transport_oversize_request_and_missing_listener_are_not_sent() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("daemon.sock");
        assert_eq!(
            dispatch_action(
                &socket,
                "count",
                &serde_json::Value::Null,
                Duration::from_millis(10)
            ),
            DaemonActionDispatch::NotSent
        );
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let input =
            serde_json::Value::String("x".repeat(qol_runtime::local_ipc::MAX_MESSAGE_BYTES));
        assert_eq!(
            dispatch_action(&socket, "count", &input, Duration::from_millis(10)),
            DaemonActionDispatch::NotSent
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
