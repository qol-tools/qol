use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

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

pub fn send_action(config: &DaemonConfig, action: &str, expect_reply: bool) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path(config)) else {
        return false;
    };
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));

    let request = DaemonRequest { action: action.to_string() };
    let Ok(mut payload) = serde_json::to_string(&request) else {
        return false;
    };
    payload.push('\n');

    if stream.write_all(payload.as_bytes()).is_err() {
        return false;
    }
    if !expect_reply {
        return true;
    }

    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(timeout));

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => false,
        Ok(_) => matches!(
            serde_json::from_str::<DaemonResponse>(line.trim()),
            Ok(DaemonResponse::Handled { .. })
        ),
    }
}

pub fn send_kill(config: &DaemonConfig) -> bool {
    send_action(config, "kill", true)
}

pub fn send_ping(config: &DaemonConfig) -> bool {
    send_action(config, "ping", true)
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
                Ok(mut s) => {
                    let result = read_and_parse(&mut s, parser);
                    match result {
                        ReadResult::Command(cmd) => {
                            let resp = if tx.send(cmd).is_ok() {
                                DaemonResponse::Handled { data: None }
                            } else {
                                DaemonResponse::Fallback
                            };
                            let is_fallback = matches!(resp, DaemonResponse::Fallback);
                            write_response(&mut s, &resp);
                            if is_fallback {
                                break;
                            }
                        }
                        ReadResult::Handled => {
                            write_response(&mut s, &DaemonResponse::Handled { data: None });
                        }
                        ReadResult::Fallback => {
                            write_response(&mut s, &DaemonResponse::Fallback);
                        }
                        ReadResult::Error(msg) => {
                            write_response(
                                &mut s,
                                &DaemonResponse::Error {
                                    message: msg.to_string(),
                                },
                            );
                        }
                        ReadResult::Ignore => {}
                    }
                }
                Err(_) => break,
            }
        }
        remove_socket_file(&socket_path);
    });

    true
}

fn read_and_parse<C>(stream: &mut UnixStream, parser: fn(&str) -> ReadResult<C>) -> ReadResult<C> {
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_read_timeout(Some(timeout));

    let mut reader = BufReader::new(&*stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return ReadResult::Ignore,
        Ok(_) => {}
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ReadResult::Ignore;
    }

    if let Ok(request) = serde_json::from_str::<DaemonRequest>(trimmed) {
        return parser(&request.action);
    }

    let cmd = match trimmed.strip_prefix("action:") {
        Some(a) => a,
        None => trimmed,
    };
    parser(cmd)
}

fn write_response(stream: &mut UnixStream, response: &DaemonResponse) {
    if let Ok(json) = serde_json::to_string(response) {
        let _ = stream.write_all(json.as_bytes());
        let _ = stream.write_all(b"\n");
    }
}

fn replace_existing_enabled() -> bool {
    std::env::var(REPLACE_EXISTING_ENV).ok().is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
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
