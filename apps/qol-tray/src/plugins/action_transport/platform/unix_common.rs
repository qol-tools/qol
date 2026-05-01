use super::super::DaemonActionDispatch;
use qol_runtime::protocol::DaemonRequest;
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

// 10s: ZCL commands need up to 5s for DATA_CONFIRM + 3s for NWK_ADDR resolution.
// The original 80ms caused every action to return "daemon unavailable".
const SOCKET_IO_TIMEOUT_MS: u64 = 10_000;
type DispatchResult<T> = Result<T, ()>;

pub(super) fn dispatch_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    let Ok(stream) = connect_stream(endpoint) else {
        return DaemonActionDispatch::Unavailable;
    };
    let Ok(stream) = send_request(stream, action_id) else {
        return DaemonActionDispatch::Unavailable;
    };
    let Ok(line) = read_response(stream) else {
        return DaemonActionDispatch::Unavailable;
    };
    super::super::protocol::parse_response(line.trim())
}

fn connect_stream(endpoint: &Path) -> DispatchResult<UnixStream> {
    let stream = UnixStream::connect(endpoint).map_err(|_| ())?;
    apply_timeout(&stream);
    Ok(stream)
}

fn apply_timeout(stream: &UnixStream) {
    let timeout = Duration::from_millis(SOCKET_IO_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.set_read_timeout(Some(timeout));
}

fn send_request(mut stream: UnixStream, action_id: &str) -> DispatchResult<UnixStream> {
    let payload = request_payload(action_id)?;
    stream.write_all(payload.as_bytes()).map_err(|_| ())?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(stream)
}

fn request_payload(action_id: &str) -> DispatchResult<String> {
    let request = DaemonRequest {
        action: action_id.to_string(),
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
