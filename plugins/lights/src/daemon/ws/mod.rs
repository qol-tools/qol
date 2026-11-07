use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::Message;

use super::state::{current_target, SharedTarget};
use crate::backend::zigbee::ZigbeeBackend;
use crate::domain::model::{LightCommand, LightTarget, RgbColor};
use crate::service::light_service::LightService;

mod platform;

const SEND_INTERVAL: Duration = Duration::from_millis(100);
const BIND_RETRY_WINDOW: Duration = Duration::from_secs(30);
const BIND_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CLIENTS: usize = 4;

#[derive(Debug, Deserialize)]
struct WsCommand {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    hex: String,
    #[serde(default)]
    level: u8,
}

#[derive(Debug, Clone)]
pub enum PendingCommand {
    Color(RgbColor),
    Brightness(u8, RgbColor),
}

pub type CommandBuffer = Arc<Mutex<Option<PendingCommand>>>;

pub fn start(
    buffer: CommandBuffer,
    service: Arc<Mutex<Option<LightService<ZigbeeBackend>>>>,
    target: SharedTarget,
) {
    let ws_port: u16 = env!("QOL_DAEMON_PORT")
        .parse()
        .expect("QOL_DAEMON_PORT must be a valid u16");

    start_send_loop(buffer.clone(), service, target);

    let expected: Arc<Option<Vec<u8>>> = Arc::new(
        std::env::var(qol_conventions::ENV_HTTP_TOKEN)
            .ok()
            .filter(|token| !token.is_empty())
            .and_then(|token| qol_runtime::local_http::stream_token(&token, ws_port))
            .map(String::into_bytes),
    );
    if expected.is_none() {
        log::warn!("ws: no tray token in the environment; every live color client is refused");
    }

    thread::Builder::new()
        .name("ws-accept".into())
        .spawn(move || {
            // The port may still be held by the predecessor generation during
            // a qol dev handoff; a one-shot bind here left the websocket dead
            // for the daemon's whole lifetime. Retrying off the startup path
            // rides out the overlap window without delaying the daemon.
            let listener =
                match bind_ws_listener_retrying(ws_port, BIND_RETRY_WINDOW, BIND_RETRY_INTERVAL) {
                    Ok(l) => l,
                    Err(e) => {
                        log::warn!("ws: failed to bind port {}: {}", ws_port, e);
                        return;
                    }
                };
            log::info!("ws: listening on 127.0.0.1:{}", ws_port);
            accept_loop(listener, buffer, expected)
        })
        .ok();
}

fn bind_ws_listener_retrying(
    port: u16,
    window: Duration,
    interval: Duration,
) -> std::io::Result<TcpListener> {
    let deadline = std::time::Instant::now() + window;
    loop {
        match bind_ws_listener(port) {
            Err(e)
                if e.kind() == std::io::ErrorKind::AddrInUse
                    && std::time::Instant::now() < deadline =>
            {
                thread::sleep(interval)
            }
            result => return result,
        }
    }
}

fn bind_ws_listener(port: u16) -> std::io::Result<TcpListener> {
    platform::bind_listener(port)
}

fn accept_loop(listener: TcpListener, buffer: CommandBuffer, expected: Arc<Option<Vec<u8>>>) {
    let live = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming().flatten() {
        let Some(slot) = ClientSlot::take(&live) else {
            log::debug!("ws: client cap reached");
            continue;
        };
        let buf = buffer.clone();
        let expected = expected.clone();
        thread::Builder::new()
            .name("ws-client".into())
            .spawn(move || handle_client(stream, buf, expected, slot))
            .ok();
    }
}

struct HandshakeStream {
    inner: TcpStream,
    deadline: Option<Instant>,
}

impl HandshakeStream {
    fn new(inner: TcpStream, deadline: Instant) -> Self {
        Self {
            inner,
            deadline: Some(deadline),
        }
    }

