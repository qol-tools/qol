use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::FromRawFd;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::checkout;
use super::config::Config;
use super::takeover;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_BODY: usize = 64 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

pub fn serve(port: u16, config: Config) -> std::io::Result<()> {
    let listener = bind_task_runner_listener(port)?;
    eprintln!("[task-runner] listening on 127.0.0.1:{port}");
    let config = Arc::new(config);
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let config = config.clone();
        thread::spawn(move || handle_connection(stream, &config));
    }
    Ok(())
}

fn bind_task_runner_listener(port: u16) -> std::io::Result<TcpListener> {
    if let Some(fd) = qol_plugin_daemon::daemon::inherited_primary_port_fd() {
        return Ok(unsafe { TcpListener::from_raw_fd(fd) });
    }
    takeover::bind_with_takeover(port)
}

struct Request {
    method: String,
    path: String,
    host: Option<String>,
    origin: Option<String>,
    sec_fetch_site: Option<String>,
    body: Vec<u8>,
}

#[derive(Deserialize)]
struct CheckoutBody {
    #[serde(rename = "projectPath")]
    project_path: Option<String>,
    branch: Option<String>,
    app: Option<String>,
}

fn handle_connection(mut stream: TcpStream, config: &Config) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    route(&mut stream, &request, config);
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    parse_request(BufReader::new(stream.try_clone().ok()?))
}

fn parse_request<R: BufRead>(mut reader: R) -> Option<Request> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).ok()? == 0 {
        return None;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut content_length = 0;
    let mut host = None;
    let mut origin = None;
    let mut sec_fetch_site = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = header_value(trimmed, "content-length") {
            content_length = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = header_value(trimmed, "host") {
            host = Some(value.trim().to_string());
        } else if let Some(value) = header_value(trimmed, "origin") {
            origin = Some(value.trim().to_string());
        } else if let Some(value) = header_value(trimmed, "sec-fetch-site") {
            sec_fetch_site = Some(value.trim().to_string());
        }
    }

    if content_length > MAX_BODY {
        return None;
    }
    let mut body = vec![0; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok()?;
    }
    Some(Request {
        method,
        path,
        host,
        origin,
        sec_fetch_site,
        body,
    })
}

fn header_value(line: &str, name: &str) -> Option<String> {
    let (key, value) = line.split_once(':')?;
    key.trim()
        .eq_ignore_ascii_case(name)
        .then(|| value.to_string())
}

fn route<W: Write>(stream: &mut W, request: &Request, config: &Config) {
    if request.method == "POST" && is_blocked_mutation(request) {
        write_json(
            stream,
            403,
            &json!({ "error": "Cross-site request blocked" }),
        );
        return;
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") => write_json(stream, 200, &health()),
        ("POST", "/shutdown") => shutdown(stream),
        ("POST", "/checkout") => handle_checkout(stream, request, config),
        _ => write_json(stream, 404, &json!({ "error": "Not found" })),
    }
}

// The daemon is a privileged local endpoint reachable from the browser. Only
// same-origin loopback callers (the host probe, the extension's background
// worker) may mutate; a web page that tries to forge a checkout/shutdown sends
// a web Origin or cross-site fetch metadata, and a non-loopback Host signals
// DNS rebinding. Any of those is rejected.
fn is_blocked_mutation(request: &Request) -> bool {
    if let Some(host) = request.host.as_deref() {
        if !is_loopback_host(host) {
            return true;
        }
    }
    if let Some(origin) = request.origin.as_deref() {
        if !is_allowed_origin(origin) {
            return true;
        }
    }
    if let Some(site) = request.sec_fetch_site.as_deref() {
        if !matches!(site, "same-origin" | "same-site" | "none") {
            return true;
        }
    }
    false
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
        || host.starts_with("localhost:")
        || host.starts_with("127.0.0.1:")
        || host.starts_with("[::1]:")
}

