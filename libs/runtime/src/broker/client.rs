//! Plugin-author client for the broker event bus.
//!
//! One call to publish, one call to subscribe; no async setup. The
//! client connects to the per-uid broker socket
//! (`broker::broker_socket_path`), which carries the same peer-cred
//! authorization as the pane-field pull API. Subscribe opens a held
//! connection and runs a reader thread that invokes the handler for
//! every delivered `BrokerEvent`; dropping the returned
//! `EventSubscription` shuts the connection down and stops the reader.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::broker::path::broker_socket_path;
use crate::broker::protocol::{BrokerEvent, BrokerRequest, SubscribeAck};
use crate::client::platform::{connect, Connection};

const TIMEOUT: Duration = Duration::from_millis(50);

/// Client for the broker's inter-plugin event bus.
#[derive(Clone)]
pub struct EventBusClient {
    socket_path: PathBuf,
}

impl EventBusClient {
    /// Resolve the broker socket for the current uid from the
    /// environment ($XDG_RUNTIME_DIR or the `/tmp` fallback). Returns
    /// `None` on platforms without broker sockets.
    pub fn from_env() -> Option<Self> {
        broker_socket_path()
            .ok()
            .map(|socket_path| Self { socket_path })
    }

    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Fire-and-forget publish of `payload` to `topic`.
    ///
    /// Returns `true` when the message was written to the broker
    /// socket; delivery to subscribers is the broker's job and is not
    /// acknowledged.
    pub fn publish(&self, topic: &str, payload: &serde_json::Value) -> bool {
        let Ok(stream) = connect(&self.socket_path) else {
            eprintln!(
                "[runtime/broker] publish to {topic:?} dropped: no broker listening on {}",
                self.socket_path.display()
            );
            return false;
        };
        if stream.set_write_timeout(Some(TIMEOUT)).is_err() {
            return false;
        }
        let request = BrokerRequest::Publish {
            topic: topic.to_string(),
            payload: payload.clone(),
        };
        let Ok(mut wire) = serde_json::to_string(&request) else {
            return false;
        };
        wire.push('\n');
        let mut stream = stream;
        stream.write_all(wire.as_bytes()).is_ok()
    }

    /// Subscribe to `topic`, invoking `handler` for every delivered
    /// event.
    ///
    /// Returns `None` when the broker is unreachable or refuses the
    /// subscription (topic not in the plugin's declared set). The
    /// handler runs on the subscription's broker reader thread - keep
    /// it cheap. Dropping the returned `EventSubscription` shuts the
    /// connection down and stops the reader.
    pub fn subscribe(
        &self,
        topic: &str,
        handler: impl Fn(String, serde_json::Value) + Send + 'static,
    ) -> Option<EventSubscription> {
        let mut stream = connect(&self.socket_path)
            .map_err(|error| {
                eprintln!(
                    "[runtime/broker] subscribe to {topic:?} failed: no broker listening on {} ({error})",
                    self.socket_path.display()
                );
            })
            .ok()?;
        stream.set_write_timeout(Some(TIMEOUT)).ok()?;
        stream.set_read_timeout(Some(TIMEOUT)).ok()?;

        let request = BrokerRequest::Subscribe {
            topic: topic.to_string(),
        };
        let mut wire = serde_json::to_string(&request).ok()?;
        wire.push('\n');

        let mut reader = BufReader::new(stream.try_clone().ok()?);
        stream.write_all(wire.as_bytes()).ok()?;
        let mut ack_line = String::new();
        reader.read_line(&mut ack_line).ok()?;
        match serde_json::from_str::<SubscribeAck>(ack_line.trim()).ok()? {
            SubscribeAck::Subscribed => {}
            SubscribeAck::Error { .. } => return None,
        }

        // Events block on read; only the ack read above had a timeout.
        reader.get_ref().set_read_timeout(None).ok()?;
        let join = std::thread::spawn(move || reader_loop(reader, handler));
        Some(EventSubscription {
            stream,
            join: Some(join),
        })
    }
}

/// Handle for one `EventBusClient::subscribe` call. Drop it to stop
/// the subscription's reader thread.
#[must_use = "dropping the subscription stops it"]
pub struct EventSubscription {
    stream: Box<dyn Connection>,
    join: Option<JoinHandle<()>>,
}

impl Drop for EventSubscription {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn reader_loop(
    mut reader: BufReader<Box<dyn Connection>>,
    handler: impl Fn(String, serde_json::Value),
) {
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if let Ok(event) = serde_json::from_str::<BrokerEvent>(line.trim()) {
                    handler(event.topic, event.payload);
                }
            }
        }
    }
}
