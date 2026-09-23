//! Headless config-path resolution for `qol sync`.
//!
//! The shared `qol-profile-sync` engine takes the profile root as plain
//! configuration; this module resolves that root plus the credential files
//! the same way qol-tray does, honoring the tray's path-root override in
//! tests and debug builds.

use anyhow::{Context, Result};
use std::path::PathBuf;

const GITHUB_AUTH_FILE: &str = ".github-auth.json";
const GITHUB_TOKEN_FILE: &str = ".github-token";

/// Test and debug builds honor the tray's path-root override so sync tests
/// can run against a tempdir without touching the real config directory.
#[cfg(any(test, debug_assertions))]
const TEST_PATH_ROOT_ENV: &str = "QOL_TRAY_TEST_PATH_ROOT";

pub(crate) fn config_dir() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(root) = std::env::var_os(TEST_PATH_ROOT_ENV) {
        return Ok(PathBuf::from(root)
            .join("config")
            .join(qol_config::NAMESPACE));
    }
    qol_config::config_dir().context("Could not determine config directory")
}

pub(crate) fn profile_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("profile"))
}

/// Whether the resolved profile root is the one a running tray owns. An
/// overridden path root is a private store, so a live tray has no say over it.
pub(crate) fn is_host_store() -> bool {
    #[cfg(any(test, debug_assertions))]
    if std::env::var_os(TEST_PATH_ROOT_ENV).is_some() {
        return false;
    }
    true
}

pub(crate) fn github_auth_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(GITHUB_AUTH_FILE))
}

pub(crate) fn github_token_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(GITHUB_TOKEN_FILE))
}

#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct EnvGuard {
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn new(root: &std::path::Path) -> Self {
            let previous = std::env::var_os(TEST_PATH_ROOT_ENV);
            std::env::set_var(TEST_PATH_ROOT_ENV, root);
            Self { previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(TEST_PATH_ROOT_ENV, previous);
                return;
            }
            std::env::remove_var(TEST_PATH_ROOT_ENV);
        }
    }

    #[test]
    fn profile_paths_compose_the_tray_layout() {
        let tmp = TempDir::new().unwrap();
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::new(tmp.path());

        assert!(profile_dir().unwrap().ends_with("qol-tray/profile"));
        assert!(github_auth_path()
            .unwrap()
            .ends_with("qol-tray/.github-auth.json"));
        assert!(github_token_path()
            .unwrap()
            .ends_with("qol-tray/.github-token"));
    }
}
