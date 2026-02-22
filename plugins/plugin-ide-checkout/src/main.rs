use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("daemon") => run_daemon(),
        Some("status") => run_status(),
        Some(action) => {
            eprintln!("Unknown action: {action}");
            ExitCode::from(1)
        }
    }
}

fn run_daemon() -> ExitCode {
    let server_path = plugin_dir().join("server.py");
    if !server_path.is_file() {
        eprintln!("Missing daemon server script: {}", server_path.display());
        return ExitCode::from(1);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = Command::new("python3").arg(&server_path).exec();
        eprintln!("Failed to start daemon: {error}");
        ExitCode::from(1)
    }

    #[cfg(not(unix))]
    {
        match Command::new("python3")
            .arg(&server_path)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
        {
            Ok(status) if status.success() => ExitCode::SUCCESS,
            Ok(_) => ExitCode::from(1),
            Err(error) => {
                eprintln!("Failed to start daemon: {error}");
                ExitCode::from(1)
            }
        }
    }
}

fn run_status() -> ExitCode {
    let message = if daemon_is_running() {
        "Task Runner daemon is running on port 42710"
    } else {
        "Task Runner daemon is NOT running"
    };

    send_notification("Task Runner", message);
    ExitCode::SUCCESS
}

fn daemon_is_running() -> bool {
    let mut stream = match TcpStream::connect("127.0.0.1:42710") {
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

fn send_notification(title: &str, message: &str) {
    if send_osascript_notification(title, message) {
        return;
    }

    if send_notify_send_notification(title, message) {
        return;
    }

    println!("{title}: {message}");
}

fn send_osascript_notification(title: &str, message: &str) -> bool {
    let escaped_title = escape_applescript(title);
    let escaped_message = escape_applescript(message);
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escaped_message, escaped_title
    );

    Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn send_notify_send_notification(title: &str, message: &str) -> bool {
    Command::new("notify-send")
        .arg(title)
        .arg(message)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn escape_applescript(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

fn plugin_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        let manifest_str =
            std::fs::read_to_string("plugin.toml").expect("Failed to read plugin.toml");
        let manifest: PluginManifest =
            toml::from_str(&manifest_str).expect("Failed to parse plugin.toml");
        manifest.validate().expect("Manifest validation failed");
    }
}
