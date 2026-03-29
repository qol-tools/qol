use anyhow::Result;

mod folder;
mod github;

use super::types::{
    SyncBranchList, SyncConnection, SyncProviderDefinition, SyncProviderFieldDefinition,
    SyncProviderFieldKey, SyncProviderFieldKind, SyncProviderFieldOptionsSource,
    SyncProviderFieldSection, SyncProviderKind,
};
use super::{DEFAULT_COMMIT_MESSAGE, DEFAULT_PATH};

#[derive(Debug, Clone)]
pub(crate) struct RemoteDocument {
    pub(crate) revision: String,
    pub(crate) content: String,
}

#[derive(Debug)]
pub(crate) enum ProviderError {
    Auth(String),
    Conflict(String),
    Invalid(String),
    Upstream(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(message) => write!(formatter, "{}", message),
            Self::Conflict(message) => write!(formatter, "{}", message),
            Self::Invalid(message) => write!(formatter, "{}", message),
            Self::Upstream(message) => write!(formatter, "{}", message),
        }
    }
}

impl std::error::Error for ProviderError {}

impl SyncConnection {
    pub(crate) fn provider_kind(&self) -> SyncProviderKind {
        match self {
            Self::Github(_) => SyncProviderKind::Github,
            Self::Folder(_) => SyncProviderKind::Folder,
        }
    }

    pub(crate) fn provider_label(&self) -> &'static str {
        match self {
            Self::Github(_) => "GitHub",
            Self::Folder(_) => "Folder",
        }
    }

    pub(crate) fn target_summary(&self) -> String {
        match self {
            Self::Github(connection) => connection.repo_url.clone(),
            Self::Folder(connection) => folder::folder_sync_target_path(connection)
                .display()
                .to_string(),
        }
    }

    pub(crate) fn repo_url(&self) -> Option<&str> {
        if let Self::Github(connection) = self {
            return Some(connection.repo_url.as_str());
        }
        None
    }

    pub(crate) fn folder_path(&self) -> Option<&str> {
        if let Self::Folder(connection) = self {
            return Some(connection.folder_path.as_str());
        }
        None
    }

    pub(crate) fn branch(&self) -> Option<&str> {
        if let Self::Github(connection) = self {
            return Some(connection.branch.as_str());
        }
        None
    }

    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Github(connection) => connection.path.as_str(),
            Self::Folder(connection) => connection.path.as_str(),
        }
    }

    pub(crate) fn commit_message(&self) -> Option<&str> {
        if let Self::Github(connection) = self {
            return Some(connection.commit_message.as_str());
        }
        None
    }

    pub(crate) fn pull_on_launch(&self) -> bool {
        match self {
            Self::Github(connection) => connection.pull_on_launch,
            Self::Folder(connection) => connection.pull_on_launch,
        }
    }

    pub(crate) fn push_on_change(&self) -> bool {
        match self {
            Self::Github(connection) => connection.push_on_change,
            Self::Folder(connection) => connection.push_on_change,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::Github(connection) => github::validate_connection(connection),
            Self::Folder(connection) => folder::validate_connection(connection),
        }
    }

    pub(crate) async fn fetch_remote_document(
        &self,
        client: &reqwest::Client,
        github_token: Option<&str>,
    ) -> std::result::Result<Option<RemoteDocument>, ProviderError> {
        match self {
            Self::Github(connection) => {
                let token = github::resolve_github_token(github_token)
                    .map_err(|error| ProviderError::Auth(error.to_string()))?;
                github::fetch_remote_document(client, connection, &token).await
            }
            Self::Folder(connection) => folder::fetch_remote_document(connection),
        }
    }

    pub(crate) async fn push_remote_document(
        &self,
        client: &reqwest::Client,
        content: &str,
        remote_revision: Option<&str>,
        github_token: Option<&str>,
    ) -> std::result::Result<String, ProviderError> {
        match self {
            Self::Github(connection) => {
                let token = github::resolve_github_token(github_token)
                    .map_err(|error| ProviderError::Auth(error.to_string()))?;
                github::push_remote_document(client, connection, &token, content, remote_revision)
                    .await
            }
            Self::Folder(connection) => {
                folder::push_remote_document(connection, content, remote_revision)
            }
        }
    }
}