fn is_allowed_origin(origin: &str) -> bool {
    let extension = [
        "chrome-extension://",
        "moz-extension://",
        "safari-web-extension://",
    ];
    if extension.iter().any(|scheme| origin.starts_with(*scheme)) {
        return true;
    }
    ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .iter()
        .any(|host| origin == *host || origin.starts_with(&format!("{host}:")))
}

fn health() -> Value {
    json!({ "status": "ok", "version": VERSION })
}

fn shutdown<W: Write>(stream: &mut W) {
    write_json(
        stream,
        200,
        &json!({ "status": "shutting-down", "version": VERSION }),
    );
    let _ = stream.flush();
    std::process::exit(0);
}

fn handle_checkout<W: Write>(stream: &mut W, request: &Request, config: &Config) {
    let body: CheckoutBody = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(_) => {
            write_json(
                stream,
                400,
                &json!({ "success": false, "error": "Invalid JSON" }),
            );
            return;
        }
    };
    let (Some(project_path), Some(branch)) = (body.project_path, body.branch) else {
        write_json(
            stream,
            400,
            &json!({ "success": false, "error": "Missing projectPath or branch" }),
        );
        return;
    };

    let checkout = match checkout::git_checkout(&project_path, &branch, config) {
        Ok(checkout) => checkout,
        Err(error) => {
            write_json(
                stream,
                error.status(),
                &json!({ "success": false, "error": error.message() }),
            );
            return;
        }
    };

    let app = body.app.unwrap_or_else(|| "idea".to_string());
    if let Err(error) = checkout::open_app(&app, &checkout.temp_path, config) {
        write_json(
            stream,
            500,
            &json!({
                "success": false,
                "error": format!(
                    "checked out {} at {} but could not open '{app}': {}",
                    checkout.branch,
                    checkout.temp_path,
                    error.message()
                ),
                "tempPath": checkout.temp_path,
            }),
        );
        return;
    }

    write_json(
        stream,
        200,
        &json!({
            "success": true,
            "branch": checkout.branch,
            "tempPath": checkout.temp_path,
            "projectPath": project_path,
        }),
    );
}

fn write_json<W: Write>(stream: &mut W, status: u16, body: &Value) {
    write_response(stream, status, "application/json", &body.to_string());
}

