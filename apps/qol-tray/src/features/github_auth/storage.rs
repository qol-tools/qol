use anyhow::Result;
use std::path::PathBuf;

use super::types::{GitHubAuthStatus, GitHubCredentialRecord, GitHubCredentialSource};

pub(crate) fn github_auth_status() -> GitHubAuthStatus {
    let credential = load_github_credential();
    GitHubAuthStatus {
        connected: credential.is_some(),
        login: credential
            .as_ref()
            .and_then(|credential| credential.login.clone()),
        source: credential.as_ref().map(|credential| credential.source),
        scopes: credential
            .map(|credential| credential.scopes)
            .unwrap_or_default(),
    }
}

pub(crate) fn oauth_access_token() -> Option<String> {
    load_github_credential().map(|credential| credential.access_token)
}

pub(crate) fn oauth_scopes() -> Vec<String> {
    load_github_credential()
        .map(|credential| credential.scopes)
        .unwrap_or_default()
}

pub(crate) fn load_github_credential() -> Option<GitHubCredentialRecord> {
    let path = crate::paths::github_auth_path().ok()?;
    if let Some(content) = read_regular_file(&path) {
        if let Ok(record) = serde_json::from_str::<GitHubCredentialRecord>(&content) {
            if record.access_token.trim().is_empty() {
                return None;
            }
            return Some(record);
        }
    }
    load_legacy_github_credential()
}

pub(crate) fn store_github_credential(record: &GitHubCredentialRecord) -> Result<()> {
    let path = crate::paths::github_auth_path()?;
    ensure_parent_dir(&path)?;
    let content = serde_json::to_vec_pretty(record)?;
    crate::file_io::atomic_write(&path, &content)?;
    delete_legacy_github_token()?;
    Ok(())
}

pub(crate) fn delete_github_credential() -> Result<()> {
    delete_file_if_exists(crate::paths::github_auth_path().ok())?;
    delete_legacy_github_token()
}

fn load_legacy_github_credential() -> Option<GitHubCredentialRecord> {
    let path = crate::paths::github_token_path().ok()?;
    let content = read_regular_file(&path)?;
    let token = content.trim();
    if token.is_empty() {
        return None;
    }
    Some(GitHubCredentialRecord {
        access_token: token.to_string(),
        source: GitHubCredentialSource::LegacyToken,
        login: None,
        scopes: Vec::new(),
    })
}

fn read_regular_file(path: &std::path::Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        log::warn!("Rejecting symlink credential file: {}", path.display());
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn delete_legacy_github_token() -> Result<()> {
    delete_file_if_exists(crate::paths::github_token_path().ok())
}

fn delete_file_if_exists(path: Option<PathBuf>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path)?;
    Ok(())
}

fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct EnvGuard {
        home: Option<std::ffi::OsString>,
        xdg_config_home: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn new(root: &std::path::Path) -> Self {
            let home = std::env::var_os("HOME");
            let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
            let home_dir = root.join("home");
            let xdg_dir = root.join("xdg-config");
            std::fs::create_dir_all(&home_dir).unwrap();
            std::fs::create_dir_all(&xdg_dir).unwrap();
            std::env::set_var("HOME", &home_dir);
            std::env::set_var("XDG_CONFIG_HOME", &xdg_dir);
            Self {
                home,
                xdg_config_home,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.home {
                std::env::set_var("HOME", value);
            }
            if self.home.is_none() {
                std::env::remove_var("HOME");
            }
            if let Some(value) = &self.xdg_config_home {
                std::env::set_var("XDG_CONFIG_HOME", value);
            }
            if self.xdg_config_home.is_none() {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }
    }

    fn setup() -> (tokio::sync::MutexGuard<'static, ()>, TempDir, EnvGuard) {
        let guard = crate::test_support::env_lock().blocking_lock();
        let root = TempDir::new().unwrap();
        let env = EnvGuard::new(root.path());
        (guard, root, env)
    }

    #[test]
    fn stores_and_loads_json_credentials() {
        let (_guard, _root, _env) = setup();
        let record = GitHubCredentialRecord {
            access_token: "secret".to_string(),
            source: GitHubCredentialSource::Oauth,
            login: Some("octocat".to_string()),
            scopes: vec!["repo".to_string()],
        };

        store_github_credential(&record).unwrap();

        assert_eq!(load_github_credential(), Some(record));
    }

    #[test]
    fn falls_back_to_legacy_token_file() {
        let (_guard, _root, _env) = setup();
        let path = crate::paths::github_token_path().unwrap();
        ensure_parent_dir(&path).unwrap();
        std::fs::write(&path, "legacy-secret\n").unwrap();

        let credential = load_github_credential().unwrap();

        assert_eq!(credential.access_token, "legacy-secret");
        assert_eq!(credential.source, GitHubCredentialSource::LegacyToken);
    }

    #[test]
    fn oauth_access_token_returns_any_stored_token() {
        let (_guard, _root, _env) = setup();
        store_github_credential(&GitHubCredentialRecord {
            access_token: "legacy-secret".to_string(),
            source: GitHubCredentialSource::LegacyToken,
            login: None,
            scopes: Vec::new(),
        })
        .unwrap();

        assert_eq!(oauth_access_token(), Some("legacy-secret".to_string()));
    }
}
