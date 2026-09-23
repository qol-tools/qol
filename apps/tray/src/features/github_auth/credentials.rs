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
