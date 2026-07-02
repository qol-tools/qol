use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

const ACK_TIMEOUT_MS: u64 = 80;
const REPLACE_EXISTING_ENV: &str = qol_conventions::ENV_DAEMON_REPLACE_EXISTING;

pub struct DaemonConfig {
    pub socket: SocketSource,
    pub support_replace_existing: bool,
}

pub enum SocketSource {
    EnvRequired,
    Fallback {
        default_socket_name: &'static str,
        use_tmpdir_env: bool,
    },
}

pub enum ReadResult<C> {
    Command(C),
    Handled,
    HandledWithData(serde_json::Value),
    Fallback,
    Error(String),
    Ignore,
}

pub fn socket_path(config: &DaemonConfig) -> Option<PathBuf> {
    if let Ok(path) = std::env::var(qol_conventions::ENV_DAEMON_SOCKET) {
        return Some(PathBuf::from(path));
    }
    match &config.socket {
        SocketSource::EnvRequired => None,
        SocketSource::Fallback {
            default_socket_name,
            use_tmpdir_env,
        } => Some(fallback_socket_path(default_socket_name, *use_tmpdir_env)),
    }
}

fn fallback_socket_path(name: &str, use_tmpdir_env: bool) -> PathBuf {
    if use_tmpdir_env {
        let dir = std::env::var("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        dir.join(name)
    } else {
        PathBuf::from("/tmp").join(name)
    }
}

pub fn send_action(config: &DaemonConfig, action: &str, expect_reply: bool) -> bool {
    let Some(path) = socket_path(config) else {
        #[cfg(debug_assertions)]
        eprintln!(
            "[daemon] {} unset and no fallback socket - cannot send action",
            qol_conventions::ENV_DAEMON_SOCKET
        );
        return false;
    };
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_write_timeout(Some(timeout));

    let request = DaemonRequest {
        action: action.to_string(),
    };
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
    if let Some(path) = socket_path(config) {
        remove_socket_file(path);
    }
}

pub fn start_listener<C: Send + 'static>(
    config: &DaemonConfig,
    tx: Sender<C>,
    parser: fn(&str) -> ReadResult<C>,
) -> bool {
    let Ok((listener, socket_path)) = bind_listener(config) else {
        return false;
    };

    qol_runtime::spawn_host_death_watchdog();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut s) => {
                    let result = read_and_parse(&mut s, parser);
                    let should_continue = handle_read_result(&mut s, result, |cmd| {
                        if tx.send(cmd).is_ok() {
                            DaemonResponse::Handled { data: None }
                        } else {
                            DaemonResponse::Fallback
                        }
                    });
                    if !should_continue {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        remove_socket_file(&socket_path);
    });

    true
}

pub fn run_stateful_listener<S, F>(
    config: &DaemonConfig,
    mut state: S,
    mut handler: F,
) -> io::Result<()>
where
    F: FnMut(&mut S, &str) -> ReadResult<()>,
{
    let (listener, socket_path) = bind_listener(config)?;

    qol_runtime::spawn_host_death_watchdog();

    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                let result = read_and_parse(&mut s, |action| handler(&mut state, action));
                handle_read_result(&mut s, result, |_| DaemonResponse::Handled { data: None });
            }
            Err(error) => {
                eprintln!("accept error: {error:#}");
            }
        }
    }

    remove_socket_file(socket_path);
    Ok(())
}

fn bind_listener(config: &DaemonConfig) -> io::Result<(UnixListener, PathBuf)> {
    let Some(socket_path) = socket_path(config) else {
        #[cfg(debug_assertions)]
        eprintln!(
            "[daemon] {} unset and no fallback socket - not binding",
            qol_conventions::ENV_DAEMON_SOCKET
        );
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("{} is not set", qol_conventions::ENV_DAEMON_SOCKET),
        ));
    };
    let support_replace = config.support_replace_existing;

    #[cfg(debug_assertions)]
    eprintln!("[daemon] binding to {:?}", socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            if send_ping(config) {
                if !support_replace || !replace_existing_enabled() {
                    #[cfg(debug_assertions)]
                    eprintln!("[daemon] existing instance alive, exiting");
                    return Err(io::Error::new(
                        ErrorKind::AddrInUse,
                        "existing daemon instance is alive",
                    ));
                }
                #[cfg(debug_assertions)]
                eprintln!("[daemon] replacing existing socket owner");
            }
            remove_socket_file(&socket_path);
            UnixListener::bind(&socket_path)?
        }
        Err(error) => return Err(error),
    };

    Ok((listener, socket_path))
}

fn read_and_parse<C, F>(stream: &mut UnixStream, mut parser: F) -> ReadResult<C>
where
    F: FnMut(&str) -> ReadResult<C>,
{
    let timeout = std::time::Duration::from_millis(ACK_TIMEOUT_MS);
    let _ = stream.set_read_timeout(Some(timeout));

    let mut reader = BufReader::new(&*stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => {
            #[cfg(debug_assertions)]
            eprintln!("[daemon] read_line EOF (0 bytes)");
            return ReadResult::Ignore;
        }
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("[daemon] read_line error: {:?}", e);
            #[cfg(not(debug_assertions))]
            let _ = e;
            return ReadResult::Ignore;
        }
        Ok(_) => {}
    }

    let trimmed = line.trim();
    #[cfg(debug_assertions)]
    eprintln!("[daemon] read line: {:?}", trimmed);

    if trimmed.is_empty() {
        return ReadResult::Ignore;
    }

    if let Ok(request) = serde_json::from_str::<DaemonRequest>(trimmed) {
        #[cfg(debug_assertions)]
        eprintln!("[daemon] parsed DaemonRequest action: {:?}", request.action);
        return parser(&request.action);
    }

    let cmd = match trimmed.strip_prefix("action:") {
        Some(a) => a,
        None => trimmed,
    };
    parser(cmd)
}

