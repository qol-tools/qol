use anyhow::Result;
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

const DEFAULT_SOCKET_PATH: &str = "/tmp/qol-pointz.sock";
const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-pointz/";

#[cfg(unix)]
pub async fn run() -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    let socket_path = socket_path();
    let _ = tokio::fs::remove_file(&socket_path).await;
    let listener = UnixListener::bind(&socket_path)?;
    log::info!("IPC server listening on {}", socket_path);

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                return;
            }

            let action = if let Ok(req) = serde_json::from_str::<DaemonRequest>(trimmed) {
                req.action
            } else {
                match trimmed.strip_prefix("action:") {
                    Some(a) => a.to_string(),
                    None => trimmed.to_string(),
                }
            };

            let response = match execute_action(&action) {
                Ok(()) => DaemonResponse::Handled { data: None },
                Err("unknown action") => DaemonResponse::Fallback,
                Err(msg) => DaemonResponse::Error { message: msg.to_string() },
            };

            if let Ok(json) = serde_json::to_string(&response) {
                let _ = writer.write_all(json.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
            }
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
