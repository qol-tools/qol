use anyhow::Result;

mod folder;
mod github;

use super::types::{
    SyncConnection, SyncProviderDefinition, SyncProviderFieldDefinition, SyncProviderFieldKey,
    SyncProviderFieldKind, SyncProviderFieldSection, SyncProviderKind,
};
use super::DEFAULT_PATH;

#[derive(Debug, Clone)]
pub(crate) struct RemoteDocument {
    pub(crate) revision: String,
    pub(crate) content: String,
}

#[derive(Debug)]
pub(crate) enum ProviderError {
    Auth(String),
    Conflict(String),
    #[expect(dead_code)]
    Invalid(String),
    Transport(String),
    Upstream(String),
}

impl ProviderError {
    pub(crate) fn is_transport(&self) -> bool {
        matches!(self, Self::Transport(_))
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(message) => write!(formatter, "{}", message),
            Self::Conflict(message) => write!(formatter, "{}", message),
            Self::Invalid(message) => write!(formatter, "{}", message),
            Self::Transport(message) => write!(formatter, "{}", message),
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
            Self::Github(connection) => format!("gist:{}", truncate_id(&connection.gist_id)),
            Self::Folder(connection) => folder::folder_sync_target_path(connection)
                .display()
                .to_string(),
        }
    }

    pub(crate) fn gist_id(&self) -> Option<&str> {
        if let Self::Github(connection) = self {
            return Some(connection.gist_id.as_str());
        }
        None
    }

    pub(crate) fn folder_path(&self) -> Option<&str> {
        if let Self::Folder(connection) = self {
            return Some(connection.folder_path.as_str());
        }
        None
    }

    pub(crate) fn path(&self) -> Option<&str> {
        if let Self::Folder(connection) = self {
            return Some(connection.path.as_str());
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
                    key: SyncProviderFieldKey::GistId,
                    label: "Gist ID".to_string(),
                    field_kind: SyncProviderFieldKind::Text,
                    section: SyncProviderFieldSection::Advanced,
                    placeholder: Some("Leave blank to auto-create".to_string()),
                    hint: Some("leave blank to auto-create a private gist".to_string()),
                    full_width: true,
                },
                pull_on_launch_field(),
                push_on_change_field(),
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
                    full_width: true,
                },
                SyncProviderFieldDefinition {
                    key: SyncProviderFieldKey::Path,
                    label: "Profile path".to_string(),
                    field_kind: SyncProviderFieldKind::Text,
                    section: SyncProviderFieldSection::Basic,
                    placeholder: Some(DEFAULT_PATH.to_string()),
                    hint: None,
                    full_width: true,
                },
                pull_on_launch_field(),
                push_on_change_field(),
            ],
        },
    ]
}

fn pull_on_launch_field() -> SyncProviderFieldDefinition {
    SyncProviderFieldDefinition {
        key: SyncProviderFieldKey::PullOnLaunch,
        label: "Pull on launch".to_string(),
        field_kind: SyncProviderFieldKind::Boolean,
        section: SyncProviderFieldSection::Advanced,
        placeholder: None,
        hint: None,
        full_width: false,
    }
}

fn push_on_change_field() -> SyncProviderFieldDefinition {
    SyncProviderFieldDefinition {
        key: SyncProviderFieldKey::PushOnChange,
        label: "Push on changes".to_string(),
        field_kind: SyncProviderFieldKind::Boolean,
        section: SyncProviderFieldSection::Advanced,
        placeholder: None,
        hint: None,
        full_width: false,
    }
}

pub(crate) async fn validate_github_token(token: &str) -> Result<()> {
    github::validate_github_token(token).await
}

pub(crate) async fn ensure_profile_gist(
    client: &reqwest::Client,
    token: &str,
) -> std::result::Result<String, ProviderError> {
    github::ensure_profile_gist(client, token).await
}

pub(crate) fn normalize_folder_path(folder_path: &str) -> Result<String> {
    folder::normalize_folder_path(folder_path)
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

fn truncate_id(id: &str) -> &str {
    if id.len() <= 8 {
        return id;
    }
    let mut end = 8;
    while !id.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    &id[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

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
                SyncProviderFieldKey::GistId,
                SyncProviderFieldKey::PullOnLaunch,
                SyncProviderFieldKey::PushOnChange,
            ]
        );
        assert_eq!(
            folder
                .fields
                .iter()
                .map(|field| field.key)
                .collect::<Vec<_>>(),
            vec![
                SyncProviderFieldKey::FolderPath,
                SyncProviderFieldKey::Path,
                SyncProviderFieldKey::PullOnLaunch,
                SyncProviderFieldKey::PushOnChange,
            ]
        );
    }

    #[test]
    fn normalize_path_expanded_cases() {
        let cases = [
            ("   ", Some(DEFAULT_PATH.to_string())),
            ("///foo.json", Some("foo.json".to_string())),
            ("a/b/c.json", Some("a/b/c.json".to_string())),
            ("foo/", None),
            ("foo/..", None),
            ("a..b.json", None),
            ("a\\b", None),
            (
                "valid-name_v2/config.json",
                Some("valid-name_v2/config.json".to_string()),
            ),
            ("UPPER.json", Some("UPPER.json".to_string())),
            ("has space", None),
            ("has!bang", None),
        ];
        for (input, expected) in cases {
            let actual = normalize_path(input).ok();
            assert_eq!(actual, expected, "input: {input:?}");
        }
    }

    #[test]
    fn truncate_id_handles_multibyte_boundary() {
        let input = "  ࠀ𐀀";
        assert_eq!(truncate_id(input), "  ࠀ");
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(200))]

            #[test]
            fn prop_safe_remote_path_rejects_empty(input in " *") {
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    assert!(!is_safe_remote_path(trimmed));
                }
            }

            #[test]
            fn prop_safe_remote_path_rejects_dotdot(
                prefix in "[a-z]{0,5}",
                suffix in "[a-z]{0,5}",
            ) {
                let path = format!("{prefix}..{suffix}");
                assert!(!is_safe_remote_path(&path));
            }

            #[test]
            fn prop_safe_remote_path_rejects_backslash(
                prefix in "[a-z]{1,5}",
                suffix in "[a-z]{1,5}",
            ) {
                let path = format!("{prefix}\\{suffix}");
                assert!(!is_safe_remote_path(&path));
            }

            #[test]
            fn prop_safe_remote_path_rejects_trailing_slash(
                base in "[a-z]{1,10}",
            ) {
                assert!(!is_safe_remote_path(&format!("{base}/")));
            }

            #[test]
            fn prop_safe_remote_path_accepts_valid_chars(
                path in "[a-zA-Z0-9/_.-]{1,30}"
            ) {
                let accepted = !path.contains("..")
                    && !path.contains('\\')
                    && !path.ends_with('/');
                assert_eq!(is_safe_remote_path(&path), accepted, "path: {path:?}");
            }

            #[test]
            fn prop_truncate_id_never_panics(input in ".*") {
                let result = truncate_id(&input);
                assert!(result.len() <= 8);
                assert!(input.starts_with(result));
            }

            #[test]
            fn prop_truncate_id_preserves_short_ascii(input in "[a-f0-9]{0,8}") {
                assert_eq!(truncate_id(&input), input.as_str());
            }
        }
    }
}