fn write_response<W: Write>(stream: &mut W, status: u16, content_type: &str, body: &str) {
    let mut response = format!("HTTP/1.1 {status} {}\r\n", reason_phrase(status));
    if !content_type.is_empty() {
        response.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    response.push_str(&format!("Content-Length: {}\r\n", body.len()));
    response.push_str("Connection: close\r\n\r\n");
    response.push_str(body);
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            host: Some("localhost".to_string()),
            origin: None,
            sec_fetch_site: None,
            body: body.as_bytes().to_vec(),
        }
    }

    fn route_to_string(request: &Request) -> String {
        let mut out = Vec::new();
        route(&mut out, request, &Config::defaults());
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn health_returns_ok_without_advertising_cors() {
        let response = route_to_string(&request("GET", "/health", ""));
        assert!(response.contains("200 OK"), "{response}");
        assert!(response.contains("\"status\":\"ok\""), "{response}");
        assert!(
            !response.contains("Access-Control-Allow-Origin"),
            "wildcard CORS must not be advertised: {response}"
        );
    }

    #[test]
    fn unknown_route_is_404() {
        assert!(route_to_string(&request("GET", "/nope", "")).contains("404 Not Found"));
    }

    #[test]
    fn checkout_with_invalid_json_is_400_failure() {
        let response = route_to_string(&request("POST", "/checkout", "not json"));
        assert!(response.contains("400 Bad Request"), "{response}");
        assert!(response.contains("\"success\":false"), "{response}");
    }

    #[test]
    fn checkout_missing_fields_is_400_failure() {
        let response = route_to_string(&request("POST", "/checkout", r#"{"branch":"x"}"#));
        assert!(response.contains("400 Bad Request"), "{response}");
        assert!(response.contains("Missing projectPath"), "{response}");
    }

    #[test]
    fn cross_site_post_is_rejected_before_any_work() {
        let mut req = request("POST", "/checkout", r#"{"projectPath":"/x","branch":"y"}"#);
        req.origin = Some("https://evil.example".to_string());
        let response = route_to_string(&req);
        assert!(response.contains("403 Forbidden"), "{response}");
    }

    #[test]
    fn blocked_mutation_allows_local_clients_and_rejects_web_callers() {
        let cases = [
            (Some("localhost:42720"), None, None, false),
            (Some("127.0.0.1:42720"), None, None, false),
            (
                Some("localhost"),
                Some("chrome-extension://abc"),
                Some("none"),
                false,
            ),
            (None, None, None, false),
            (Some("evil.example"), None, None, true),
            (Some("localhost"), Some("https://evil.example"), None, true),
            (Some("localhost"), None, Some("cross-site"), true),
        ];
        for (host, origin, site, expected) in cases {
            let req = Request {
                method: "POST".to_string(),
                path: "/checkout".to_string(),
                host: host.map(str::to_string),
                origin: origin.map(str::to_string),
                sec_fetch_site: site.map(str::to_string),
                body: Vec::new(),
            };
            assert_eq!(
                is_blocked_mutation(&req),
                expected,
                "host={host:?} origin={origin:?} site={site:?}"
            );
        }
    }

    #[test]
    fn header_value_matches_name_case_insensitively() {
        assert_eq!(
            header_value("Content-Length: 12", "content-length").as_deref(),
            Some(" 12")
        );
        assert_eq!(header_value("Host: localhost", "content-length"), None);
    }

    #[test]
    fn parse_request_extracts_method_path_host_and_body() {
        use std::io::Cursor;
        let raw = "POST /checkout HTTP/1.1\r\nHost: localhost:42720\r\nContent-Length: 9\r\n\r\n{\"a\":\"b\"}";
        let request = parse_request(Cursor::new(raw.as_bytes())).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/checkout");
        assert_eq!(request.host.as_deref(), Some("localhost:42720"));
        assert_eq!(request.body, b"{\"a\":\"b\"}".to_vec());
    }

    #[test]
    fn parse_request_rejects_oversized_body() {
        use std::io::Cursor;
        let raw = format!(
            "POST /checkout HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        assert!(parse_request(Cursor::new(raw.into_bytes())).is_none());
    }

    #[test]
    fn serves_health_over_a_real_tcp_socket() {
        use std::io::Read;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, &Config::defaults());
        });

        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();

        assert!(response.contains("200 OK"), "{response}");
        assert!(response.contains("\"status\":\"ok\""), "{response}");
        assert!(
            !response.contains("Access-Control-Allow-Origin"),
            "{response}"
        );
    }

    // Both tests below share QOL_TRAY_DAEMON_PORT_FD (an unsuffixed, single
    // process-wide name, unlike pointz's per-name port env vars), so they
    // need to be serialized against each other the same way
    // qol-plugin-daemon's own env-var tests are.
    fn port_fd_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    #[test]
    fn bind_task_runner_listener_adopts_an_inherited_fd() {
        use std::os::fd::{AsRawFd, IntoRawFd};

        let _lock = port_fd_env_lock();
        let pre_bound = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let expected_port = pre_bound.local_addr().unwrap().port();
        let fd = pre_bound.into_raw_fd();
        std::env::set_var(qol_conventions::ENV_DAEMON_PORT_FD, fd.to_string());

        let listener = bind_task_runner_listener(0);

        std::env::remove_var(qol_conventions::ENV_DAEMON_PORT_FD);
        let listener = listener.unwrap();
        assert_eq!(
            listener.local_addr().unwrap().port(),
            expected_port,
            "must adopt the pre-bound listener rather than binding its own"
        );
        assert_eq!(listener.as_raw_fd(), fd);
    }

    #[test]
    fn bind_task_runner_listener_binds_directly_when_env_var_absent() {
        let _lock = port_fd_env_lock();
        std::env::remove_var(qol_conventions::ENV_DAEMON_PORT_FD);

        let listener = bind_task_runner_listener(0);

        assert!(
            listener.is_ok(),
            "must fall back to binding its own port when nothing is pre-bound"
        );
    }
}
