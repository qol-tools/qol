use anyhow::Result;

pub trait CredentialProvider {
    fn github_bearer_token(&self) -> Option<String>;
}

pub struct LocalCredentialProvider;

impl CredentialProvider for LocalCredentialProvider {
    fn github_bearer_token(&self) -> Option<String> {
        load_github_token()
    }
}

pub fn github_bearer_token() -> Option<String> {
    LocalCredentialProvider.github_bearer_token()
}

pub fn load_github_token() -> Option<String> {
    crate::features::github_auth::oauth_access_token()
}

pub fn store_github_token(token: &str) -> Result<()> {
    crate::features::github_auth::store_github_credential(
        &crate::features::github_auth::GitHubCredentialRecord {
            access_token: token.trim().to_string(),
            source: crate::features::github_auth::GitHubCredentialSource::LegacyToken,
            login: None,
            scopes: Vec::new(),
        },
    )
}

pub fn delete_github_token() -> Result<()> {
    crate::features::github_auth::delete_github_credential()
}
