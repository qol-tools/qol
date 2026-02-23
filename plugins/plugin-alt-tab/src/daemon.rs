use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::Sender;

const ACK_TIMEOUT_MS: u64 = 80;

pub enum Command {
    Show,
    ShowReverse,
    Kill,
}

pub fn send_show() -> bool {
    send_raw(b"show", false)
}

pub fn send_show_reverse() -> bool {
    send_raw(b"show-reverse", false)
}

pub fn send_kill() -> bool {
    send_raw(b"kill", true)
}

fn send_ping() -> bool {
    send_raw(b"ping", true)
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    let socket_path = socket_path();
    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            if send_ping() {
                return false; // existing instance is alive
            }
            remove_socket_file(&socket_path);
            let Ok(l) = UnixListener::bind(&socket_path) else {
                return false;
            };
            l
        }
        Err(_) => return false,
    };

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut s) => match read_command(&mut s) {
                    ReadResult::Command(cmd) => {
                        let _ = s.write_all(b"handled\n");
                        if tx.send(cmd).is_err() {
                            break;
                        }
                    }
                    ReadResult::Handled => {
                        let _ = s.write_all(b"handled\n");
                    }
                    ReadResult::Fallback => {
                        let _ = s.write_all(b"fallback\n");
                    }
                    ReadResult::Error(msg) => {
                        let _ = s.write_all(format!("error {}\n", msg).as_bytes());
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

fn send_raw(msg: &[u8], expect_reply: bool) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));
    if stream.write_all(msg).is_err() {
        return false;
    }
    if !expect_reply {
        return true;
    }
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(timeout));
    let mut buf = [0u8; 128];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 => std::str::from_utf8(&buf[..n])
            .map(|s| s.trim().starts_with("handled"))
            .unwrap_or(false),
        _ => false,
    }
}

fn socket_path() -> std::path::PathBuf {
    std::env::var("QOL_TRAY_DAEMON_SOCKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let dir = std::env::var("TMPDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
            dir.join("qol-alt-tab.sock")
        })
}

fn remove_socket_file(path: impl AsRef<std::path::Path>) {
    let path = path.as_ref();
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_socket() {
        let _ = fs::remove_file(path);
    }
}

enum ReadResult {
    Command(Command),
    Handled,
    Fallback,
    Error(&'static str),
    Ignore,
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
        Ok(v) => v.trim(),
        Err(_) => return ReadResult::Error("invalid utf8"),
    };
    let cmd = match raw.strip_prefix("action:") {
        Some(a) => a,
        None => raw,
    };
    match cmd {
        "ping" => ReadResult::Handled,
        "show" | "open" => ReadResult::Command(Command::Show),
        "show-reverse" | "open-reverse" => ReadResult::Command(Command::ShowReverse),
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}
