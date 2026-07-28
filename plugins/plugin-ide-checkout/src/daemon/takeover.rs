use std::net::TcpListener;

pub fn bind_with_takeover(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}
