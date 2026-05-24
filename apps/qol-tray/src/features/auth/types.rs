use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum AuthProvider {
    #[serde(rename = "github")]
    GitHub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "provider", content = "value")]
pub(crate) enum Scope {
    #[serde(rename = "github")]
    GitHub(GitHubScope),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GitHubScope {
    Repo,
}

impl Scope {
    pub(crate) fn provider(&self) -> AuthProvider {
        match self {
            Scope::GitHub(_) => AuthProvider::GitHub,
        }
    }

    pub(crate) fn wire_name(&self) -> &'static str {
        match self {
            Scope::GitHub(GitHubScope::Repo) => "repo",
        }
    }
}

pub(crate) struct ScopeRequirement {
    pub(crate) scope: Scope,
    pub(crate) reason: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AuthHealthIssue {
    InsufficientScope {
        provider: AuthProvider,
        missing: Vec<MissingScope>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MissingScope {
    pub(crate) scope: Scope,
    pub(crate) wire_name: &'static str,
    pub(crate) reason: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AuthHealth {
    pub(crate) issues: Vec<AuthHealthIssue>,
}
