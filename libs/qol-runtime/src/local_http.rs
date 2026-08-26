use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

pub struct Client {
    port: u16,
    token: String,
    connect_timeout: Duration,
    io_timeout: Duration,
}

impl Client {
    pub fn new(port: u16, token: impl Into<String>) -> Self {
        Self {
            port,
            token: token.into(),
            connect_timeout: Duration::from_secs(2),
            io_timeout: Duration::from_secs(2),
        }
    }

    pub fn with_io_timeout(mut self, timeout: Duration) -> Self {
        self.io_timeout = timeout;
        self
    }

    pub fn request(&self, method: Method, path: &str, body: Option<&str>) -> io::Result<Response> {
        let request = self.build_request(method, path, body.unwrap_or(""), "close")?;
        let mut stream = self.connect()?;
        stream.write_all(request.as_bytes())?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw)?;
        let raw = String::from_utf8(raw)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        parse_response(&raw)
    }

    fn connect(&self) -> io::Result<TcpStream> {
        let stream = TcpStream::connect_timeout(&self.socket_addr()?, self.connect_timeout)?;
        stream.set_read_timeout(Some(self.io_timeout))?;
        stream.set_write_timeout(Some(self.io_timeout))?;
        stream.set_nodelay(true)?;
        Ok(stream)
    }

    fn socket_addr(&self) -> io::Result<SocketAddr> {
        let ip = qol_conventions::LOCAL_HOST
            .parse::<IpAddr>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Ok(SocketAddr::new(ip, self.port))
    }

    fn build_request(
        &self,
        method: Method,
        path: &str,
        body: &str,
        connection: &str,
    ) -> io::Result<String> {
        if !path.starts_with('/') || path.contains(['\r', '\n']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local HTTP path must be an absolute path without line breaks",
            ));
        }
        if self.token.is_empty() || self.token.contains(['\r', '\n']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local HTTP token must be non-empty and single-line",
            ));
        }
        Ok(format!(
            "{} {path} HTTP/1.1\r\nHost: {}:{}\r\nOrigin: http://{}:{}\r\n{}: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: {connection}\r\n\r\n{body}",
            method.as_str(),
            qol_conventions::LOCAL_HOST,
            self.port,
            qol_conventions::LOCAL_HOST,
            self.port,
            qol_conventions::HTTP_AUTH_HEADER,
            self.token,
            body.len()
        ))
    }
}

pub struct Session {
    client: Client,
    stream: Option<TcpStream>,
}

impl Session {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            stream: None,
        }
    }

    pub fn request(
        &mut self,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> io::Result<Response> {
        let reused = self.stream.is_some();
        match self.request_on_open_connection(method, path, body) {
            Ok(response) => Ok(response),
            Err(error) if reused && is_stale_connection(&error) => {
                self.stream = None;
                self.request_on_open_connection(method, path, body)
            }
            Err(error) => Err(error),
        }
    }

    fn request_on_open_connection(
        &mut self,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> io::Result<Response> {
        let request = self
            .client
            .build_request(method, path, body.unwrap_or(""), "keep-alive")?;
        let stream = match &mut self.stream {
            Some(stream) => stream,
            none => none.insert(self.client.connect()?),
        };
        stream.write_all(request.as_bytes())?;
        let response = read_kept_alive_response(stream);
        if response.is_err() {
            self.stream = None;
        }
        response
    }
}

/// True when the failure is the kept-alive socket having been closed under us,
/// which a fresh connection retries away. A timeout is not in this set: the
/// peer is simply slow, and retrying would silently pay the budget twice and
/// issue a second full request on the server.
fn is_stale_connection(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof
    )
}

fn read_kept_alive_response(stream: &mut TcpStream) -> io::Result<Response> {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(end) = find_header_end(&raw) {
            break end;
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "local HTTP connection closed before the response headers",
            ));
        }
        raw.extend_from_slice(&chunk[..read]);
    };
    let headers = String::from_utf8(raw[..header_end].to_vec())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let length = content_length(&headers).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "local HTTP response has no content-length",
        )
    })?;
    let body_start = header_end + 4;
    while raw.len() < body_start + length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "local HTTP connection closed inside the response body",
            ));
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    let raw = String::from_utf8(raw[..body_start + length].to_vec())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    parse_response(&raw)
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &str) -> Option<usize> {
    headers
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|line| line.split_once(':'))
        .and_then(|(_, value)| value.trim().parse().ok())
}