    fn settle(&mut self) {
        self.deadline = None;
        let _ = self.inner.set_read_timeout(None);
        let _ = self.inner.set_write_timeout(None);
    }

    fn remaining(&self) -> std::io::Result<Option<Duration>> {
        let Some(deadline) = self.deadline else {
            return Ok(None);
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::from(std::io::ErrorKind::TimedOut));
        }
        Ok(Some(remaining))
    }
}

impl Read for HandshakeStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(remaining) = self.remaining()? {
            self.inner.set_read_timeout(Some(remaining))?;
        }
        self.inner.read(buf)
    }
}

impl Write for HandshakeStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(remaining) = self.remaining()? {
            self.inner.set_write_timeout(Some(remaining))?;
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct ClientSlot(Arc<AtomicUsize>);

impl ClientSlot {
    fn take(live: &Arc<AtomicUsize>) -> Option<Self> {
        if live.fetch_add(1, Ordering::SeqCst) >= MAX_CLIENTS {
            live.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self(live.clone()))
    }
}

impl Drop for ClientSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn handle_client(
    stream: TcpStream,
    buffer: CommandBuffer,
    expected: Arc<Option<Vec<u8>>>,
    _slot: ClientSlot,
) {
    let stream = HandshakeStream::new(stream, Instant::now() + HANDSHAKE_TIMEOUT);
    let Some(mut ws) = accept_client(stream, expected.as_deref()) else {
        return;
    };
    ws.get_mut().settle();

    loop {
        let msg = match ws.read() {
            Ok(msg) => msg,
            Err(_) => break,
        };

        if msg.is_close() {
            break;
        }

        let Message::Text(ref text) = msg else {
            continue;
        };
        let Ok(cmd) = serde_json::from_str::<WsCommand>(text) else {
            continue;
        };
        let Some(pending) = parse_pending(&cmd) else {
            continue;
        };
        if let Ok(mut buf) = buffer.lock() {
            *buf = Some(pending);
        }
    }
}

#[allow(clippy::result_large_err)]
fn accept_client(
    stream: HandshakeStream,
    expected: Option<&[u8]>,
) -> Option<tungstenite::WebSocket<HandshakeStream>> {
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let refuse = |status: StatusCode| {
            let mut refused = ErrorResponse::new(None);
            *refused.status_mut() = status;
            refused
        };
        if request
            .headers()
            .contains_key(tungstenite::http::header::ORIGIN)
        {
            return Err(refuse(StatusCode::FORBIDDEN));
        }
        let presented = request.headers().get(qol_conventions::HTTP_AUTH_HEADER);
        let authorised = match (expected, presented) {
            (Some(expected), Some(presented)) => same_token(presented.as_bytes(), expected),
            _ => false,
        };
        if !authorised {
            return Err(refuse(StatusCode::UNAUTHORIZED));
        }
        Ok(response)
    };
    match tungstenite::accept_hdr(stream, check) {
        Ok(ws) => Some(ws),
        Err(error) => {
            log::warn!("ws: handshake refused: {error}");
            None
        }
    }
}

fn same_token(presented: &[u8], expected: &[u8]) -> bool {
    presented.len() == expected.len()
        && presented
            .iter()
            .zip(expected)
            .fold(0u8, |diff, (left, right)| diff | (left ^ right))
            == 0
}

fn parse_pending(cmd: &WsCommand) -> Option<PendingCommand> {
    match cmd.kind.as_str() {
        "color" => Some(PendingCommand::Color(parse_hex(&cmd.hex)?)),
        "brightness" => Some(PendingCommand::Brightness(cmd.level, parse_hex(&cmd.hex)?)),
        _ => None,
    }
}

