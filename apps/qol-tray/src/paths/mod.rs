use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) mod file_io;
mod platform;

const ACTIVE_INSTALL_ID_FILE: &str = qol_config::ACTIVE_INSTALL_ID_FILE;
const ACTIVE_PROFILE_FILE: &str = "active";
pub const DEFAULT_PROFILE_NAME: &str = "default";
#[cfg(any(test, debug_assertions))]
const TEST_PATH_ROOT_ENV: &str = "QOL_TRAY_TEST_PATH_ROOT";

pub use qol_conventions::STATE_SOCKET_PATH;

#[cfg(test)]
thread_local! {
    static TEST_PATH_ROOTS: std::cell::RefCell<Vec<PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) struct TestPathRootGuard;

#[cfg(all(test, feature = "dev"))]
pub(crate) struct TestEnvPathRootGuard {
    previous: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl Drop for TestPathRootGuard {
    fn drop(&mut self) {
        TEST_PATH_ROOTS.with(|roots| {
            roots.borrow_mut().pop();
        });
    }
}

#[cfg(all(test, feature = "dev"))]
impl Drop for TestEnvPathRootGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var(TEST_PATH_ROOT_ENV, previous);
            return;
        }
        std::env::remove_var(TEST_PATH_ROOT_ENV);
    }
}

#[cfg(test)]
pub(crate) fn push_test_path_root(root: &Path) -> TestPathRootGuard {
    TEST_PATH_ROOTS.with(|roots| {
        roots.borrow_mut().push(root.to_path_buf());
    });
    TestPathRootGuard
}

#[cfg(all(test, feature = "dev"))]
pub(crate) fn push_test_env_path_root(root: &Path) -> TestEnvPathRootGuard {
    let previous = std::env::var_os(TEST_PATH_ROOT_ENV);
    std::env::set_var(TEST_PATH_ROOT_ENV, root);
    TestEnvPathRootGuard { previous }
}

#[cfg(test)]
fn test_path_root() -> Option<PathBuf> {
    TEST_PATH_ROOTS
        .with(|roots| roots.borrow().last().cloned())
        .or_else(test_env_path_root)
}

#[cfg(all(not(test), debug_assertions))]
fn test_path_root() -> Option<PathBuf> {
    test_env_path_root()
}

#[cfg(any(test, debug_assertions))]
fn test_env_path_root() -> Option<PathBuf> {
    std::env::var_os(TEST_PATH_ROOT_ENV).map(PathBuf::from)
}

pub fn is_safe_path_component(s: &str) -> bool {
    qol_plugin_api::manifest::is_valid_safe_identifier(s)
}

fn legacy_config_dir() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(root) = test_path_root() {
        return Ok(root.join("config").join(qol_config::NAMESPACE));
    }

    qol_config::config_dir().context("Could not determine config directory")
}

pub fn shared_config_dir() -> Result<PathBuf> {
    legacy_config_dir()
}

pub(crate) fn base_data_dir() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(root) = test_path_root() {
        return Ok(root.join("data").join(qol_config::NAMESPACE));
    }

    qol_config::data_dir().context("Could not determine local data directory")
}

pub fn installs_dir() -> Result<PathBuf> {
    base_data_dir().map(|p| p.join("installs"))
}

fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn validated_install_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    valid_install_id(trimmed).then(|| trimmed.to_string())
}

fn active_install_id_path() -> Result<PathBuf> {
    base_data_dir().map(|p| p.join(ACTIVE_INSTALL_ID_FILE))
}

fn install_id_from_active_file() -> Option<String> {
    validated_install_id(&fs::read_to_string(active_install_id_path().ok()?).ok()?)
}

pub fn set_active_install_id(install_id: &str) -> Result<()> {
    if !valid_install_id(install_id) {
        return Err(anyhow!("invalid install id"));
    }
    let path = active_install_id_path()?;
    file_io::ensure_parent_dir(&path)?;
    fs::write(&path, format!("{}\n", install_id))
        .with_context(|| format!("Failed to write active install marker {}", path.display()))
}

