use anyhow::Result;

pub mod github;
pub mod memory;

pub use github::GitHubGistStore;
pub use memory::MemoryGistStore;

#[derive(Debug, Clone)]
pub struct GistMetadata {
    pub id: String,
    pub description: String,
    pub files: Vec<String>,
    pub updated_at: String,
    pub public: bool,
}

/// Backend abstraction for reading the authenticated user's GitHub gists.
///
/// Implementations are expected to be safe to share across threads and to be
/// cheap to clone references to. The `token` argument is a GitHub OAuth or
/// fine-grained PAT string passed as a `Bearer` credential by the real impl
/// and ignored by the in-memory impl.
///
/// Note on scopes: listing gists requires the `gist` OAuth scope. When the
/// scope is missing GitHub silently returns an empty array rather than a 4xx
/// response, so an empty result from [`GistStore::list`] is ambiguous between
/// "user has no gists" and "token lacks the gist scope".
#[async_trait::async_trait]
pub trait GistStore: Send + Sync {
    /// List all gists for the authenticated user.
    async fn list(&self, token: &str) -> Result<Vec<GistMetadata>>;

    /// Fetch the content of a specific file inside a gist by name.
    async fn fetch_file(&self, token: &str, gist_id: &str, file_name: &str) -> Result<String>;
}
