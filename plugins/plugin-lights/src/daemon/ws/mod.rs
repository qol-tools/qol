use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use tungstenite::Message;

use crate::backend::zigbee::ZigbeeBackend;
use crate::domain::model::{LightCommand, LightTarget, RgbColor};
use crate::service::light_service::LightService;

mod platform;

const SEND_INTERVAL: Duration = Duration::from_millis(100);
const BIND_RETRY_WINDOW: Duration = Duration::from_secs(30);
const BIND_RETRY_INTERVAL: Duration = Duration::from_millis(500);

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
    target: LightTarget,
) {
    let ws_port: u16 = env!("QOL_DAEMON_PORT")
        .parse()
        .expect("QOL_DAEMON_PORT must be a valid u16");

    start_send_loop(buffer.clone(), service, target);

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
                        eprintln!("ws: failed to bind port {}: {}", ws_port, e);
                        return;
                    }
                };
            eprintln!("ws: listening on 127.0.0.1:{}", ws_port);
            accept_loop(listener, buffer)
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

fn accept_loop(listener: TcpListener, buffer: CommandBuffer) {
    for stream in listener.incoming().flatten() {
        let buf = buffer.clone();
        thread::Builder::new()
            .name("ws-client".into())
            .spawn(move || handle_client(stream, buf))
            .ok();
    }
}

fn handle_client(stream: std::net::TcpStream, buffer: CommandBuffer) {
    let mut ws = match tungstenite::accept(stream) {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("ws: handshake failed: {}", e);
            return;
        }
    };

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
    target: LightTarget,
) {
    thread::Builder::new()
        .name("ws-send".into())
        .spawn(move || loop {
            thread::sleep(SEND_INTERVAL);
            let Some(cmd) = buffer.lock().ok().and_then(|mut b| b.take()) else {
                continue;
            };
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

    fn assert_rgb(command: Option<PendingCommand>, red: u8, green: u8, blue: u8) {
        let Some(PendingCommand::Color(color)) = command else {
            panic!("expected color command");
        };
        assert_eq!((color.red, color.green, color.blue), (red, green, blue));
    }
}
