use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub fn post_to_daemon(path: &str, body: &str) -> std::io::Result<(u16, String)> {
    let addr: SocketAddr = (
        [127, 0, 0, 1],
        crate::features::plugin_store::DEFAULT_SERVER_PORT,
    )
        .into();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    let timeout = Some(Duration::from_secs(5));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);

    let token = read_token()?;
    let request = json_post_request(path, body, &token);
    stream.write_all(request.as_bytes())?;

    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    let response = String::from_utf8_lossy(&buf);
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    Ok((status, body))
}

pub fn browser_url(route: &str, port: u16) -> String {
    let token = read_token().ok();
    qol_conventions::local_hash_url_with_token(route, port, token.as_deref())
}

fn json_post_request(path: &str, body: &str, token: &str) -> String {
    let body = if body.is_empty() { "{}" } else { body };
    format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nX-Qol-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        port = crate::features::plugin_store::DEFAULT_SERVER_PORT,
        len = body.len(),
    )
}

fn read_token() -> std::io::Result<String> {
    crate::features::plugin_store::server::security::current_token().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "valid HTTP auth token is unavailable",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_request_always_contains_valid_json() {
        let cases = [
            ("", "{}"),
            (r#"{"route":"sessions"}"#, r#"{"route":"sessions"}"#),
        ];

        for (body, expected) in cases {
            let request = json_post_request("/api/test", body, "secret");
            let (_, actual) = request.split_once("\r\n\r\n").unwrap();

            assert_eq!(actual, expected);
            assert!(serde_json::from_str::<serde_json::Value>(actual).is_ok());
            assert!(request.contains(&format!("Content-Length: {}\r\n", expected.len())));
            assert!(request.contains("X-Qol-Token: secret\r\n"));
        }
    }

    #[test]
    fn browser_url_keeps_token_in_fragment() {
        let port = qol_conventions::DEFAULT_PORT;
        let url = qol_conventions::local_hash_url_with_token("shortcuts", port, Some("secret"));

        assert_eq!(
            url,
            format!("http://127.0.0.1:{port}/#shortcuts?qol_token=secret")
        );
    }
}