pub fn has_active_install_id() -> bool {
    install_id_from_active_file().is_some()
}

pub fn plugins_dir() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("plugins"))
}

pub fn profile_dir() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("profile"))
}

pub fn current_os_subdir() -> &'static str {
    platform::os_bucket()
}

pub(crate) fn active_profile_marker_path() -> Result<PathBuf> {
    profile_dir().map(|p| p.join(ACTIVE_PROFILE_FILE))
}

pub fn active_profile_name() -> String {
    active_profile_marker_path()
        .ok()
        .and_then(|path| fs::read_to_string(&path).ok())
        .map(|raw| raw.trim().to_string())
        .filter(|name| is_safe_path_component(name))
        .unwrap_or_else(|| DEFAULT_PROFILE_NAME.to_string())
}

pub fn profile_manifest_path() -> Result<PathBuf> {
    active_scope_store().map(|s| s.manifest_path())
}

pub fn profile_sync_config_path() -> Result<PathBuf> {
    profile_dir().map(|p| p.join("sync.json"))
}

pub fn profile_os_dir() -> Result<PathBuf> {
    active_scope_store().map(|s| s.os_dir())
}

pub fn profile_plugins_lock_path() -> Result<PathBuf> {
    active_scope_store().map(|s| s.plugins_lock_path())
}

pub fn profile_plugin_configs_dir() -> Result<PathBuf> {
    active_scope_store().map(|s| s.core_plugin_configs_dir())
}

pub fn hotkeys_path() -> Result<PathBuf> {
    active_scope_store().map(|s| s.hotkeys_path())
}

pub fn shortcuts_path() -> Result<PathBuf> {
    active_scope_store().map(|s| s.shortcuts_path())
}

pub fn task_runner_config_path() -> Result<PathBuf> {
    active_scope_store().map(|s| s.task_runner_path())
}

fn active_scope_store() -> Result<crate::features::profile::ProfileScopeStore> {
    crate::features::profile::ProfileScopeStore::from_active()
}

pub fn github_token_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join(".github-token"))
}

pub fn github_auth_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join(".github-auth.json"))
}

pub fn http_auth_token_path() -> Result<PathBuf> {
    qol_config::http_auth_token_path().context("Could not determine HTTP auth token path")
}

pub fn sync_dir() -> Result<PathBuf> {
    active_scope_store().map(|s| s.device_sync_dir())
}

pub fn sync_state_path() -> Result<PathBuf> {
    active_scope_store().map(|s| s.sync_state_path())
}

pub fn sync_backups_dir() -> Result<PathBuf> {
    active_scope_store().map(|s| s.sync_backups_dir())
}

pub fn suppressed_errors_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("suppressed-errors.json"))
}

pub fn plugin_cache_path() -> Result<PathBuf> {
    Ok(runtime_cache_dir().join("plugin-cache.json"))
}

#[cfg(feature = "dev")]
pub fn dev_config_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("dev/config.json"))
}

pub fn mode_config_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("mode.json"))
}

pub fn runtime_gpui_config_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("runtime/gpui.json"))
}

pub fn runtime_dir() -> PathBuf {
    #[cfg(any(test, debug_assertions))]
    if let Some(root) = test_path_root() {
        return root
            .join("data")
            .join(qol_config::NAMESPACE)
            .join("runtime");
    }

    qol_config::runtime_dir().unwrap_or_else(|| PathBuf::from(qol_conventions::RUNTIME_DIR_PATH))
}

pub fn runtime_pids_dir() -> PathBuf {
    runtime_dir().join("pids")
}

pub fn runtime_cache_dir() -> PathBuf {
    runtime_dir().join("cache")
}

pub fn init_runtime_dirs() -> Result<()> {
    init_runtime_dirs_at(&runtime_dir())
}

fn init_runtime_dirs_at(base: &Path) -> Result<()> {
    qol_fs::recreate_private_dir(base)
        .with_context(|| format!("Failed to recreate runtime dir {}", base.display()))?;
    for subdir in ["pids", "cache", "sockets"] {
        qol_fs::create_private_dir(&base.join(subdir))
            .with_context(|| format!("Failed to create runtime subdir {}", subdir))?;
    }
    Ok(())
}

