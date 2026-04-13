use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use tungstenite::Message;

use crate::backend::zigbee::ZigbeeBackend;
use crate::domain::model::{LightCommand, LightTarget, RgbColor};
use crate::service::light_service::LightService;

const WS_PORT: u16 = 42710;
const SEND_INTERVAL: Duration = Duration::from_millis(100);

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
    let listener = match TcpListener::bind(format!("127.0.0.1:{}", WS_PORT)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ws: failed to bind port {}: {}", WS_PORT, e);
            return;
        }
    };
    eprintln!("ws: listening on 127.0.0.1:{}", WS_PORT);

    start_send_loop(buffer.clone(), service, target);

    thread::Builder::new()
        .name("ws-accept".into())
        .spawn(move || accept_loop(listener, buffer))
        .ok();
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
        "color" => Some(PendingCommand::Color(parse_hex(&cmd.hex))),
        "brightness" => Some(PendingCommand::Brightness(cmd.level, parse_hex(&cmd.hex))),
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
            let Ok(mut guard) = service.lock() else { continue };
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

fn parse_hex(hex: &str) -> RgbColor {
    let r = u8::from_str_radix(hex.get(0..2).unwrap_or("ff"), 16).unwrap_or(255);
    let g = u8::from_str_radix(hex.get(2..4).unwrap_or("ff"), 16).unwrap_or(255);
    let b = u8::from_str_radix(hex.get(4..6).unwrap_or("ff"), 16).unwrap_or(255);
    RgbColor {
        red: r,
        green: g,
        blue: b,
    }
}
