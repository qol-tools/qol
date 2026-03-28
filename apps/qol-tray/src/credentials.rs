use anyhow::Result;
use std::path::PathBuf;

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
    let path = token_path()?;
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() {
        log::warn!("Token file is a symlink, rejecting: {:?}", path);
        return None;
    }

    let token = std::fs::read_to_string(&path).ok()?;
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    log::info!("Loaded GitHub token from {:?}", path);
    Some(token.to_string())
}

pub fn store_github_token(token: &str) -> Result<()> {
    let Some(path) = token_path() else {
        anyhow::bail!("Could not determine token path");
    };
    ensure_token_dir(&path)?;
    crate::file_io::atomic_write(&path, token.trim().as_bytes())?;
    log::info!("Stored GitHub token to {:?}", path);
    Ok(())
}

pub fn delete_github_token() -> Result<()> {
    let Some(path) = token_path() else {
        return Ok(());
    };
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

fn token_path() -> Option<PathBuf> {
    crate::paths::github_token_path().ok()
}

fn ensure_token_dir(path: &std::path::Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    Ok(())
}
