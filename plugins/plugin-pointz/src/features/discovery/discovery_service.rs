use anyhow::Result;
use tokio::net::UdpSocket;
use crate::domain::config::ServerConfig;
use crate::domain::models::DiscoveryResponse;
use crate::utils::get_hostname;

pub struct DiscoveryService {
    pub(crate) socket: UdpSocket,
    pub(crate) response: DiscoveryResponse,
}

impl DiscoveryService {
    pub async fn new() -> Result<Self> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", ServerConfig::DISCOVERY_PORT)).await?;
        socket.set_broadcast(true)?;
        let response = DiscoveryResponse {
            hostname: get_hostname(),
        };
        Ok(Self { socket, response })
    }

    pub fn is_discovery_request(&self, request: &str) -> bool {
        request.trim() == ServerConfig::DISCOVER_MESSAGE
    }

    async fn send_response(&self, addr: std::net::SocketAddr) {
        let Ok(json) = serde_json::to_string(&self.response) else {
            return;
        };
        let _ = self.socket.send_to(json.as_bytes(), addr).await;
    }

    pub async fn run(&self) -> Result<()> {
        let mut buf = [0; ServerConfig::DISCOVERY_BUFFER_SIZE];

        loop {
            let Ok((size, addr)) = self.socket.recv_from(&mut buf).await else {
                continue;
            };

            let request = String::from_utf8_lossy(&buf[..size]);
            if !self.is_discovery_request(&request) {
                continue;
            }

            self.send_response(addr).await;
        }
    }
}

