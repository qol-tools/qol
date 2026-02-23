use crate::PlatformState;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const ENV_STATE_SOCKET: &str = "QOL_TRAY_STATE_SOCKET";
const DEFAULT_SOCKET: &str = "/tmp/qol-tray-state.sock";
const TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct PlatformStateClient {
    socket_path: PathBuf,
}

impl PlatformStateClient {
    pub fn from_env() -> Self {
        let path = std::env::var(ENV_STATE_SOCKET)
            .unwrap_or_else(|_| DEFAULT_SOCKET.to_string());
        Self {
            socket_path: PathBuf::from(path),
        }
    }

    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub fn get_state(&self) -> Option<PlatformState> {
        let stream = UnixStream::connect(&self.socket_path).ok()?;
        stream.set_read_timeout(Some(TIMEOUT)).ok()?;
        stream.set_write_timeout(Some(TIMEOUT)).ok()?;

        let mut stream = stream;
        stream.write_all(b"GET_STATE\n").ok()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;

        serde_json::from_str(&line).ok()
    }

    /// Tell the runtime which monitor has focus (by index into the monitors list).
    pub fn set_focus(&self, monitor_idx: usize) {
        let Ok(mut stream) = UnixStream::connect(&self.socket_path) else { return };
        let _ = stream.set_write_timeout(Some(TIMEOUT));
        let _ = write!(stream, "SET_FOCUS {}\n", monitor_idx);
    }
}
