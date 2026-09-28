use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkFailure {
    Listener,
    Discovery,
    Authority,
    Cleanup,
    Task,
    RevisionExhausted,
    RuntimeUnavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ListenerStatus {
    Starting {},
    Listening { port: u16 },
    Closed {},
    Failed { error: NetworkFailure },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryStatus {
    Starting {},
    Ready {},
    Closed {},
    Failed { error: NetworkFailure },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressFamily {
    Ipv4Only,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkStatus {
    pub family: AddressFamily,
    pub listener: ListenerStatus,
    pub enrollment_listener: ListenerStatus,
    pub discovery: DiscoveryStatus,
    pub stopping: bool,
    pub failure: Option<NetworkFailure>,
}

impl Default for NetworkStatus {
    fn default() -> Self {
        Self {
            family: AddressFamily::Ipv4Only,
            listener: ListenerStatus::Starting {},
            enrollment_listener: ListenerStatus::Starting {},
            discovery: DiscoveryStatus::Starting {},
            stopping: false,
            failure: None,
        }
    }
}
