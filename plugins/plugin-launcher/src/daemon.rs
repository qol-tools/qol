use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::Sender;

const DEFAULT_SOCKET_PATH: &str = "/tmp/qol-launcher.sock";
const ACK_TIMEOUT_MS: u64 = 80;

pub enum Command {
    Show,
    Kill,
}

enum ReadResult {
    Command(Command),
    Handled,
    Fallback,
    Error(&'static str),
    Ignore,
}

pub fn send_show() -> bool {
    send_raw(b"show", false)
}

pub fn send_kill() -> bool {
    send_raw(b"kill", true)
}

fn send_ping() -> bool {
    send_raw(b"ping", true)
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    let socket_path = socket_path();
    #[cfg(debug_assertions)]
    eprintln!("[daemon] binding to {:?}", socket_path);
    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => {
            #[cfg(debug_assertions)]
            eprintln!("[daemon] bound successfully");
            listener
        }
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            #[cfg(debug_assertions)]
            eprintln!("[daemon] socket in use, pinging existing");
            if send_ping() {
                #[cfg(debug_assertions)]
                eprintln!("[daemon] existing instance alive, exiting");
                return false;
            }
            #[cfg(debug_assertions)]
            eprintln!("[daemon] no response, removing stale socket");
            remove_socket_file(&socket_path);
            let Ok(listener) = UnixListener::bind(&socket_path) else {
                #[cfg(debug_assertions)]
                eprintln!("[daemon] rebind failed");
                return false;
            };
            listener
        }
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("[daemon] bind error: {}", e);
            return false;
        }
    };

    std::thread::spawn(move || {
        #[cfg(debug_assertions)]
        eprintln!("[daemon] listener thread started");
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => match read_command(&mut stream) {
                    ReadResult::Command(cmd) => {
                        #[cfg(debug_assertions)]
                        eprintln!("[daemon] received command: {}", match &cmd { Command::Show => "show", Command::Kill => "kill" });
                        let _ = stream.write_all(b"handled\n");
                        if tx.send(cmd).is_err() {
                            #[cfg(debug_assertions)]
                            eprintln!("[daemon] channel closed, exiting listener");
                            break;
                        }
                    }
                    ReadResult::Handled => {
                        #[cfg(debug_assertions)]
                        eprintln!("[daemon] received ping");
                        let _ = stream.write_all(b"handled\n");
                    }
                    ReadResult::Fallback => {
                        #[cfg(debug_assertions)]
                        eprintln!("[daemon] received unknown command");
                        let _ = stream.write_all(b"fallback\n");
                    }
                    ReadResult::Error(message) => {
                        #[cfg(debug_assertions)]
                        eprintln!("[daemon] read error: {}", message);
                        let _ = stream.write_all(format!("error {}\n", message).as_bytes());
                    }
                    ReadResult::Ignore => {}
                },
                Err(_) => break,
            }
        }
        remove_socket_file(&socket_path);
    });

    true
}

pub fn cleanup() {
    remove_socket_file(socket_path());
}

fn send_raw(msg: &[u8], expect_handled_reply: bool) -> bool {
    let socket_path = socket_path();
    let Ok(mut stream) = UnixStream::connect(&socket_path) else {
        return false;
    };
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));
    if stream.write_all(msg).is_err() {
        return false;
    }
    if !expect_handled_reply {
        return true;
    }
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(timeout));
    let mut buf = [0u8; 128];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 => match std::str::from_utf8(&buf[..n]) {
            Ok(response) => {
                let response = response.trim();
                response.starts_with("handled")
            }
            Err(_) => false,
        },
        Ok(_) => false,
        Err(_) => false,
    }
}

fn socket_path() -> std::path::PathBuf {
    std::env::var("QOL_TRAY_DAEMON_SOCKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(DEFAULT_SOCKET_PATH))
}

fn remove_socket_file(path: impl AsRef<std::path::Path>) {
    let path = path.as_ref();
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_socket() {
        let _ = fs::remove_file(path);
    }
}

fn read_command(stream: &mut UnixStream) -> ReadResult {
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
        "ping" => ReadResult::Handled,
        "show" | "open" => ReadResult::Command(Command::Show),
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
