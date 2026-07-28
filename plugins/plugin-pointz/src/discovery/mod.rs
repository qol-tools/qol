mod model;

use crate::config::ServerConfig;
use crate::network::{bind_udp_or_inherit, get_hostname};
use crate::security::CommandGate;
use anyhow::Result;
use model::DiscoveryResponse;
use std::sync::Arc;
use tokio::net::UdpSocket;

pub struct DiscoveryService {
    pub(crate) socket: UdpSocket,
    security: Arc<CommandGate>,
}

impl DiscoveryService {
    pub async fn new(security: Arc<CommandGate>) -> Result<Self> {
        let socket = bind_udp_or_inherit("discovery", ServerConfig::DISCOVERY_PORT).await?;
        socket.set_broadcast(true)?;
        Ok(Self { socket, security })
    }

    pub fn is_discovery_request(&self, request: &str) -> bool {
        request.trim() == ServerConfig::DISCOVER_MESSAGE
    }

    async fn send_response(&self, addr: std::net::SocketAddr) {
        let auth = self.security.discovery_auth();
        let response = DiscoveryResponse {
            hostname: get_hostname(),
            server_id: auth.server_id,
            authentication: "hmac-sha256-v1",
            pairing_secret: auth.pairing_secret,
        };
        let Ok(json) = serde_json::to_string(&response) else {
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
