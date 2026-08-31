use crate::plugins::action_transport::DaemonActionDispatch;
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub(super) const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(10);
type DispatchResult<T> = Result<T, ()>;

pub(super) fn dispatch_action(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    let Ok(stream) = connect_stream(endpoint, timeout) else {
        return DaemonActionDispatch::Unavailable;
    };
    let Ok(stream) = send_request(stream, action_id, input) else {
        return DaemonActionDispatch::Unavailable;
    };
    let Ok(line) = read_response(stream) else {
        return DaemonActionDispatch::Unavailable;
    };
    parse_response(line.trim())
}

pub(super) fn can_connect(endpoint: &Path) -> bool {
    connect_stream(endpoint, DEFAULT_IO_TIMEOUT).is_ok()
}

fn connect_stream(endpoint: &Path, timeout: Duration) -> DispatchResult<UnixStream> {
    let stream = UnixStream::connect(endpoint).map_err(|_| ())?;
    qol_runtime::local_ipc::authorize_peer(&stream).map_err(|_| ())?;
    apply_timeout(&stream, timeout);
    Ok(stream)
}

fn apply_timeout(stream: &UnixStream, timeout: Duration) {
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.set_read_timeout(Some(timeout));
}

fn send_request(
    mut stream: UnixStream,
    action_id: &str,
    input: &serde_json::Value,
) -> DispatchResult<UnixStream> {
    let payload = request_payload(action_id, input)?;
    stream.write_all(payload.as_bytes()).map_err(|_| ())?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(stream)
}

fn request_payload(action_id: &str, input: &serde_json::Value) -> DispatchResult<String> {
    let request = DaemonRequest {
        action: action_id.to_string(),
        input: input.clone(),
    };
    let mut payload = serde_json::to_string(&request).map_err(|_| ())?;
    payload.push('\n');
    Ok(payload)
}

fn read_response(stream: UnixStream) -> DispatchResult<String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    read_line(&mut reader, &mut line)?;
    Ok(line)
}

fn read_line(reader: &mut BufReader<UnixStream>, line: &mut String) -> DispatchResult<()> {
    match reader.read_line(line) {
        Ok(0) | Err(_) => Err(()),
        Ok(_) => Ok(()),
    }
}

fn parse_response(line: &str) -> DaemonActionDispatch {
    if let Ok(response) = serde_json::from_str::<DaemonResponse>(line) {
        return match response {
            DaemonResponse::Handled { data } => DaemonActionDispatch::Handled { payload: data },
            DaemonResponse::Fallback => DaemonActionDispatch::Fallback,
            DaemonResponse::Error { message } => DaemonActionDispatch::Error(message),
            DaemonResponse::NotReady { .. } => DaemonActionDispatch::Unavailable,
        };
    }

    let word = line.split_whitespace().next().unwrap_or("");
    match word {
        "handled" => DaemonActionDispatch::Handled { payload: None },
        "fallback" => DaemonActionDispatch::Fallback,
        "error" => {
            DaemonActionDispatch::Error(line.strip_prefix("error").unwrap_or("").trim().to_string())
        }
        _ => DaemonActionDispatch::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::Instant;

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

        assert!(matches!(dispatch, DaemonActionDispatch::Unavailable));
        assert!(
            started.elapsed() < Duration::from_secs(1),
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
            ("", DaemonActionDispatch::Unavailable),
            ("garbage", DaemonActionDispatch::Unavailable),
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
}
