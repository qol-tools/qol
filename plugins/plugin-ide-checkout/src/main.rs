mod daemon;

use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

fn daemon_port() -> u16 {
    env!("QOL_DAEMON_PORT")
        .parse()
        .expect("QOL_DAEMON_PORT must be a valid u16")
}

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("daemon") => daemon::run(),
        Some("status") => run_status(),
        Some(action) => {
            eprintln!("Unknown action: {action}");
            ExitCode::from(1)
        }
    }
}

fn run_status() -> ExitCode {
    let message = if daemon_is_running() {
        format!("Task Runner daemon is running on port {}", daemon_port())
    } else {
        "Task Runner daemon is NOT running".to_string()
    };

    qol_plugin_daemon::notification::send_notification("Task Runner", &message);
    ExitCode::SUCCESS
}

fn daemon_is_running() -> bool {
    let mut stream = match TcpStream::connect(("127.0.0.1", daemon_port())) {
        Ok(stream) => stream,
        Err(_) => return false,
    };

    if stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .is_err()
    {
        return false;
    }
    if stream
        .set_write_timeout(Some(Duration::from_millis(300)))
        .is_err()
    {
        return false;
    }

    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }

    let mut buffer = [0_u8; 256];
    let size = match stream.read(&mut buffer) {
        Ok(size) if size > 0 => size,
        _ => return false,
    };

    let response = match std::str::from_utf8(&buffer[..size]) {
        Ok(response) => response,
        Err(_) => return false,
    };

    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
