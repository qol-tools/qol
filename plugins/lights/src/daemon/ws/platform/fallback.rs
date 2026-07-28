use std::net::TcpListener;

pub(crate) fn bind_listener(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(format!("127.0.0.1:{port}"))
}
