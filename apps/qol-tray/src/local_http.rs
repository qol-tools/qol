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

    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        port = crate::features::plugin_store::DEFAULT_SERVER_PORT,
        len = body.len(),
    );
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
