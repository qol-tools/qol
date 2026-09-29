use base64::Engine;
use qol_runtime::local_http::{Client, Method};
use std::io;
use std::time::Duration;

const DAEMON_IO_TIMEOUT: Duration = Duration::from_secs(5);

pub fn read_auth_token() -> io::Result<String> {
    let path = qol_config::http_auth_token_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Could not determine HTTP auth token path",
        )
    })?;
    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP auth token path is not a regular file",
        ));
    }
    let token = std::fs::read_to_string(&path)?;
    let token = token.trim();
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP auth token is not valid base64url",
            )
        })?;
    if decoded.len() != 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP auth token must be 32 bytes",
        ));
    }
    Ok(token.to_string())
}

pub fn get_from_daemon(path: &str) -> io::Result<(u16, String)> {
    let response = daemon_client(DAEMON_IO_TIMEOUT)?.request(Method::Get, path, None)?;
    Ok((response.status, response.body))
}

pub fn post_to_daemon(path: &str, body: &str) -> io::Result<(u16, String)> {
    post_to_daemon_with_timeout(path, body, DAEMON_IO_TIMEOUT)
}

pub fn post_to_daemon_with_timeout(
    path: &str,
    body: &str,
    timeout: Duration,
) -> io::Result<(u16, String)> {
    let body = if body.is_empty() { "{}" } else { body };
    let response = daemon_client(timeout)?.request(Method::Post, path, Some(body))?;
    Ok((response.status, response.body))
}

fn daemon_client(timeout: Duration) -> io::Result<Client> {
    Ok(Client::new(qol_conventions::DEFAULT_PORT, read_auth_token()?).with_io_timeout(timeout))
}

pub fn run_exec(target: &str, action: &str) -> Result<(), String> {
    qol_runtime::probe!(
        "ACTION_EXEC",
        "plugin={} action={} phase=start",
        target,
        action
    );
    if target == "shortcut" {
        return fire_shortcut_request(action);
    }
    if !crate::manifest::is_valid_plugin_id(target) {
        return Err(format!("Invalid plugin id: {target}"));
    }
    if !crate::manifest::is_valid_action_id(action) {
        return Err(format!("Invalid action id: {action}"));
    }
    fire_action_request(target, action)
}

fn fire_shortcut_request(id: &str) -> Result<(), String> {
    if crate::manifest::validate_safe_identifier(id).is_err() {
        return Err(format!("Invalid shortcut id: {id}"));
    }
    fire_daemon_post("shortcut", id, &format!("/api/shortcuts/{id}/execute"))
}

fn fire_action_request(plugin_id: &str, action_id: &str) -> Result<(), String> {
    let path = format!("/api/plugins/{plugin_id}/actions/{action_id}");
    fire_daemon_post(plugin_id, action_id, &path)
}

fn fire_daemon_post(plugin_id: &str, action_id: &str, path: &str) -> Result<(), String> {
    #[cfg(debug_assertions)]
    let started = std::time::Instant::now();
    #[cfg(not(debug_assertions))]
    let started = ();
    trace_action_exec("send", plugin_id, action_id, &started);
    let result = post_to_daemon(path, "");
    trace_action_exec("sent", plugin_id, action_id, &started);
    match result {
        Ok((status, _)) if (200..300).contains(&status) => Ok(()),
        Ok((status, body)) if body.is_empty() => Err(format!("Request failed (HTTP {status})")),
        Ok((_, body)) => Err(body),
        Err(_) => Err("qol-tray is not running".to_string()),
    }
}

#[cfg(debug_assertions)]
fn trace_action_exec(phase: &str, plugin_id: &str, action_id: &str, started: &std::time::Instant) {
    qol_runtime::probe!(
        "ACTION_EXEC",
        "plugin={plugin_id} action={action_id} phase={phase} elapsed_ms={}",
        started.elapsed().as_millis()
    );
}

#[cfg(not(debug_assertions))]
fn trace_action_exec(phase: &str, plugin_id: &str, action_id: &str, started: &()) {
    let _ = (phase, plugin_id, action_id, started);
}

#[cfg(test)]
mod tests {
    use super::run_exec;

    #[test]
    fn run_exec_rejects_invalid_plugin_and_action_ids_without_network() {
        assert!(run_exec("bad plugin id!", "settings").is_err());
        assert!(run_exec("qol-monitor", "bad action!").is_err());
        assert!(run_exec("shortcut", "bad id!").is_err());
        assert!(run_exec("qol-monitor", "").is_err());
    }
}