fn parse_response(raw: &str) -> io::Result<Response> {
    let status = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP status line"))?;
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    Ok(Response { status, body })
}

#[cfg(test)]
mod tests {
    use super::is_stale_connection;
    use std::io;

    /// A kept-alive socket closed under us is retried on a fresh connection.
    /// A timeout is not: retrying would pay the budget twice and issue a
    /// second full request on a peer that is merely slow.
    #[test]
    fn only_closed_connections_are_worth_retrying() {
        let retried = [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::NotConnected,
            io::ErrorKind::UnexpectedEof,
        ];
        for kind in retried {
            assert!(
                is_stale_connection(&io::Error::from(kind)),
                "{kind:?} must be retried on a fresh connection"
            );
        }
        let surfaced = [
            io::ErrorKind::TimedOut,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::InvalidData,
            io::ErrorKind::PermissionDenied,
        ];
        for kind in surfaced {
            assert!(
                !is_stale_connection(&io::Error::from(kind)),
                "{kind:?} must surface instead of being retried"
            );
        }
    }

    use super::{parse_response, Client, Method, Response};

    #[test]
    fn requests_share_the_authenticated_loopback_contract() {
        let port = qol_conventions::DEFAULT_PORT;
        let client = Client::new(port, "secret-token");
        let cases = [
            (Method::Get, "/api/config", "", "GET"),
            (Method::Post, "/api/actions/open", "{}", "POST"),
            (Method::Put, "/api/config", r#"{"enabled":true}"#, "PUT"),
        ];

        for (method, path, body, verb) in cases {
            let request = client.build_request(method, path, body, "close").unwrap();
            let (headers, actual_body) = request.split_once("\r\n\r\n").unwrap();

            assert!(headers.starts_with(&format!("{verb} {path} HTTP/1.1\r\n")));
            assert!(headers.contains(&format!("Host: {}:{port}\r\n", qol_conventions::LOCAL_HOST)));
            assert!(headers.contains(&format!(
                "Origin: http://{}:{port}\r\n",
                qol_conventions::LOCAL_HOST
            )));
            assert!(headers.contains(&format!(
                "{}: secret-token\r\n",
                qol_conventions::HTTP_AUTH_HEADER
            )));
            assert!(headers.contains(&format!("Content-Length: {}\r\n", body.len())));
            assert_eq!(actual_body, body);
        }
    }

    #[test]
    fn request_metadata_rejects_header_injection() {
        let cases = [
            ("/api/config\r\nInjected: true", "secret-token"),
            ("/api/config", "secret-token\r\nInjected: true"),
            ("api/config", "secret-token"),
            ("/api/config", ""),
        ];

        for (path, token) in cases {
            let error = Client::new(qol_conventions::DEFAULT_PORT, token)
                .build_request(Method::Get, path, "", "close")
                .unwrap_err();

            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn kept_alive_responses_are_delimited_by_content_length() {
        let cases = [
            ("content-length: 17\r\ncontent-type: a", Some(17)),
            ("Content-Length:  42 \r\n", Some(42)),
            ("content-type: application/json", None),
            ("content-length: notanumber", None),
        ];
        for (headers, expected) in cases {
            assert_eq!(
                super::content_length(headers),
                expected,
                "headers: {headers}"
            );
        }

        let cases = [
            (b"HTTP/1.1 200 OK\r\n\r\nbody".as_slice(), Some(15)),
            (b"HTTP/1.1 200 OK\r\npartial".as_slice(), None),
        ];
        for (raw, expected) in cases {
            assert_eq!(super::find_header_end(raw), expected);
        }
    }

    #[test]
    fn responses_parse_status_and_body() {
        let cases = [
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}",
                Response {
                    status: 200,
                    body: "{}".to_string(),
                },
            ),
            (
                "HTTP/1.1 422 Unprocessable Entity\r\n\r\nbad value",
                Response {
                    status: 422,
                    body: "bad value".to_string(),
                },
            ),
        ];

        for (raw, expected) in cases {
            assert_eq!(parse_response(raw).unwrap(), expected);
        }
    }

    #[test]
    fn malformed_status_is_rejected() {
        let error = parse_response("not HTTP\r\n\r\nbad").unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
