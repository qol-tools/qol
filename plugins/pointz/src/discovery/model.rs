use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct DiscoveryResponse {
    pub hostname: String,
    pub server_id: String,
    pub authentication: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pairing_secret: Option<String>,
}