fn handle_read_result<C, F>(
    stream: &mut UnixStream,
    result: ReadResult<C>,
    command_response: F,
) -> bool
where
    F: FnOnce(C) -> DaemonResponse,
{
    let response = match result {
        ReadResult::Command(cmd) => {
            let response = command_response(cmd);
            let should_continue = !matches!(response, DaemonResponse::Fallback);
            write_response(stream, &response);
            return should_continue;
        }
        ReadResult::Handled => DaemonResponse::Handled { data: None },
        ReadResult::HandledWithData(data) => DaemonResponse::Handled { data: Some(data) },
        ReadResult::Fallback => DaemonResponse::Fallback,
        ReadResult::Error(message) => DaemonResponse::Error { message },
        ReadResult::Ignore => return true,
    };
    write_response(stream, &response);
    true
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_SOCKET_ID: AtomicUsize = AtomicUsize::new(0);

    fn stream_with_line(line: &str) -> UnixStream {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(line.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        reader
    }

    fn response_for_result<C>(
        result: ReadResult<C>,
        command_response: impl FnOnce(C) -> DaemonResponse,
    ) -> (bool, DaemonResponse) {
        let (mut server, client) = UnixStream::pair().unwrap();
        let should_continue = handle_read_result(&mut server, result, command_response);
        drop(server);

        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let response = serde_json::from_str(line.trim()).unwrap();
        (should_continue, response)
    }

    fn temp_socket_name(tag: &str) -> &'static str {
        let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        Box::leak(
            format!("qol-plugin-daemon-{tag}-{}-{id}.sock", std::process::id()).into_boxed_str(),
        )
    }

    fn fallback_config(socket_name: &'static str) -> DaemonConfig {
        DaemonConfig {
            socket: SocketSource::Fallback {
                default_socket_name: socket_name,
                use_tmpdir_env: false,
            },
            support_replace_existing: false,
        }
    }

    #[test]
    fn parses_json_request_action() {
        let mut stream = stream_with_line(r#"{"action":"open"}"#);
        let result = read_and_parse(&mut stream, |action| {
            ReadResult::<String>::Command(action.to_string())
        });

        match result {
            ReadResult::Command(action) => assert_eq!(action, "open"),
            _ => panic!("expected command"),
        }
    }

    #[test]
    fn parses_plain_and_prefixed_actions() {
        for (line, expected) in [("open", "open"), ("action:reload", "reload")] {
            let mut stream = stream_with_line(line);
            let result = read_and_parse(&mut stream, |action| {
                ReadResult::<String>::Command(action.to_string())
            });

            match result {
                ReadResult::Command(action) => assert_eq!(action, expected),
                _ => panic!("expected command"),
            }
        }
    }

    #[test]
    fn ignores_empty_requests() {
        let mut stream = stream_with_line("");
        let result = read_and_parse(&mut stream, |_| ReadResult::<()>::Handled);

        match result {
            ReadResult::Ignore => {}
            _ => panic!("expected ignore"),
        }
    }

    #[test]
    fn writes_handled_data_response() {
        let payload = serde_json::json!({ "state": "offline" });
        let (should_continue, response) = response_for_result::<()>(
            ReadResult::HandledWithData(payload.clone()),
            |_| unreachable!(),
        );

        assert!(should_continue);
        match response {
            DaemonResponse::Handled { data } => assert_eq!(data, Some(payload)),
            _ => panic!("expected handled response"),
        }
    }

    #[test]
    fn fallback_response_keeps_listener_running() {
        let (should_continue, response) =
            response_for_result::<()>(ReadResult::Fallback, |_| unreachable!());

        assert!(should_continue);
        match response {
            DaemonResponse::Fallback => {}
            _ => panic!("expected fallback response"),
        }
    }

    #[test]
    fn command_send_fallback_stops_threaded_listener() {
        let (should_continue, response) =
            response_for_result(ReadResult::Command(()), |_| DaemonResponse::Fallback);

        assert!(!should_continue);
        match response {
            DaemonResponse::Fallback => {}
            _ => panic!("expected fallback response"),
        }
    }

    #[test]
    fn writes_owned_error_message() {
        let (should_continue, response) = response_for_result::<()>(
            ReadResult::Error("hardware offline".to_string()),
            |_| unreachable!(),
        );

        assert!(should_continue);
        match response {
            DaemonResponse::Error { message } => assert_eq!(message, "hardware offline"),
            _ => panic!("expected error response"),
        }
    }

    #[test]
    fn parser_can_keep_state_across_requests() {
        let mut count = 0;

        for line in ["first", "second"] {
            let mut stream = stream_with_line(line);
            let result = read_and_parse(&mut stream, |action| {
                count += 1;
                assert_eq!(action, line);
                ReadResult::<()>::Handled
            });

            match result {
                ReadResult::Handled => {}
                _ => panic!("expected handled"),
            }
        }

        assert_eq!(count, 2);
    }

    #[test]
    fn bind_listener_replaces_stale_socket() {
        let socket_name = temp_socket_name("stale");
        let path = PathBuf::from("/tmp").join(socket_name);
        let _ = fs::remove_file(&path);

        {
            let _stale_listener = UnixListener::bind(&path).unwrap();
        }

        assert!(fs::symlink_metadata(&path).is_ok());

        let config = fallback_config(socket_name);
        let (_listener, bound_path) = bind_listener(&config).unwrap();

        assert_eq!(bound_path, path);
        remove_socket_file(path);
    }
}