pub(crate) fn sync_provider_definitions() -> Vec<SyncProviderDefinition> {
    vec![
        SyncProviderDefinition {
            kind: SyncProviderKind::Github,
            label: "GitHub".to_string(),
            fields: vec![
                SyncProviderFieldDefinition {
                    key: SyncProviderFieldKey::RepoUrl,
                    label: "Repo URL".to_string(),
                    field_kind: SyncProviderFieldKind::Text,
                    section: SyncProviderFieldSection::Basic,
                    placeholder: Some("https://github.com/owner/repo".to_string()),
                    hint: None,
                    options_source: None,
                    full_width: true,
                },
                SyncProviderFieldDefinition {
                    key: SyncProviderFieldKey::Token,
                    label: "GitHub PAT".to_string(),
                    field_kind: SyncProviderFieldKind::Password,
                    section: SyncProviderFieldSection::Basic,
                    placeholder: Some("Paste PAT".to_string()),
                    hint: Some("leave blank to keep the stored PAT".to_string()),
                    options_source: None,
                    full_width: true,
                },
                SyncProviderFieldDefinition {
                    key: SyncProviderFieldKey::Path,
                    label: "Profile path".to_string(),
                    field_kind: SyncProviderFieldKind::Text,
                    section: SyncProviderFieldSection::Basic,
                    placeholder: Some(DEFAULT_PATH.to_string()),
                    hint: None,
                    options_source: None,
                    full_width: true,
                },
                SyncProviderFieldDefinition {
                    key: SyncProviderFieldKey::Branch,
                    label: "Branch".to_string(),
                    field_kind: SyncProviderFieldKind::Select,
                    section: SyncProviderFieldSection::Advanced,
                    placeholder: None,
                    hint: None,
                    options_source: Some(SyncProviderFieldOptionsSource::GithubBranches),
                    full_width: false,
                },
                SyncProviderFieldDefinition {
                    key: SyncProviderFieldKey::CommitMessage,
                    label: "Commit message".to_string(),
                    field_kind: SyncProviderFieldKind::Text,
                    section: SyncProviderFieldSection::Advanced,
                    placeholder: Some(DEFAULT_COMMIT_MESSAGE.to_string()),
                    hint: None,
                    options_source: None,
                    full_width: false,
                },
            ],
        },
        SyncProviderDefinition {
            kind: SyncProviderKind::Folder,
            label: "Folder".to_string(),
            fields: vec![
                SyncProviderFieldDefinition {
                    key: SyncProviderFieldKey::FolderPath,
                    label: "Folder path".to_string(),
                    field_kind: SyncProviderFieldKind::Text,
                    section: SyncProviderFieldSection::Basic,
                    placeholder: Some("Absolute folder path".to_string()),
                    hint: None,
                    options_source: None,
                    full_width: true,
                },
                SyncProviderFieldDefinition {
                    key: SyncProviderFieldKey::Path,
                    label: "Profile path".to_string(),
                    field_kind: SyncProviderFieldKind::Text,
                    section: SyncProviderFieldSection::Basic,
                    placeholder: Some(DEFAULT_PATH.to_string()),
                    hint: None,
                    options_source: None,
                    full_width: true,
                },
            ],
        },
    ]
}

pub(crate) async fn validate_github_token(token: &str) -> Result<()> {
    github::validate_github_token(token).await
}

pub(crate) async fn fetch_github_default_branch(
    client: &reqwest::Client,
    repo_url: &str,
    token: &str,
) -> std::result::Result<String, ProviderError> {
    github::fetch_github_default_branch(client, repo_url, token).await
}

pub(crate) async fn fetch_github_branches(
    client: &reqwest::Client,
    repo_url: &str,
    token: &str,
) -> std::result::Result<SyncBranchList, ProviderError> {
    github::fetch_github_branches(client, repo_url, token).await
}

