use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

pub fn bind_with_takeover(port: u16) -> std::io::Result<TcpListener> {
    match try_bind(port) {
        Ok(listener) => return Ok(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {}
        Err(error) => return Err(error),
    }

    if !replace_existing_enabled() {
        return try_bind(port);
    }

    eprintln!("[task-runner] port {port} busy, replacing existing instance");
    request_shutdown(port);
    if let Some(listener) = retry_bind(port, Duration::from_millis(1500)) {
        return Ok(listener);
    }

    eprintln!("[task-runner] existing instance did not yield; forcing takeover");
    force_kill_listeners(port);
    if let Some(listener) = retry_bind(port, Duration::from_secs(3)) {
        return Ok(listener);
    }

    try_bind(port)
}

fn try_bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

fn retry_bind(port: u16, timeout: Duration) -> Option<TcpListener> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(listener) = try_bind(port) {
            return Some(listener);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn replace_existing_enabled() -> bool {
    std::env::var(qol_conventions::ENV_DAEMON_REPLACE_EXISTING)
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn request_shutdown(port: u16) {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let request =
        "POST /shutdown HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let _ = stream.write_all(request.as_bytes());
    let mut discard = [0; 64];
    let _ = stream.read(&mut discard);
}

fn force_kill_listeners(port: u16) {
    for pid in listening_pids(port) {
        if pid == std::process::id() || !is_our_daemon(pid) {
            continue;
        }
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        std::thread::sleep(Duration::from_millis(500));
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status();
    }
}

fn is_our_daemon(pid: u32) -> bool {
    let Some(marker) = ownership_marker() else {
        return false;
    };
    let Ok(output) = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains(&marker)
}

// A stale instance was launched from this same install: the host spawns the
// daemon by its absolute path and sets QOL_TRAY_PLUGIN_DIR, so the install
// directory appears verbatim in the candidate's command line. Matching that
// absolute path proves ownership; a bare "task-runner" substring does not.
fn ownership_marker() -> Option<String> {
    if let Some(dir) = std::env::var_os("QOL_TRAY_PLUGIN_DIR") {
        if !dir.is_empty() {
            return Some(dir.to_string_lossy().into_owned());
        }
    }
    std::env::current_exe()
        .ok()
        .map(|exe| exe.to_string_lossy().into_owned())
}

fn listening_pids(port: u16) -> Vec<u32> {
    let Ok(output) = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|token| token.parse().ok())
        .collect()
}
