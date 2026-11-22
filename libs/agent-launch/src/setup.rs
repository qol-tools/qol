use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use qol_plugin_api::profile::{active_profile_dir, CORE_SUBDIR, PROFILE_DIR};

use crate::account::ClaudeAccountSpec;

pub const FILE_NAME: &str = "agent-setup.json";
pub const VERSION: u64 = 1;
const CLAUDE_HARNESS: &str = "claude";

pub fn path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| path_in(&dir.join(PROFILE_DIR)))
}

pub fn path_in(profile_root: &Path) -> PathBuf {
    active_profile_dir(profile_root)
        .join(CORE_SUBDIR)
        .join(FILE_NAME)
}

#[derive(serde::Deserialize)]
struct SetupFile {
    version: Option<u64>,
    #[serde(default)]
    accounts: BTreeMap<String, SetupAccount>,
}

#[derive(serde::Deserialize)]
struct SetupAccount {
    harness: Option<String>,
    home: Option<String>,
    key: Option<String>,
    token_env: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClaudeAccount {
    pub(crate) name: String,
    pub(crate) home: String,
    pub(crate) spec: Option<ClaudeAccountSpec>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Setup {
    Missing,
    Unusable(String),
    Accounts(Vec<ClaudeAccount>),
}

pub(crate) fn read(path: &Path) -> Setup {
    match std::fs::read_to_string(path) {
        Ok(encoded) => match parse(&encoded) {
            Ok(accounts) => Setup::Accounts(accounts),
            Err(error) => Setup::Unusable(format!("{}: {error:#}", path.display())),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Setup::Missing,
        Err(error) => Setup::Unusable(format!("{}: {error}", path.display())),
    }
}

fn parse(encoded: &str) -> Result<Vec<ClaudeAccount>> {
    let file: SetupFile = serde_json::from_str(encoded)?;
    match file.version {
        Some(VERSION) => {}
        Some(version) => bail!("it is version {version}, and this qol reads version {VERSION}"),
        None => bail!("it has no version"),
    }
    Ok(file
        .accounts
        .into_iter()
        .filter(|(_, account)| account.harness.as_deref() == Some(CLAUDE_HARNESS))
        .filter_map(|(name, account)| {
            let home = account.home.filter(|home| !home.trim().is_empty())?;
            let spec = account
                .key
                .filter(|key| !key.trim().is_empty())
                .map(|key| ClaudeAccountSpec::new(key, account.token_env));
            Some(ClaudeAccount { name, home, spec })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(name: &str, home: &str, key: Option<&str>) -> ClaudeAccount {
        ClaudeAccount {
            name: name.to_owned(),
            home: home.to_owned(),
            spec: key.map(|key| ClaudeAccountSpec::new(key.to_owned(), None)),
        }
    }

    #[test]
    fn claude_accounts_are_read_and_other_harnesses_and_unknown_fields_are_left_alone() {
        let accounts = parse(
            r#"{
                "version": 1,
                "accounts": {
                    "clauded": {"harness": "claude", "home": "~/.claude", "purpose": "personal",
                                "marketplaces": ["qol-skills"], "plugins": {"a@b": true}},
                    "claudedd": {"harness": "claude", "home": "~/.claude-deepmind",
                                 "key": "claude-code-deepmind"},
                    "claudeapi": {"harness": "claude", "home": "/srv/claude", "key": "claude-api",
                                  "token_env": "ANTHROPIC_API_KEY"},
                    "pi": {"harness": "pi", "home": "~/.pi/agent", "key": "deepseek"},
                    "nameless": {"home": "~/.claude-x", "key": "x"},
                    "homeless": {"harness": "claude", "home": " ", "key": "y"}
                },
                "launch": {"claude": {"env": {"A": "1"}, "args": ["--x"]}},
                "marketplaces": {"qol-skills": {"source": {}}}
            }"#,
        )
        .unwrap();

        assert_eq!(
            accounts,
            [
                ClaudeAccount {
                    name: "claudeapi".to_owned(),
                    home: "/srv/claude".to_owned(),
                    spec: Some(ClaudeAccountSpec::new(
                        "claude-api".to_owned(),
                        Some("ANTHROPIC_API_KEY".to_owned())
                    )),
                },
                claude("clauded", "~/.claude", None),
                claude(
                    "claudedd",
                    "~/.claude-deepmind",
                    Some("claude-code-deepmind")
                ),
            ]
        );
    }

    #[test]
    fn a_file_without_a_known_version_is_not_used() {
        for (encoded, reason) in [
            (r#"{"accounts": {}}"#, "no version"),
            (r#"{"version": 2, "accounts": {}}"#, "version 2"),
            (r#"{"version": 1, "accounts": []}"#, "invalid type"),
            ("not json", "expected"),
        ] {
            let error = parse(encoded).unwrap_err();
            assert!(
                format!("{error:#}").contains(reason),
                "{encoded}: {error:#}"
            );
        }
    }

    #[test]
    fn a_missing_file_differs_from_an_unusable_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert_eq!(read(&path), Setup::Missing);

        std::fs::write(&path, r#"{"version": 9}"#).unwrap();
        assert!(matches!(read(&path), Setup::Unusable(reason) if reason.contains("version 9")));

        std::fs::write(&path, r#"{"version": 1}"#).unwrap();
        assert_eq!(read(&path), Setup::Accounts(Vec::new()));
    }

    #[test]
    fn the_file_sits_in_the_core_scope_of_the_active_profile() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            path_in(root.path()),
            root.path().join("default").join("core").join(FILE_NAME)
        );

        std::fs::write(root.path().join("active"), "work\n").unwrap();
        assert_eq!(
            path_in(root.path()),
            root.path().join("work").join("core").join(FILE_NAME)
        );
    }
}
