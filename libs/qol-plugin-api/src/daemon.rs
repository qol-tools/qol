use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

const ACK_TIMEOUT_MS: u64 = 80;
const REPLACE_EXISTING_ENV: &str = "QOL_TRAY_DAEMON_REPLACE_EXISTING";

pub struct DaemonConfig {
    pub default_socket_name: &'static str,
    pub use_tmpdir_env: bool,
    pub support_replace_existing: bool,
}

pub enum ReadResult<C> {
    Command(C),
    Handled,
    Fallback,
    Error(&'static str),
    Ignore,
}

pub fn socket_path(config: &DaemonConfig) -> PathBuf {
    std::env::var("QOL_TRAY_DAEMON_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            if config.use_tmpdir_env {
                let dir = std::env::var("TMPDIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("/tmp"));
                dir.join(config.default_socket_name)
            } else {
                PathBuf::from("/tmp").join(config.default_socket_name)
            }
        })
}

pub fn send_raw(config: &DaemonConfig, msg: &[u8], expect_reply: bool) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path(config)) else {
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

pub fn send_command(config: &DaemonConfig, cmd: &str, expect_reply: bool) -> bool {
    send_raw(config, cmd.as_bytes(), expect_reply)
}

pub fn send_kill(config: &DaemonConfig) -> bool {
    send_raw(config, b"kill", true)
}

pub fn send_ping(config: &DaemonConfig) -> bool {
    send_raw(config, b"ping", true)
}

pub fn cleanup(config: &DaemonConfig) {
    remove_socket_file(socket_path(config));
}

pub fn start_listener<C: Send + 'static>(
    config: &DaemonConfig,
    tx: Sender<C>,
    parser: fn(&str) -> ReadResult<C>,
) -> bool {
    let socket_path = socket_path(config);
    let support_replace = config.support_replace_existing;

    #[cfg(debug_assertions)]
    eprintln!("[daemon] binding to {:?}", socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            if send_ping(config) {
                if !support_replace || !replace_existing_enabled() {
                    #[cfg(debug_assertions)]
                    eprintln!("[daemon] existing instance alive, exiting");
                    return false;
                }
                #[cfg(debug_assertions)]
                eprintln!("[daemon] replacing existing socket owner");
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
                Ok(mut s) => match read_and_parse(&mut s, parser) {
                    ReadResult::Command(cmd) => {
                        if tx.send(cmd).is_err() {
                            let _ = s.write_all(b"fallback\n");
                            break;
                        }
                        let _ = s.write_all(b"handled\n");
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

fn read_and_parse<C>(stream: &mut UnixStream, parser: fn(&str) -> ReadResult<C>) -> ReadResult<C> {
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
    parser(cmd)
}

fn replace_existing_enabled() -> bool {
    std::env::var(REPLACE_EXISTING_ENV)
        .ok()
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
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