fn start_send_loop(
    buffer: CommandBuffer,
    service: Arc<Mutex<Option<LightService<ZigbeeBackend>>>>,
    target: SharedTarget,
) {
    thread::Builder::new()
        .name("ws-send".into())
        .spawn(move || loop {
            thread::sleep(SEND_INTERVAL);
            let Some(cmd) = buffer.lock().ok().and_then(|mut b| b.take()) else {
                continue;
            };
            let target = current_target(&target);
            let Ok(mut guard) = service.lock() else {
                continue;
            };
            if let Some(svc) = guard.as_mut() {
                dispatch(svc, &target, cmd);
            }
        })
        .ok();
}

fn dispatch(svc: &mut LightService<ZigbeeBackend>, target: &LightTarget, cmd: PendingCommand) {
    match cmd {
        PendingCommand::Color(color) => {
            let _ = svc.apply_command(target, &LightCommand::SetColor { color });
        }
        PendingCommand::Brightness(level, color) => {
            let _ = svc.apply_command(target, &LightCommand::SetBrightness { level });
            let _ = svc.apply_command(target, &LightCommand::SetColor { color });
        }
    }
}

fn parse_hex(hex: &str) -> Option<RgbColor> {
    let (red, green, blue) = qol_color::parse_hex_color(hex)?;
    Some(RgbColor { red, green, blue })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression tests for the generation-handoff window: the predecessor
    // qol-tray generation still holds the ws port when this daemon starts,
    // and a one-shot bind left the websocket dead for the daemon's lifetime.
    #[test]
    fn ws_bind_retries_until_the_holder_releases_the_port() {
        let holder = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(holder);
        });

        let bound =
            bind_ws_listener_retrying(port, Duration::from_secs(5), Duration::from_millis(25));

        release.join().unwrap();
        assert!(
            bound.is_ok(),
            "the bind must succeed once the previous holder exits"
        );
    }

    #[test]
    fn ws_bind_gives_up_after_the_retry_window() {
        let holder = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();

        let bound =
            bind_ws_listener_retrying(port, Duration::from_millis(100), Duration::from_millis(25));

        assert_eq!(
            bound.unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse,
            "a port that never frees must surface the original error, not spin forever"
        );
    }

    #[test]
    fn parse_pending_accepts_hash_and_plain_rgb() {
        let plain = parse_pending(&WsCommand {
            kind: "color".to_string(),
            hex: "203040".to_string(),
            level: 0,
        });
        let hashed = parse_pending(&WsCommand {
            kind: "brightness".to_string(),
            hex: "#203040".to_string(),
            level: 42,
        });

        assert_rgb(plain, 0x20, 0x30, 0x40);
        let Some(PendingCommand::Brightness(42, color)) = hashed else {
            panic!("expected brightness command");
        };
        assert_eq!((color.red, color.green, color.blue), (0x20, 0x30, 0x40));
    }

    #[test]
    fn parse_pending_rejects_malformed_live_colors() {
        for hex in ["", "#123", "#12345678", "gggggg"] {
            assert!(
                parse_pending(&WsCommand {
                    kind: "color".to_string(),
                    hex: hex.to_string(),
                    level: 0,
                })
                .is_none(),
                "hex: {hex}"
            );
            assert!(
                parse_pending(&WsCommand {
                    kind: "brightness".to_string(),
                    hex: hex.to_string(),
                    level: 42,
                })
                .is_none(),
                "hex: {hex}"
            );
        }
    }

    fn serve_one(expected: Option<&'static str>) -> (u16, std::thread::JoinHandle<bool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let stream = HandshakeStream::new(stream, Instant::now() + HANDSHAKE_TIMEOUT);
            accept_client(stream, expected.map(str::as_bytes)).is_some()
        });
        (port, server)
    }

    fn client_request(port: u16) -> tungstenite::client::ClientRequestBuilder {
        tungstenite::client::ClientRequestBuilder::new(
            format!("ws://127.0.0.1:{port}").parse().unwrap(),
        )
    }

    fn refused_status<T>(result: Result<T, tungstenite::Error>) -> StatusCode {
        match result {
            Err(tungstenite::Error::Http(response)) => response.status(),
            Err(other) => panic!("expected an http refusal, got {other}"),
            Ok(_) => panic!("expected a refusal, the handshake was accepted"),
        }
    }

    #[test]
    fn a_handshake_with_the_token_and_no_origin_is_accepted() {
        let (port, server) = serve_one(Some("secret"));

        let connected = tungstenite::connect(
            client_request(port).with_header(qol_conventions::HTTP_AUTH_HEADER, "secret"),
        );

        assert!(connected.is_ok());
        assert!(server.join().unwrap());
    }

    #[test]
    fn a_handshake_with_an_origin_header_is_refused() {
        let (port, server) = serve_one(Some("secret"));

        let connected = tungstenite::connect(
            client_request(port)
                .with_header(qol_conventions::HTTP_AUTH_HEADER, "secret")
                .with_header("Origin", "http://page.example"),
        );

        assert_eq!(refused_status(connected), StatusCode::FORBIDDEN);
        assert!(!server.join().unwrap());
    }

    #[test]
    fn a_handshake_without_the_token_is_refused() {
        let (port, server) = serve_one(Some("secret"));

        let connected = tungstenite::connect(client_request(port));

        assert_eq!(refused_status(connected), StatusCode::UNAUTHORIZED);
        assert!(!server.join().unwrap());
    }

    #[test]
    fn a_handshake_with_a_wrong_token_is_refused() {
        let (port, server) = serve_one(Some("secret"));

        let connected = tungstenite::connect(
            client_request(port).with_header(qol_conventions::HTTP_AUTH_HEADER, "wrong!"),
        );

        assert_eq!(refused_status(connected), StatusCode::UNAUTHORIZED);
        assert!(!server.join().unwrap());
    }

    #[test]
    fn a_daemon_without_a_token_refuses_every_handshake() {
        let (port, server) = serve_one(None);

        let connected = tungstenite::connect(
            client_request(port).with_header(qol_conventions::HTTP_AUTH_HEADER, "secret"),
        );

        assert_eq!(refused_status(connected), StatusCode::UNAUTHORIZED);
        assert!(!server.join().unwrap());
    }

    #[test]
    fn a_stalled_handshake_ends_at_the_total_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (done, server_done) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let started = Instant::now();
            let stream = HandshakeStream::new(stream, started + Duration::from_millis(300));
            let result = accept_client(stream, Some(b"secret"));
            let _ = done.send(());
            (result.is_none(), started.elapsed())
        });

        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.write_all(b"GET / HTTP/1.1\r\n").unwrap();
        for _ in 0..3 {
            thread::sleep(Duration::from_millis(120));
            let _ = client.write_all(b"X-Slow: 1\r\n");
        }
        let _ = server_done.recv_timeout(Duration::from_secs(10));
        drop(client);

        let (refused, elapsed) = server.join().unwrap();
        assert!(refused);
        assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    #[test]
    fn the_client_cap_frees_a_slot_when_a_client_leaves() {
        let live = Arc::new(AtomicUsize::new(0));
        let mut slots: Vec<ClientSlot> = (0..MAX_CLIENTS)
            .map(|_| ClientSlot::take(&live).expect("a slot below the cap"))
            .collect();

        assert!(ClientSlot::take(&live).is_none());
        slots.pop();

        assert!(ClientSlot::take(&live).is_some());
    }

    #[test]
    fn same_token_compares_whole_values() {
        assert!(same_token(b"secret", b"secret"));
        assert!(!same_token(b"secret", b"secre"));
        assert!(!same_token(b"secret", b"secreT"));
    }

    fn assert_rgb(command: Option<PendingCommand>, red: u8, green: u8, blue: u8) {
        let Some(PendingCommand::Color(color)) = command else {
            panic!("expected color command");
        };
        assert_eq!((color.red, color.green, color.blue), (red, green, blue));
    }
}
