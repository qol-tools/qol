use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

pub(crate) const WEBSITE_URL: &str = "http://localhost:42700";
const WEB_HEALTH_URL: &str = "http://127.0.0.1:42700/";
const DEV_HEALTH_URL: &str = "http://127.0.0.1:42700/api/dev/worktrees";
const DEV_RECOMPILE_URL: &str = "http://127.0.0.1:42700/api/dev/recompile-self";
const DEV_RELOAD_URL: &str = "http://127.0.0.1:42700/api/dev/reload";
const DEV_LINKS_URL: &str = "http://127.0.0.1:42700/api/dev/links";
const AUTH_HEALTH_URL: &str = "http://127.0.0.1:42700/api/auth/health";
const LOGS_HEALTH_URL: &str = "http://127.0.0.1:42700/api/logs/entries";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_INTERVAL: Duration = Duration::from_millis(250);
const HTTP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub(crate) enum DevLinkOutcome {
    Created,
    AlreadyLinked,
}

pub(crate) fn wait_for_health() -> Result<()> {
    let deadline = Instant::now() + HEALTH_TIMEOUT;
    while Instant::now() < deadline {
        if health_ok() {
            return Ok(());
        }
        std::thread::sleep(HEALTH_INTERVAL);
    }
    bail!("qol-tray dev server did not become healthy");
}

pub(crate) fn post_recompile(branch: &str) -> Result<()> {
    let body = json!({ "worktree_branch": branch }).to_string();
    post_recompile_body(&body)
}

pub(crate) fn health_ok() -> bool {
    http_get_ok(DEV_HEALTH_URL).unwrap_or(false)
}

pub(crate) fn web_ok() -> bool {
    http_get_ok(WEB_HEALTH_URL).unwrap_or(false)
}

pub(crate) struct EndpointStatus {
    pub(crate) label: &'static str,
    pub(crate) url: &'static str,
    pub(crate) ok: bool,
}

pub(crate) fn probe_endpoints() -> Vec<EndpointStatus> {
    const ENDPOINTS: [(&str, &str); 4] = [
        ("website", WEB_HEALTH_URL),
        ("dev api", DEV_HEALTH_URL),
        ("github", AUTH_HEALTH_URL),
        ("logs", LOGS_HEALTH_URL),
    ];
    ENDPOINTS
        .iter()
        .map(|(label, url)| EndpointStatus {
            label,
            url,
            ok: http_get_ok(url).unwrap_or(false),
        })
        .collect()
}

pub(crate) fn post_recompile_current() -> Result<()> {
    post_recompile_body("{}")
}

pub(crate) fn post_reload_plugins() -> Result<()> {
    let status = http_request("POST", DEV_RELOAD_URL, Some("{}"))?;
    if status / 100 == 2 {
        return Ok(());
    }
    bail!("plugin reload request failed with HTTP {status}");
}

fn post_recompile_body(body: &str) -> Result<()> {
    let status = http_request("POST", DEV_RECOMPILE_URL, Some(body))?;
    if status / 100 == 2 {
        return Ok(());
    }
    bail!("recompile request failed with HTTP {status}");
}

pub(crate) fn post_dev_link(plugin_dir: &std::path::Path) -> Result<DevLinkOutcome> {
    let path = plugin_dir.to_string_lossy().to_string();
    let body = json!({ "path": path }).to_string();
    let status = http_request("POST", DEV_LINKS_URL, Some(&body))?;
    classify_link_status(status)
}

fn classify_link_status(status: u16) -> Result<DevLinkOutcome> {
    match status {
        200..=299 => Ok(DevLinkOutcome::Created),
        409 => Ok(DevLinkOutcome::AlreadyLinked),
        other => bail!("dev-link request failed with HTTP {other}"),
    }
}

fn http_get_ok(url: &str) -> Result<bool> {
    let status = http_request("GET", url, None)?;
    Ok(status == 200)
}

fn http_request(method: &str, url: &str, body: Option<&str>) -> Result<u16> {
    let target = HttpTarget::parse(url)?;
    let mut addrs = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve {}", target.host))?;
    let addr = addrs
        .next()
        .ok_or_else(|| anyhow!("no address for {}", target.host))?;
    let mut stream = TcpStream::connect_timeout(&addr, HTTP_TIMEOUT)
        .with_context(|| format!("failed to connect to {}", target.host))?;
    stream.set_read_timeout(Some(HTTP_TIMEOUT))?;
    stream.set_write_timeout(Some(HTTP_TIMEOUT))?;
    let body = body.unwrap_or("");
    let request = format!(
        "{method} {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        target.path,
        target.host,
        target.port,
        body.len(),
        body
    );
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    parse_http_status(&response)
}

struct HttpTarget {
    host: String,
    port: u16,
    path: String,
}

impl HttpTarget {
    fn parse(url: &str) -> Result<Self> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| anyhow!("only http:// URLs are supported"))?;
        let slash = rest
            .find('/')
            .ok_or_else(|| anyhow!("URL has no path: {url}"))?;
        let authority = &rest[..slash];
        let path = &rest[slash..];
        let colon = authority
            .rfind(':')
            .ok_or_else(|| anyhow!("URL has no port: {url}"))?;
        let host = &authority[..colon];
        let port = authority[colon + 1..]
            .parse::<u16>()
            .with_context(|| format!("invalid port in {url}"))?;
        Ok(Self {
            host: host.to_string(),
            port,
            path: path.to_string(),
        })
    }
}

fn parse_http_status(response: &str) -> Result<u16> {
    let line = response
        .lines()
        .next()
        .ok_or_else(|| anyhow!("empty HTTP response"))?;
    let status = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow!("malformed HTTP status line: {line}"))?;
    status
        .parse::<u16>()
        .with_context(|| format!("invalid HTTP status in {line}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_status_line() {
        assert_eq!(
            parse_http_status("HTTP/1.1 202 Accepted\r\n\r\n").unwrap(),
            202
        );
    }

    #[test]
    fn classify_link_status_handles_known_codes() {
        let cases = [
            (200, Some(false)),
            (201, Some(false)),
            (204, Some(false)),
            (409, Some(true)),
            (400, None),
            (500, None),
            (502, None),
        ];
        for (status, want) in cases {
            let got = classify_link_status(status);
            match want {
                Some(already) => {
                    let outcome = got.unwrap_or_else(|e| panic!("status {status}: {e}"));
                    let is_already = matches!(outcome, DevLinkOutcome::AlreadyLinked);
                    assert_eq!(is_already, already, "status {status}");
                }
                None => {
                    let err = got.unwrap_err().to_string();
                    assert!(err.contains(&status.to_string()), "status {status}: {err}");
                }
            }
        }
    }
}
