use qol_runtime::local_ipc::LocalStream;
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::path::Path;
use std::time::{Duration, Instant};

mod platform;

type DispatchResult<T> = Result<T, ()>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonActionDispatch {
    Handled {
        payload: Option<serde_json::Value>,
    },
    Fallback,
    Error(String),
    NotSent,
    OutcomeUnknown,
    NotReady {
        phase: qol_runtime::protocol::ReadinessPhase,
        detail: Option<String>,
    },
}

pub fn dispatch_daemon_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    dispatch_daemon_action_with_timeout(endpoint, action_id, platform::default_io_timeout())
}

pub fn dispatch_daemon_action_with_input(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
) -> DaemonActionDispatch {
    dispatch_daemon_action_request(endpoint, action_id, input, platform::default_io_timeout())
}

pub fn dispatch_daemon_action_with_timeout(
    endpoint: &Path,
    action_id: &str,
    timeout: Duration,
) -> DaemonActionDispatch {
    dispatch_daemon_action_request(endpoint, action_id, &serde_json::Value::Null, timeout)
}

pub fn dispatch_daemon_action_with_input_and_timeout(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    dispatch_daemon_action_request(endpoint, action_id, input, timeout)
}

pub fn dispatch_daemon_theme(endpoint: &Path, native: &str, accent: &str) -> DaemonActionDispatch {
    let line = format!("theme {native} {accent}");
    dispatch_action(
        endpoint,
        &line,
        &serde_json::Value::Null,
        platform::default_io_timeout(),
    )
}

fn dispatch_daemon_action_request(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    if !crate::plugins::manifest::is_valid_action_id(action_id) {
        return DaemonActionDispatch::NotSent;
    }
    dispatch_action(endpoint, action_id, input, timeout)
}

pub fn daemon_listener_reachable(endpoint: &Path) -> bool {
    connect_stream(endpoint, platform::default_io_timeout()).is_ok()
}

#[cfg(test)]
pub(crate) fn default_io_timeout() -> Duration {
    platform::default_io_timeout()
}

pub(crate) fn probe_operation_instance(endpoint: &Path, instance: &str, timeout: Duration) -> bool {
    let DaemonActionDispatch::Handled {
        payload: Some(payload),
    } = dispatch_daemon_action_with_timeout(endpoint, "ping", timeout)
    else {
        return false;
    };
    serde_json::from_value::<qol_runtime::protocol::DaemonOperationCapability>(payload)
        .is_ok_and(|capability| capability.fence_version == 1 && capability.instance == instance)
}

pub(crate) fn dispatch_prepared(
    endpoint: &Path,
    payload: &[u8],
    timeout: Duration,
) -> DaemonActionDispatch {
    dispatch_payload(endpoint, payload, timeout)
}

fn dispatch_action(
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

fn dispatch_payload(endpoint: &Path, payload: &[u8], timeout: Duration) -> DaemonActionDispatch {
    if payload.len() > qol_runtime::local_ipc::MAX_MESSAGE_BYTES {
        return DaemonActionDispatch::NotSent;
    }
    let deadline = Instant::now() + timeout;
    platform::before_forward();
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

fn connect_stream(endpoint: &Path, timeout: Duration) -> DispatchResult<LocalStream> {
    let stream = platform::connect(endpoint, timeout)?;
    qol_runtime::local_ipc::authorize_peer(&stream).map_err(|_| ())?;
    apply_timeout(&stream, timeout);
    Ok(stream)
}

fn apply_timeout(stream: &LocalStream, timeout: Duration) {
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
    stream: &mut LocalStream,
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

fn read_response(mut stream: LocalStream, deadline: Instant) -> DispatchResult<String> {
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
    use qol_runtime::local_ipc::LocalListener;

    #[test]
    fn dispatch_respects_short_read_timeout_when_listener_does_not_answer() {
        let dir = tempfile::TempDir::new().unwrap();
        let socket_path = dir.path().join("hung-daemon.sock");
        let _listener = LocalListener::bind(&socket_path).unwrap();
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
            let listener = LocalListener::bind(&socket).unwrap();
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
        let listener = LocalListener::bind(&socket).unwrap();
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
