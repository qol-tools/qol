pub(crate) mod frame;
pub(crate) mod request;

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use frame::{Decoder, Frame};
use request::{Request, Status};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub(crate) enum ClientEvent {
    Status(Status),
    Disconnected(String),
}

pub(crate) struct Connection {
    writer: Arc<Mutex<UnixStream>>,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

impl Connection {
    pub(crate) fn connect(path: &Path, events: Sender<ClientEvent>) -> io::Result<Self> {
        Self::from_stream(UnixStream::connect(path)?, events)
    }

    pub(crate) fn from_stream(stream: UnixStream, events: Sender<ClientEvent>) -> io::Result<Self> {
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let writer = Arc::new(Mutex::new(stream.try_clone()?));
        let closed = Arc::new(AtomicBool::new(false));
        {
            let writer = Arc::clone(&writer);
            let closed = Arc::clone(&closed);
            std::thread::Builder::new()
                .name("keyremap-pqrs-reader".into())
                .spawn(move || read_loop(stream, &writer, &events, &closed))?;
        }
        {
            let writer = Arc::clone(&writer);
            let closed = Arc::clone(&closed);
            std::thread::Builder::new()
                .name("keyremap-pqrs-heartbeat".into())
                .spawn(move || heartbeat_loop(&writer, &closed))?;
        }
        Ok(Self {
            writer,
            next_id: AtomicU64::new(1),
            closed,
        })
    }

    pub(crate) fn send(&self, request: &Request) -> io::Result<()> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        write_frame(
            &self.writer,
            &Frame::Request {
                id,
                payload: request.encode(),
            },
        )
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(stream) = self.writer.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

fn write_frame(writer: &Mutex<UnixStream>, frame: &Frame) -> io::Result<()> {
    let mut stream = writer
        .lock()
        .map_err(|_| io::Error::other("pqrs writer lock poisoned"))?;
    stream.write_all(&frame.encode())
}

fn heartbeat_loop(writer: &Mutex<UnixStream>, closed: &AtomicBool) {
    while !closed.load(Ordering::SeqCst) {
        std::thread::sleep(HEARTBEAT_INTERVAL);
        if closed.load(Ordering::SeqCst) || write_frame(writer, &Frame::Heartbeat).is_err() {
            return;
        }
    }
}

fn read_loop(
    mut stream: UnixStream,
    writer: &Mutex<UnixStream>,
    events: &Sender<ClientEvent>,
    closed: &AtomicBool,
) {
    let reason = read_frames(&mut stream, writer, events);
    closed.store(true, Ordering::SeqCst);
    let _ = events.send(ClientEvent::Disconnected(reason));
}

fn read_frames(
    stream: &mut UnixStream,
    writer: &Mutex<UnixStream>,
    events: &Sender<ClientEvent>,
) -> String {
    let mut decoder = Decoder::default();
    let mut buffer = [0u8; 1024];
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) => return "the pqrs daemon closed the connection".to_string(),
            Ok(read) => read,
            Err(error) => return format!("reading from the pqrs daemon failed: {error}"),
        };
        decoder.push(&buffer[..read]);
        loop {
            match decoder.next_frame() {
                Ok(None) => break,
                Ok(Some(frame)) => {
                    if let Err(error) = handle_frame(frame, writer, events) {
                        return format!("writing to the pqrs daemon failed: {error}");
                    }
                }
                Err(error) => return format!("the pqrs daemon sent a bad frame: {error}"),
            }
        }
    }
}

fn handle_frame(
    frame: Frame,
    writer: &Mutex<UnixStream>,
    events: &Sender<ClientEvent>,
) -> io::Result<()> {
    match frame {
        Frame::HealthCheck => write_frame(writer, &Frame::HealthCheckResponse),
        Frame::Request { id, payload } => {
            forward_status(&payload, events);
            write_frame(
                writer,
                &Frame::Response {
                    id,
                    payload: Vec::new(),
                },
            )
        }
        Frame::Response { payload, .. } => {
            forward_status(&payload, events);
            Ok(())
        }
        Frame::Heartbeat | Frame::HealthCheckResponse | Frame::UserData(_) => Ok(()),
    }
}

fn forward_status(payload: &[u8], events: &Sender<ClientEvent>) {
    match request::parse_status(payload) {
        Ok(statuses) => {
            for status in statuses {
                let _ = events.send(ClientEvent::Status(status));
            }
        }
        Err(error) => log::warn!("ignoring a pqrs status message: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    fn read_frame(stream: &mut UnixStream, decoder: &mut Decoder) -> Frame {
        let mut buffer = [0u8; 256];
        loop {
            if let Some(frame) = decoder.next_frame().unwrap() {
                return frame;
            }
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0, "connection closed before a frame arrived");
            decoder.push(&buffer[..read]);
        }
    }

    fn pair() -> (Connection, UnixStream, mpsc::Receiver<ClientEvent>) {
        let (client, server) = UnixStream::pair().unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (sender, events) = mpsc::channel();
        let connection = Connection::from_stream(client, sender).unwrap();
        (connection, server, events)
    }

    #[test]
    fn a_pushed_status_request_is_reported_and_acknowledged() {
        let (_connection, mut server, events) = pair();
        server
            .write_all(
                &Frame::Request {
                    id: 41,
                    payload: vec![4, 1],
                }
                .encode(),
            )
            .unwrap();

        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            event,
            ClientEvent::Status(Status::KeyboardReady(true))
        ));
        let mut decoder = Decoder::default();
        assert_eq!(
            read_frame(&mut server, &mut decoder),
            Frame::Response {
                id: 41,
                payload: Vec::new()
            }
        );
    }

    #[test]
    fn health_checks_are_answered() {
        let (_connection, mut server, _events) = pair();
        server.write_all(&Frame::HealthCheck.encode()).unwrap();
        let mut decoder = Decoder::default();
        assert_eq!(
            read_frame(&mut server, &mut decoder),
            Frame::HealthCheckResponse
        );
    }

    #[test]
    fn requests_go_out_as_request_frames_with_rising_ids() {
        let (connection, mut server, _events) = pair();
        connection.send(&Request::KeyboardReset).unwrap();
        connection.send(&Request::KeyboardReset).unwrap();
        let mut decoder = Decoder::default();
        let Frame::Request { id: first, payload } = read_frame(&mut server, &mut decoder) else {
            panic!("expected a request frame");
        };
        assert_eq!(payload, vec![7, 0, 2]);
        let Frame::Request { id: second, .. } = read_frame(&mut server, &mut decoder) else {
            panic!("expected a request frame");
        };
        assert!(second > first);
    }

    #[test]
    fn the_daemon_closing_the_socket_is_reported() {
        let (_connection, server, events) = pair();
        drop(server);
        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(event, ClientEvent::Disconnected(_)));
    }
}
