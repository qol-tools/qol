use anyhow::Result;

const DEFAULT_SOCKET_PATH: &str = "/tmp/qol-pointz.sock";
const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-pointz/";

#[cfg(unix)]
pub async fn run() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    let socket_path = socket_path();
    let _ = tokio::fs::remove_file(&socket_path).await;
    let listener = UnixListener::bind(&socket_path)?;
    log::info!("IPC server listening on {}", socket_path);

    loop {
        let (mut stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0u8; 128];
            let n = match stream.read(&mut buf).await {
                Ok(n) => n,
                Err(_) => return,
            };
            if n == 0 {
                return;
            }

            let raw = match std::str::from_utf8(&buf[..n]) {
                Ok(value) => value.trim(),
                Err(_) => {
                    let _ = stream.write_all(b"error invalid utf8\n").await;
                    return;
                }
            };
            let action = match raw.strip_prefix("action:") {
                Some(action_id) => {
                    if !is_valid_action_id(action_id) {
                        let _ = stream.write_all(b"error invalid action id\n").await;
                        return;
                    }
                    action_id
                }
                None => raw,
            };

            let response = match action {
                "settings" => match execute_action("settings") {
                    Ok(()) => b"handled\n".to_vec(),
                    Err(message) => format!("error {}\n", message).into_bytes(),
                },
                _ => b"fallback\n".to_vec(),
            };
            let _ = stream.write_all(&response).await;
        });
    }
}

#[cfg(unix)]
fn socket_path() -> String {
    std::env::var("QOL_TRAY_DAEMON_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string())
}

#[cfg(not(unix))]
pub async fn run() -> Result<()> {
    Ok(())
}

fn is_valid_action_id(action: &str) -> bool {
    !action.is_empty()
        && action.len() <= 64
        && !action.starts_with('-')
        && action
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn execute_action(action: &str) -> Result<(), &'static str> {
    match action {
        "settings" => open_settings(),
        _ => Err("unknown action"),
    }
}

fn open_settings() -> Result<(), &'static str> {
    #[cfg(target_os = "linux")]
    {
        return std::process::Command::new("xdg-open")
            .arg(SETTINGS_URL)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|_| "failed to open settings url");
    }

    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("open")
            .arg(SETTINGS_URL)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|_| "failed to open settings url");
    }

    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("cmd")
            .args(["/C", "start", SETTINGS_URL])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|_| "failed to open settings url");
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err("unsupported platform")
    }
}
