use crate::dev_console::TrayHandle;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

fn api_url(path: &str) -> String {
    format!("{}{path}", qol_conventions::local_base_url())
}

fn local_api_url(port: u16, route: &str) -> String {
    format!("http://{}:{port}/api{route}", qol_conventions::LOCAL_HOST)
}

pub(crate) fn website_url() -> String {
    format!("http://localhost:{}", qol_conventions::DEFAULT_PORT)
}
fn web_health_url() -> String {
    api_url("/")
}
fn dev_health_url() -> String {
    api_url("/api/dev/enabled")
}
fn dev_recompile_url() -> String {
    api_url("/api/dev/recompile-self")
}
fn dev_reload_url() -> String {
    api_url("/api/dev/reload")
}
fn dev_links_url() -> String {
    api_url("/api/dev/links")
}
fn dev_discovery_url() -> String {
    api_url("/api/dev/discovery-state")
}
fn plugin_health_url() -> String {
    api_url("/api/dev/plugin-health")
}
fn auth_health_url() -> String {
    api_url("/api/auth/health")
}
fn logs_health_url() -> String {
    api_url("/api/logs/entries")
}
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_INTERVAL: Duration = Duration::from_millis(250);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_INTERVAL: Duration = Duration::from_millis(100);
const HTTP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub(crate) enum DevLinkOutcome {
    Created,
    AlreadyLinked,
}

#[derive(Clone, PartialEq, serde::Deserialize)]
pub(crate) struct DevLink {
    #[serde(default)]
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) source: String,
    pub(crate) needs_rebuild: bool,
    pub(crate) rebuild_reason: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct DiscoveredPlugin {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) path: String,
}

#[derive(serde::Deserialize)]
struct DiscoveryStatePayload {
    #[serde(default)]
    plugins: Vec<DiscoveredPlugin>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkspacePlugin {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) path: String,
    pub(crate) linked: bool,
    pub(crate) needs_rebuild: bool,
    pub(crate) rebuild_reason: String,
}

