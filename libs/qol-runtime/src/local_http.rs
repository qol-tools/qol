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
        let request = self.build_request(method, path, body.unwrap_or(""))?;
        let mut stream = TcpStream::connect_timeout(&self.socket_addr()?, self.connect_timeout)?;
        stream.set_read_timeout(Some(self.io_timeout))?;
        stream.set_write_timeout(Some(self.io_timeout))?;
        stream.write_all(request.as_bytes())?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw)?;
        let raw = String::from_utf8(raw)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        parse_response(&raw)
    }

    fn socket_addr(&self) -> io::Result<SocketAddr> {
        let ip = qol_conventions::LOCAL_HOST
            .parse::<IpAddr>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Ok(SocketAddr::new(ip, self.port))
    }

    fn build_request(&self, method: Method, path: &str, body: &str) -> io::Result<String> {
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
            "{} {path} HTTP/1.1\r\nHost: {}:{}\r\nOrigin: http://{}:{}\r\n{}: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
            let request = client.build_request(method, path, body).unwrap();
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
                .build_request(Method::Get, path, "")
                .unwrap_err();

            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
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
