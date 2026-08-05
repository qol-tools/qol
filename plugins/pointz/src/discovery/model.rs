use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct DiscoveryResponse {
    pub hostname: String,
    pub server_id: String,
    pub authentication: &'static str,
    pub pairing_open: bool,
}