pub(crate) enum LinkToggle {
    Linked,
    Unlinked,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct PluginHealthSnapshot {
    #[serde(default)]
    pub(crate) tick: u64,
    #[serde(default)]
    pub(crate) daemon_autostart_held: bool,
    #[serde(default)]
    pub(crate) plugins: Vec<PluginHealthRow>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PluginHealthRow {
    pub(crate) plugin_id: String,
    pub(crate) status: PluginDaemonStatus,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum PluginDaemonStatus {
    NotExpected,
    AutostartBlocked,
    Down {
        consecutive_failures: u32,
        suppressed: bool,
    },
    Probation {
        pid: u32,
        consecutive_failures: u32,
    },
    Stable {
        pid: u32,
    },
}

pub(crate) fn fetch_plugin_health() -> Result<PluginHealthSnapshot> {
    let url = plugin_health_url();
    let (status, body) = http_exchange("GET", &url, None)?;
    if status != 200 {
        bail!("GET {url} returned {status}");
    }
    serde_json::from_str(&body).context("invalid plugin health payload")
}

pub(crate) fn fetch_plugin_health_rows() -> Result<Option<Vec<PluginHealthRow>>> {
    let snapshot = fetch_plugin_health()?;
    if snapshot.tick == 0 || snapshot.daemon_autostart_held {
        return Ok(None);
    }
    Ok(Some(snapshot.plugins))
}

pub(crate) fn fetch_dev_links() -> Result<Vec<DevLink>> {
    let url = dev_links_url();
    let (status, body) = http_exchange("GET", &url, None)?;
    if status != 200 {
        bail!("GET {url} returned {status}");
    }
    serde_json::from_str(&body).context("invalid dev links payload")
}

pub(crate) fn fetch_discovered_plugins() -> Result<Vec<DiscoveredPlugin>> {
    let url = dev_discovery_url();
    let (status, body) = http_exchange("GET", &url, None)?;
    if status != 200 {
        bail!("GET {url} returned {status}");
    }
    let payload: DiscoveryStatePayload =
        serde_json::from_str(&body).context("invalid discovery payload")?;
    Ok(payload.plugins)
}

pub(crate) fn fetch_workspace_plugins() -> Result<Vec<WorkspacePlugin>> {
    let links = fetch_dev_links()?;
    let discovered = fetch_discovered_plugins().unwrap_or_default();
    Ok(merge_workspace_plugins(&links, &discovered))
}

pub(crate) fn merge_workspace_plugins(
    links: &[DevLink],
    discovered: &[DiscoveredPlugin],
) -> Vec<WorkspacePlugin> {
    let mut by_id: std::collections::HashMap<String, WorkspacePlugin> =
        std::collections::HashMap::new();
    for plugin in discovered {
        by_id.insert(
            plugin.id.clone(),
            WorkspacePlugin {
                id: plugin.id.clone(),
                name: plugin.name.clone(),
                version: String::new(),
                path: plugin.path.clone(),
                linked: false,
                needs_rebuild: false,
                rebuild_reason: String::new(),
            },
        );
    }
    for link in links {
        match by_id.get_mut(&link.id) {
            Some(existing) => {
                existing.version = link.version.clone();
                if !link.source.is_empty() {
                    existing.path = link.source.clone();
                }
                existing.linked = true;
                existing.needs_rebuild = link.needs_rebuild;
                existing.rebuild_reason = link.rebuild_reason.clone();
            }
            None => {
                by_id.insert(
                    link.id.clone(),
                    WorkspacePlugin {
                        id: link.id.clone(),
                        name: link.name.clone(),
                        version: link.version.clone(),
                        path: link.source.clone(),
                        linked: true,
                        needs_rebuild: link.needs_rebuild,
                        rebuild_reason: link.rebuild_reason.clone(),
                    },
                );
            }
        }
    }
    let mut rows: Vec<WorkspacePlugin> = by_id.into_values().collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    rows
}

pub(crate) fn toggle_dev_link(plugin: &WorkspacePlugin) -> Result<LinkToggle> {
    if plugin.linked {
        delete_dev_link(&plugin.id)?;
        Ok(LinkToggle::Unlinked)
    } else {
        if plugin.path.is_empty() {
            bail!("no source path known for {}", plugin.id);
        }
        post_dev_link(std::path::Path::new(&plugin.path))?;
        Ok(LinkToggle::Linked)
    }
}

pub(crate) fn delete_dev_link(id: &str) -> Result<()> {
    let url = format!("{}/{id}", dev_links_url());
    let status = http_request("DELETE", &url, None)?;
    if status / 100 == 2 {
        return Ok(());
    }
    bail!("unlink request failed with HTTP {status}")
}

pub(crate) fn wait_for_shutdown_best_effort() {
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    while Instant::now() < deadline {
        if !api_port_open() {
            return;
        }
        std::thread::sleep(SHUTDOWN_INTERVAL);
    }
}

pub(crate) fn wait_for_health_or_exit(child: &mut TrayHandle) -> Result<()> {
    let deadline = Instant::now() + HEALTH_TIMEOUT;
    while Instant::now() < deadline {
        if health_ok() {
            return Ok(());
        }
        if let Some(status) = child
            .try_wait()
            .context("failed to inspect qol-tray dev process")?
        {
            bail!("qol-tray dev process exited before server became healthy: {status}");
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
    http_get_bool(&dev_health_url()).unwrap_or(false)
}

pub(crate) fn web_ok() -> bool {
    http_get_ok(&web_health_url()).unwrap_or(false)
}

pub(crate) struct EndpointStatus {
    pub(crate) label: &'static str,
    pub(crate) url: String,
    pub(crate) ok: bool,
}

pub(crate) fn probe_endpoints() -> Vec<EndpointStatus> {
    let endpoints: [(&'static str, String); 4] = [
        ("website", web_health_url()),
        ("dev api", dev_health_url()),
        ("github", auth_health_url()),
        ("logs", logs_health_url()),
    ];
    endpoints
        .into_iter()
        .map(|(label, url)| EndpointStatus {
            label,
            ok: http_get_ok(&url).unwrap_or(false),
            url,
        })
        .collect()
}

pub(crate) fn post_recompile_current() -> Result<()> {
    post_recompile_body("{}")
}

pub(crate) fn post_reload_plugins() -> Result<()> {
    let status = http_request("POST", &dev_reload_url(), Some("{}"))?;
    if status / 100 == 2 {
        return Ok(());
    }
    bail!("plugin reload request failed with HTTP {status}");
}

pub(crate) fn post_promote_generation(port: u16) -> Result<()> {
    let url = local_api_url(port, qol_conventions::DEV_PROMOTE_GENERATION_ROUTE);
    let status = http_request("POST", &url, Some("{}"))?;
    if status / 100 == 2 {
        return Ok(());
    }
    bail!("generation promotion request failed with HTTP {status}");
}

fn post_recompile_body(body: &str) -> Result<()> {
    let status = http_request("POST", &dev_recompile_url(), Some(body))?;
    if status / 100 == 2 {
        return Ok(());
    }
    bail!("recompile request failed with HTTP {status}");
}

pub(crate) fn post_dev_link(plugin_dir: &std::path::Path) -> Result<DevLinkOutcome> {
    let path = plugin_dir.to_string_lossy().to_string();
    let body = json!({ "path": path }).to_string();
    let status = http_request("POST", &dev_links_url(), Some(&body))?;
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

fn http_get_bool(url: &str) -> Result<bool> {
    let (status, body) = http_exchange("GET", url, None)?;
    Ok(status == 200 && parse_json_bool(&body).unwrap_or(false))
}

fn http_request(method: &str, url: &str, body: Option<&str>) -> Result<u16> {
    Ok(http_exchange(method, url, body)?.0)
}

fn http_exchange(method: &str, url: &str, body: Option<&str>) -> Result<(u16, String)> {
    let target = HttpTarget::parse(url)?;
    let mut stream = connect_http_target(&target, HTTP_TIMEOUT)?;
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
    let status = parse_http_status(&response)?;
    Ok((status, response_body(&response)))
}

fn api_port_open() -> bool {
    let target = HttpTarget {
        host: "127.0.0.1".to_string(),
        port: qol_conventions::DEFAULT_PORT,
        path: "/".to_string(),
    };
    connect_http_target(&target, Duration::from_millis(100)).is_ok()
}

fn connect_http_target(target: &HttpTarget, timeout: Duration) -> Result<TcpStream> {
    let mut addrs = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve {}", target.host))?;
    let addr = addrs
        .next()
        .ok_or_else(|| anyhow!("no address for {}", target.host))?;
    let stream = TcpStream::connect_timeout(&addr, timeout)
        .with_context(|| format!("failed to connect to {}", target.host))?;
    Ok(stream)
}

fn response_body(response: &str) -> String {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default()
}

fn parse_json_bool(body: &str) -> Option<bool> {
    match body.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
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
    fn plugin_health_payload_parses_tagged_statuses() {
        let body = r#"{"tick":4,"process_pid":1,"role":"stable","bind_port":42700,
            "daemon_autostart_held":false,"generation_id":null,
            "plugins":[{"plugin_id":"plugin-foo","status":{"state":"down","consecutive_failures":5,"suppressed":true}}]}"#;
        let snapshot: PluginHealthSnapshot = serde_json::from_str(body).unwrap();
        assert_eq!(snapshot.tick, 4);
        assert_eq!(
            snapshot.plugins[0].status,
            PluginDaemonStatus::Down {
                consecutive_failures: 5,
                suppressed: true
            }
        );
    }

    #[test]
    fn parses_http_status_line() {
        assert_eq!(
            parse_http_status("HTTP/1.1 202 Accepted\r\n\r\n").unwrap(),
            202
        );
    }

    #[test]
    fn response_body_splits_headers_from_payload() {
        let cases = [
            ("HTTP/1.1 200 OK\r\nA: b\r\n\r\n[1,2]", "[1,2]"),
            ("HTTP/1.1 204 No Content\r\n\r\n", ""),
            ("HTTP/1.1 200 OK no separator", ""),
        ];
        for (response, expected) in cases {
            assert_eq!(response_body(response), expected, "response: {response:?}");
        }
    }

    #[test]
    fn dev_health_probe_uses_dev_enabled_metadata() {
        assert!(
            dev_health_url().ends_with("/api/dev/enabled"),
            "dev health must not depend on gated or heavy discovery endpoints"
        );
    }

    #[test]
    fn parses_json_bool_payload() {
        assert_eq!(parse_json_bool("true"), Some(true));
        assert_eq!(parse_json_bool(" false\n"), Some(false));
        assert_eq!(parse_json_bool(r#"{"ok":true}"#), None);
    }

    #[test]
    fn parses_dev_links_payload_ignoring_unknown_fields() {
        let payload = r#"[{"id":"a","name":"foo","source":"/a/b/c","needs_rebuild":true,"rebuild_reason":"Source changed","fingerprint":"x"}]"#;
        let links: Vec<DevLink> = serde_json::from_str(payload).unwrap();
        assert_eq!(links.len(), 1, "one link parsed");
        assert_eq!(links[0].name, "foo");
        assert!(links[0].needs_rebuild, "needs_rebuild carried through");
        assert_eq!(links[0].rebuild_reason, "Source changed");
    }

    fn discovered(id: &str, name: &str, path: &str) -> DiscoveredPlugin {
        DiscoveredPlugin {
            id: id.into(),
            name: name.into(),
            path: path.into(),
        }
    }

    fn link(id: &str, name: &str, source: &str, needs_rebuild: bool) -> DevLink {
        DevLink {
            id: id.into(),
            name: name.into(),
            version: "1.0.0".into(),
            source: source.into(),
            needs_rebuild,
            rebuild_reason: if needs_rebuild { "Source changed" } else { "" }.into(),
        }
    }

    #[test]
    fn merge_marks_unlinked_and_lets_links_override_path() {
        let discovered = [
            discovered("b", "Beta", "/ws/b"),
            discovered("a", "Alpha", "/clone/a"),
        ];
        let links = [link("a", "Alpha", "/ws/a", true)];
        let rows = merge_workspace_plugins(&links, &discovered);
        assert_eq!(rows.len(), 2, "one row per id");
        // sorted by name: Alpha, Beta
        assert_eq!(rows[0].id, "a");
        assert!(rows[0].linked, "a is linked");
        assert_eq!(
            rows[0].path, "/ws/a",
            "linked source overrides discovered path"
        );
        assert_eq!(rows[0].version, "1.0.0");
        assert!(rows[0].needs_rebuild);
        assert_eq!(rows[1].id, "b");
        assert!(!rows[1].linked, "b only discovered → linkable");
        assert_eq!(rows[1].path, "/ws/b");
    }

    #[test]
    fn merge_dedupes_duplicate_discovered_last_path_wins() {
        let discovered = [
            discovered("a", "A", "/first/a"),
            discovered("a", "A", "/second/a"),
        ];
        let rows = merge_workspace_plugins(&[], &discovered);
        assert_eq!(rows.len(), 1, "duplicate ids collapse");
        assert!(!rows[0].linked);
        assert_eq!(rows[0].path, "/second/a", "last discovered path wins");
    }

    #[test]
    fn merge_includes_links_absent_from_discovery() {
        let links = [link("x", "X", "/ws/x", false)];
        let rows = merge_workspace_plugins(&links, &[]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].linked);
        assert_eq!(rows[0].path, "/ws/x");
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