pub fn open_url(url: &str) -> Result<()> {
    qol_apps::desktop_integration::open_with_default_app(url)?;
    Ok(())
}

#[cfg(feature = "dev")]
pub fn repo_root_from_manifest_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    qol_workspace::workspace_root_from(&manifest).unwrap_or(manifest)
}

#[cfg(feature = "dev")]
pub fn default_workspace_root() -> Option<PathBuf> {
    let config_dir = shared_config_dir().ok()?;
    let configured = qol_dev_build::tray::read_default_workspace(&config_dir)?;
    Some(
        qol_workspace::workspace_root_from(Path::new(&configured))
            .unwrap_or_else(|_| PathBuf::from(configured)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(feature = "dev")]
    #[test]
    fn default_workspace_root_follows_recorded_workspace_not_manifest_dir() {
        let tmp = TempDir::new().unwrap();
        let _guard = push_test_path_root(tmp.path());
        let base = tmp.path().join("base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(
            base.join("Cargo.toml"),
            "[workspace]\nmembers = [\"apps/qol-tray\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        let config_dir = shared_config_dir().unwrap();
        std::fs::create_dir_all(config_dir.join("dev")).unwrap();
        std::fs::write(
            config_dir.join("dev").join("default-workspace.txt"),
            format!("{}\n", base.display()),
        )
        .unwrap();

        assert_eq!(default_workspace_root(), Some(base.clone()));
    }

    #[cfg(feature = "dev")]
    #[test]
    fn default_workspace_root_is_none_without_a_recorded_workspace() {
        let tmp = TempDir::new().unwrap();
        let _guard = push_test_path_root(tmp.path());

        assert_eq!(default_workspace_root(), None);
    }

    #[test]
    fn paths_have_correct_suffixes() {
        let tmp = TempDir::new().unwrap();
        let _guard = push_test_path_root(tmp.path());

        let cases: Vec<(Result<PathBuf>, &str)> = vec![
            (plugins_dir(), "plugins"),
            (profile_dir(), "profile"),
            (profile_plugins_lock_path(), "plugins.lock.json"),
            (profile_plugin_configs_dir(), "plugin-configs"),
            (hotkeys_path(), "hotkeys.json"),
            (shortcuts_path(), "shortcuts.json"),
            (task_runner_config_path(), "task-runner.json"),
            (github_token_path(), ".github-token"),
            (github_auth_path(), ".github-auth.json"),
            (sync_dir(), "sync"),
            (sync_state_path(), "state.json"),
            (sync_backups_dir(), "backups"),
        ];

        for (result, expected_suffix) in cases {
            let path = result.unwrap();
            assert!(
                path.ends_with(expected_suffix),
                "path {:?} should end with {}",
                path,
                expected_suffix
            );
            if expected_suffix == "plugins" {
                assert!(path.to_string_lossy().contains("qol-tray"));
            }
        }
    }

    #[test]
    fn runtime_dir_is_user_scoped() {
        let dir = runtime_dir();
        assert!(dir.ends_with("qol-tray/runtime"));
    }

    #[test]
    fn runtime_subdirs_have_correct_suffixes() {
        let cases = [(runtime_pids_dir(), "pids"), (runtime_cache_dir(), "cache")];
        for (path, suffix) in cases {
            assert!(
                path.ends_with(suffix),
                "path {:?} should end with {}",
                path,
                suffix
            );
        }
    }

    #[test]
    fn plugin_cache_path_is_under_runtime() {
        let path = plugin_cache_path().unwrap();
        assert!(
            path.starts_with(runtime_dir()),
            "cache path {:?} should be under the private runtime directory",
            path
        );
    }

    #[test]
    fn init_runtime_dirs_creates_fresh_structure() {
        let temp = tempfile::TempDir::new().unwrap();
        let test_dir = temp.path().join("runtime");
        let pids = test_dir.join("pids");
        let cache = test_dir.join("cache");
        let sockets = test_dir.join("sockets");

        std::fs::create_dir_all(&pids).unwrap();
        std::fs::write(pids.join("stale.pid"), "999").unwrap();

        init_runtime_dirs_at(&test_dir).unwrap();

        assert!(pids.is_dir(), "pids dir should exist");
        assert!(cache.is_dir(), "cache dir should exist");
        assert!(sockets.is_dir(), "sockets dir should exist");
        assert!(
            !pids.join("stale.pid").exists(),
            "stale files should be wiped"
        );
    }

    #[test]
    fn is_safe_path_component_cases() {
        let valid = [
            "plugin-launcher",
            "my_plugin",
            "plugin123",
            "UPPERCASE",
            "a",
            "MixedCase123",
        ];

        for s in valid {
            assert!(is_safe_path_component(s), "should be valid: {:?}", s);
        }

        let invalid = [
            "../etc",
            "foo/bar",
            "foo\\bar",
            "..",
            ".",
            "",
            "plugin\0evil",
            "plugin/",
            "/plugin",
            "plugin\\",
            "\\plugin",
            "a/b/c",
            "../..",
            "foo/../bar",
            " ",
            " plugin",
            "plugin ",
            ".hidden",
            "..hidden",
            "plugin..name",
            "plugin.name",
            "-plugin",
        ];

        for s in invalid {
            assert!(!is_safe_path_component(s), "should be invalid: {:?}", s);
        }
    }

    #[test]
    fn active_profile_name_defaults_when_no_marker() {
        let tmp = TempDir::new().unwrap();
        let _guard = push_test_path_root(tmp.path());
        assert_eq!(active_profile_name(), DEFAULT_PROFILE_NAME);
    }

    #[test]
    fn active_profile_name_falls_back_on_invalid_marker_content() {
        let tmp = TempDir::new().unwrap();
        let _guard = push_test_path_root(tmp.path());

        let marker = active_profile_marker_path().unwrap();
        file_io::ensure_parent_dir(&marker).unwrap();
        fs::write(&marker, "../escape\n").unwrap();

        assert_eq!(active_profile_name(), DEFAULT_PROFILE_NAME);
    }

    fn write_active_profile_marker(name: &str) {
        let marker = active_profile_marker_path().unwrap();
        file_io::ensure_parent_dir(&marker).unwrap();
        fs::write(&marker, format!("{}\n", name)).unwrap();
    }

    #[test]
    fn switching_profile_changes_resolved_paths() {
        let tmp = TempDir::new().unwrap();
        let _guard = push_test_path_root(tmp.path());

        write_active_profile_marker("personal");
        let personal_hotkeys = hotkeys_path().unwrap();
        let personal_shortcuts = shortcuts_path().unwrap();

        write_active_profile_marker("work");
        let work_hotkeys = hotkeys_path().unwrap();
        let work_shortcuts = shortcuts_path().unwrap();

        let sep = std::path::MAIN_SEPARATOR_STR;
        let personal_tail = format!("{sep}profile{sep}personal{sep}os{sep}");
        let work_tail = format!("{sep}profile{sep}work{sep}os{sep}");
        assert!(personal_hotkeys.to_string_lossy().contains(&personal_tail));
        assert!(work_hotkeys.to_string_lossy().contains(&work_tail));
        assert!(personal_shortcuts
            .to_string_lossy()
            .contains(&personal_tail));
        assert!(work_shortcuts.to_string_lossy().contains(&work_tail));
        assert_ne!(personal_hotkeys, work_hotkeys);
        assert_ne!(personal_shortcuts, work_shortcuts);
    }

    #[test]
    fn override_branch_nests_under_qol_config_namespace() {
        let tmp = TempDir::new().unwrap();
        let _guard = push_test_path_root(tmp.path());

        let data = base_data_dir().unwrap();
        assert!(
            data.ends_with(format!("data/{}", qol_config::NAMESPACE)),
            "data dir {data:?} should nest data/<namespace>"
        );

        let config = shared_config_dir().unwrap();
        assert!(
            config.ends_with(format!("config/{}", qol_config::NAMESPACE)),
            "config dir {config:?} should nest config/<namespace>"
        );
    }
}
