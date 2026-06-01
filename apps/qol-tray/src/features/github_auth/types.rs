use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GitHubCredentialSource {
    Oauth,
    LegacyToken,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GitHubCredentialRecord {
    pub(crate) access_token: String,
    pub(crate) source: GitHubCredentialSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) login: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct GitHubAuthStatus {
    pub(crate) connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<GitHubCredentialSource>,
    #[serde(default)]
    pub(crate) scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct GitHubAuthStartResponse {
    pub(crate) session_id: String,
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
    pub(crate) interval: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GitHubAuthSessionState {
    Pending,
    Authorized,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct GitHubAuthSessionResponse {
    pub(crate) state: GitHubAuthSessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}
