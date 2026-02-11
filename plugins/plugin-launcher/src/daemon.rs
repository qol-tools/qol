use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::Sender;

use crate::monitor::{ActiveMonitor, FocusCache};

const DEFAULT_SOCKET_PATH: &str = "/tmp/qol-launcher.sock";

pub enum Command {
    Show(Option<ActiveMonitor>),
    Kill,
}

enum ReadResult {
    Command(Command),
    Fallback,
    Error(&'static str),
    Ignore,
}

pub fn send_show() -> bool {
    send_raw(b"show")
}

pub fn send_kill() -> bool {
    send_raw(b"kill")
}

pub fn start_listener(tx: Sender<Command>, focus_cache: FocusCache) -> bool {
    if send_show() {
        return false;
    }

    let socket_path = socket_path();
    let _ = fs::remove_file(&socket_path);
    let Ok(listener) = UnixListener::bind(&socket_path) else {
        return false;
    };

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => match read_command(&mut stream, &focus_cache) {
                    ReadResult::Command(cmd) => {
                        let _ = stream.write_all(b"handled\n");
                        if tx.send(cmd).is_err() {
                            break;
                        }
                    }
                    ReadResult::Fallback => {
                        let _ = stream.write_all(b"fallback\n");
                    }
                    ReadResult::Error(message) => {
                        let _ = stream.write_all(format!("error {}\n", message).as_bytes());
                    }
                    ReadResult::Ignore => {}
                },
                Err(_) => break,
            }
        }
        let _ = fs::remove_file(&socket_path);
    });

    true
}

pub fn cleanup() {
    let _ = fs::remove_file(socket_path());
}

fn send_raw(msg: &[u8]) -> bool {
    let socket_path = socket_path();
    let Ok(mut stream) = UnixStream::connect(&socket_path) else {
        return false;
    };
    let timeout = std::time::Duration::from_millis(500);
    let _ = stream.set_write_timeout(Some(timeout));
    let _ = stream.set_read_timeout(Some(timeout));
    if stream.write_all(msg).is_err() {
        return false;
    }
    let mut buf = [0u8; 128];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 => match std::str::from_utf8(&buf[..n]) {
            Ok(response) => !response.trim().starts_with("error"),
            Err(_) => true,
        },
        Ok(_) => true,
        Err(_) => true,
    }
}

fn socket_path() -> std::path::PathBuf {
    std::env::var("QOL_TRAY_DAEMON_SOCKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(DEFAULT_SOCKET_PATH))
}

fn read_command(stream: &mut UnixStream, focus_cache: &FocusCache) -> ReadResult {
    let mut buf = [0u8; 128];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return ReadResult::Ignore,
    };
    if n == 0 {
        return ReadResult::Ignore;
    }

    let raw = match std::str::from_utf8(&buf[..n]) {
        Ok(value) => value.trim(),
        Err(_) => return ReadResult::Error("invalid utf8"),
    };
    let command = match raw.strip_prefix("action:") {
        Some(action_id) => {
            if !is_valid_action_id(action_id) {
                return ReadResult::Error("invalid action id");
            }
            action_id
        }
        None => raw,
    };

    match command {
        "show" | "open" => {
            let snap = focus_cache.snapshot();
            #[cfg(debug_assertions)]
            eprintln!(
                "[daemon] show snapshot: {:?}",
                snap.as_ref().map(|m| m.bounds())
            );
            ReadResult::Command(Command::Show(snap))
        }
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}

fn is_valid_action_id(action: &str) -> bool {
    !action.is_empty()
        && action.len() <= 64
        && !action.starts_with('-')
        && action
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
