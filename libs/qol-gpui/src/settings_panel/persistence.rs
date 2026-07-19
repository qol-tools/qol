use std::path::{Path, PathBuf};
use std::time::Duration;

use qol_conventions::DEFAULT_PORT;

pub(super) fn config_path(plugin_id: &str) -> anyhow::Result<PathBuf> {
    qol_config::plugin_config_paths(&[plugin_id])
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no plugin config path available"))
}

fn tray_config_route(plugin_id: &str) -> String {
    format!("/api/plugins/{plugin_id}/config")
}

pub(super) fn query(plugin_id: &str, query: &str) -> Result<serde_json::Value, String> {
    let route = format!("/api/plugins/{plugin_id}/queries/{query}");
    let (status, body) = tray_http("GET", &route, None).map_err(|error| error.to_string())?;
    if status != 200 {
        return Err(format!("query `{query}` failed with HTTP {status}: {body}"));
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("query `{query}` returned invalid JSON: {error}"))
}

pub(super) fn run_action(
    plugin_id: &str,
    action: &str,
    input: &serde_json::Value,
) -> Result<(), String> {
    let route = format!("/api/plugins/{plugin_id}/actions/{action}");
    let body = serde_json::to_string(input).map_err(|error| error.to_string())?;
    let (status, response) =
        tray_http("POST", &route, Some(&body)).map_err(|error| error.to_string())?;
    if (200..300).contains(&status) {
        return Ok(());
    }
    Err(format!(
        "action `{action}` failed with HTTP {status}: {response}"
    ))
}

pub(super) fn load_values(plugin_id: &str, path: &Path) -> serde_json::Value {
    if let Ok((200, body)) = tray_http("GET", &tray_config_route(plugin_id), None) {
        if let Ok(values) = serde_json::from_str(&body) {
            return values;
        }
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub(super) fn save_values(plugin_id: &str, path: &Path, values: &serde_json::Value) {
    let body = match serde_json::to_string(values) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("[{plugin_id}] settings serialize failed: {error:#}");
            return;
        }
    };
    match tray_http("PUT", &tray_config_route(plugin_id), Some(&body)) {
        Ok((200, _)) => return,
        Ok((status, payload)) => {
            eprintln!(
                "[{plugin_id}] settings save rejected by tray ({status}): {}",
                payload.trim()
            );
            return;
        }
        Err(error) => eprintln!("[{plugin_id}] tray unreachable, saving locally: {error:#}"),
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(path, body) {
        eprintln!("[{plugin_id}] settings save failed: {error:#}");
    }
}

fn tray_http(method: &str, route: &str, body: Option<&str>) -> anyhow::Result<(u16, String)> {
    use std::io::{Read, Write};
    let stream = std::net::TcpStream::connect((qol_conventions::LOCAL_HOST, DEFAULT_PORT))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut stream = stream;
    let body = body.unwrap_or("");
    let request = format!(
        "{method} {route} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        qol_conventions::LOCAL_HOST,
        body.len()
    );
    stream.write_all(request.as_bytes())?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    Ok(parse_http_response(&raw))
}

fn parse_http_response(raw: &str) -> (u16, String) {
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

#[cfg(test)]
mod tests {
    use super::parse_http_response;

    #[test]
    fn http_response_parsing_extracts_status_and_body() {
        let cases = [
            ("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}", 200, "{}"),
            (
                "HTTP/1.1 422 Unprocessable\r\n\r\nbad value",
                422,
                "bad value",
            ),
            ("garbage", 0, ""),
        ];
        for (raw, status, body) in cases {
            assert_eq!(
                parse_http_response(raw),
                (status, body.to_string()),
                "raw: {raw}"
            );
        }
    }
}
