use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

const PORT: u16 = 42720;

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
    let Some(server_path) = find_server_script(server_script_dirs()) else {
        eprintln!("Missing daemon server script server.py (looked next to the binary and in QOL_TRAY_PLUGIN_DIR)");
        return ExitCode::from(1);
    };

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
        format!("Task Runner daemon is running on port {PORT}")
    } else {
        "Task Runner daemon is NOT running".to_string()
    };

    send_notification("Task Runner", &message);
    ExitCode::SUCCESS
}

fn daemon_is_running() -> bool {
    let mut stream = match TcpStream::connect(("127.0.0.1", PORT)) {
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

fn server_script_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(exe_dir) = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
    {
        dirs.push(exe_dir);
    }
    if let Some(plugin_dir) = env::var_os("QOL_TRAY_PLUGIN_DIR") {
        dirs.push(PathBuf::from(plugin_dir));
    }
    dirs
}

fn find_server_script(dirs: Vec<PathBuf>) -> Option<PathBuf> {
    dirs.into_iter()
        .map(|dir| dir.join("server.py"))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }

    #[test]
    fn find_server_script_picks_first_dir_containing_it() {
        let empty = tempfile::tempdir().unwrap();
        let with_script = tempfile::tempdir().unwrap();
        std::fs::write(with_script.path().join("server.py"), "").unwrap();

        let dirs = vec![empty.path().to_path_buf(), with_script.path().to_path_buf()];
        assert_eq!(
            super::find_server_script(dirs),
            Some(with_script.path().join("server.py")),
            "skips dirs without the script"
        );
        assert_eq!(
            super::find_server_script(vec![empty.path().to_path_buf()]),
            None,
            "no candidate yields None"
        );
    }
}
