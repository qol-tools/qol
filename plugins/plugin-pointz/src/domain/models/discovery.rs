use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct DiscoveryResponse {
    pub hostname: String,
}

