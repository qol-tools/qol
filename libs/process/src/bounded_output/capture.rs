use super::CapturedOutput;
use std::io::{self, Read};

pub(super) const WAIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

pub(super) fn spawn_capture(
    stream: impl Read + Send + 'static,
    output_limit: usize,
    stream_name: &str,
) -> io::Result<std::thread::JoinHandle<io::Result<CapturedOutput>>> {
    std::thread::Builder::new()
        .name(format!("qol-process-{stream_name}-capture"))
        .spawn(move || capture_stream(stream, output_limit))
}

pub(super) fn capture_stream(
    mut stream: impl Read,
    output_limit: usize,
) -> io::Result<CapturedOutput> {
    let mut bytes = Vec::with_capacity(output_limit.min(8 * 1024));
    let mut truncated = false;
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        let retained = output_limit.saturating_sub(bytes.len()).min(read);
        bytes.extend_from_slice(&chunk[..retained]);
        truncated |= retained < read;
    }
    Ok(CapturedOutput { bytes, truncated })
}

pub(super) fn join_capture_thread(
    reader: std::thread::JoinHandle<io::Result<CapturedOutput>>,
    stream_name: &str,
) -> io::Result<CapturedOutput> {
    reader
        .join()
        .map_err(|_| io::Error::other(format!("{stream_name} reader panicked")))?
}

pub(super) fn combine_errors(primary: io::Error, context: &str, secondary: io::Error) -> io::Error {
    io::Error::new(primary.kind(), format!("{primary}; {context}: {secondary}"))
}