pub(crate) fn normalize_repo_url(repo_url: &str) -> Result<String> {
    github::normalize_repo_url(repo_url)
}

pub(crate) fn normalize_folder_path(folder_path: &str) -> Result<String> {
    folder::normalize_folder_path(folder_path)
}

pub(crate) fn normalize_requested_branch(branch: &str) -> Result<Option<String>> {
    github::normalize_requested_branch(branch)
}

pub(crate) fn normalize_path(path: &str) -> Result<String> {
    let trimmed = path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(DEFAULT_PATH.to_string());
    }
    if !is_safe_remote_path(trimmed) {
        anyhow::bail!("Invalid remote path");
    }
    Ok(trimmed.to_string())
}

pub(crate) fn normalize_commit_message(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return DEFAULT_COMMIT_MESSAGE.to_string();
    }
    trimmed.to_string()
}

fn is_safe_remote_path(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value.contains("..") || value.contains('\\') || value.ends_with('/') {
        return false;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' || ch == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::profile::sync::{
        SyncProviderFieldKey, SyncProviderFieldOptionsSource, SyncProviderKind,
    };

    #[test]
    fn parse_github_repo_accepts_supported_url_shapes() {
        let cases = vec![
            (
                "https://github.com/example-owner/example.repo",
                Some(("example-owner", "example.repo")),
            ),
            (
                "https://github.com/example-owner/example.repo.git",
                Some(("example-owner", "example.repo")),
            ),
            (
                "git@github.com:example-owner/example.repo.git",
                Some(("example-owner", "example.repo")),
            ),
            (
                "ssh://git@ssh.github.com:443/example-owner/example.repo.git",
                Some(("example-owner", "example.repo")),
            ),
            (
                "ssh://git@github.com/example-owner/example.repo.git",
                Some(("example-owner", "example.repo")),
            ),
            ("git@host-alias:example-owner/example.repo.git", None),
            ("ssh://git@host-alias/example-owner/example.repo.git", None),
            ("https://example.com/example-owner/example.repo", None),
        ];

        for (input, expected) in cases {
            let parsed = github::parse_github_repo(input).ok();
            let expected = expected.map(|(owner, repo)| (owner.to_string(), repo.to_string()));
            assert_eq!(parsed, expected, "input: {input}");
        }
    }

    #[test]
    fn normalize_path_cases() {
        let cases = vec![
            ("", Some(DEFAULT_PATH.to_string())),
            ("qol/profile.json", Some("qol/profile.json".to_string())),
            ("/qol/profile.json", Some("qol/profile.json".to_string())),
            ("../bad.json", None),
            ("qol\\bad.json", None),
        ];

        for (input, expected) in cases {
            let actual = normalize_path(input).ok();
            assert_eq!(actual, expected, "input: {input}");
        }
    }

    #[test]
    fn sync_provider_definitions_expose_provider_owned_fields() {
        let providers = sync_provider_definitions();
        let github = providers
            .iter()
            .find(|provider| provider.kind == SyncProviderKind::Github)
            .unwrap();
        let folder = providers
            .iter()
            .find(|provider| provider.kind == SyncProviderKind::Folder)
            .unwrap();

        assert_eq!(
            github
                .fields
                .iter()
                .map(|field| field.key)
                .collect::<Vec<_>>(),
            vec![
                SyncProviderFieldKey::RepoUrl,
                SyncProviderFieldKey::Token,
                SyncProviderFieldKey::Path,
                SyncProviderFieldKey::Branch,
                SyncProviderFieldKey::CommitMessage,
            ]
        );
        assert_eq!(
            folder
                .fields
                .iter()
                .map(|field| field.key)
                .collect::<Vec<_>>(),
            vec![SyncProviderFieldKey::FolderPath, SyncProviderFieldKey::Path]
        );
        assert_eq!(
            github
                .fields
                .iter()
                .find(|field| field.key == SyncProviderFieldKey::Branch)
                .and_then(|field| field.options_source),
            Some(SyncProviderFieldOptionsSource::GithubBranches)
        );
    }
}
