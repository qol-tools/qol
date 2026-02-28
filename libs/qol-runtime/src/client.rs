use crate::protocol::{RuntimeEvent, RuntimeEventKind, RuntimeRequest, SubscribeAck};
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

pub struct Subscription {
    reader: BufReader<UnixStream>,
}

impl PlatformStateClient {
    pub fn from_env() -> Self {
        let path =
            std::env::var(ENV_STATE_SOCKET).unwrap_or_else(|_| DEFAULT_SOCKET.to_string());
        Self {
            socket_path: PathBuf::from(path),
        }
    }

    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub fn get_state(&self) -> Option<PlatformState> {
        let mut stream = UnixStream::connect(&self.socket_path).ok()?;
        stream.set_read_timeout(Some(TIMEOUT)).ok()?;
        stream.set_write_timeout(Some(TIMEOUT)).ok()?;

        let request = RuntimeRequest::GetState;
        let mut payload = serde_json::to_string(&request).ok()?;
        payload.push('\n');
        stream.write_all(payload.as_bytes()).ok()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;

        serde_json::from_str(&line).ok()
    }

    pub fn set_focus(&self, monitor_idx: usize) {
        let Ok(mut stream) = UnixStream::connect(&self.socket_path) else {
            return;
        };
        let _ = stream.set_write_timeout(Some(TIMEOUT));
        let request = RuntimeRequest::SetFocus { monitor_idx };
        let Ok(mut payload) = serde_json::to_string(&request) else {
            return;
        };
        payload.push('\n');
        let _ = stream.write_all(payload.as_bytes());
    }

    pub fn subscribe(&self, events: Vec<RuntimeEventKind>) -> Option<Subscription> {
        let mut stream = UnixStream::connect(&self.socket_path).ok()?;
        stream.set_write_timeout(Some(TIMEOUT)).ok()?;
        stream.set_read_timeout(None).ok()?;

        let request = RuntimeRequest::Subscribe { events };
        let mut payload = serde_json::to_string(&request).ok()?;
        payload.push('\n');
        stream.write_all(payload.as_bytes()).ok()?;

        let mut reader = BufReader::new(stream);
        let mut ack_line = String::new();
        reader.read_line(&mut ack_line).ok()?;

        let ack: SubscribeAck = serde_json::from_str(ack_line.trim()).ok()?;
        if !matches!(ack, SubscribeAck::Subscribed) {
            return None;
        }

        Some(Subscription { reader })
    }
}

impl Subscription {
    pub fn next_event(&mut self) -> Option<RuntimeEvent> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => serde_json::from_str(line.trim()).ok(),
        }
    }

    pub fn events(self) -> impl Iterator<Item = RuntimeEvent> {
        SubscriptionIter { sub: self }
    }
}

struct SubscriptionIter {
    sub: Subscription,
}

impl Iterator for SubscriptionIter {
    type Item = RuntimeEvent;

    fn next(&mut self) -> Option<RuntimeEvent> {
        self.sub.next_event()
    }
}
