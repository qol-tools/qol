use anyhow::Result;
use std::sync::Arc;

use crate::daemon::EventBus;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) mod common;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallKind {
    UserLocal,
    SystemWide,
    Development,
}

impl InstallKind {
    pub(crate) fn detect() -> Self {
        let exe = std::env::current_exe()
            .and_then(|p| std::fs::canonicalize(&p).or(Ok(p)))
            .ok();
        let exe_str = exe.as_deref().and_then(|p| p.to_str()).unwrap_or_default();
        let home = dirs::home_dir().and_then(|h| h.to_str().map(String::from));
        install_kind_for_path(exe_str, home.as_deref())
    }
}

fn install_kind_for_path(exe_path: &str, home: Option<&str>) -> InstallKind {
    if exe_path.contains("target/debug/") || exe_path.contains("target/release/") {
        return InstallKind::Development;
    }

    #[cfg(target_os = "macos")]
    if exe_path.contains(".app/Contents/MacOS/") {
        if let Some(home) = home {
            if exe_path.starts_with(home) {
                return InstallKind::UserLocal;
            }
        }
    }

    if let Some(home) = home {
        let local_bin = format!("{home}/.local/bin/");
        if exe_path.starts_with(&local_bin) {
            return InstallKind::UserLocal;
        }
    }

    InstallKind::SystemWide
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("updates::platform::download_and_install is not implemented for this target OS");

#[cfg(target_os = "linux")]
pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    log::info!("Install kind: {:?}", InstallKind::detect());
    linux::download_and_install(events).await
}

#[cfg(target_os = "macos")]
pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    log::info!("Install kind: {:?}", InstallKind::detect());
    macos::download_and_install(events).await
}

#[cfg(target_os = "windows")]
#[allow(clippy::unused_async)]
pub(super) async fn download_and_install(_events: Arc<EventBus>) -> Result<()> {
    log::info!("Install kind: {:?}", InstallKind::detect());
    open_latest_release_page()
}

#[cfg(target_os = "windows")]
fn open_latest_release_page() -> Result<()> {
    let url = format!("https://github.com/{}/releases/latest", super::GITHUB_REPO);
    crate::paths::open_url(&url)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_detected_from_target_debug() {
        let cases = [
            "/a/b/target/debug/foo",
            "/a/b/target/release/foo",
            "/a/target/debug/deps/foo",
        ];
        for path in cases {
            assert_eq!(
                install_kind_for_path(path, Some("/a")),
                InstallKind::Development,
                "path: {path}"
            );
        }
    }

    #[test]
    fn user_local_detected_from_local_bin() {
        assert_eq!(
            install_kind_for_path("/a/.local/bin/foo", Some("/a")),
            InstallKind::UserLocal,
        );
    }

    #[test]
    fn system_wide_detected_from_usr_bin() {
        let cases = ["/usr/bin/foo", "/usr/local/bin/foo", "/opt/foo/bin/foo"];
        for path in cases {
            assert_eq!(
                install_kind_for_path(path, Some("/a")),
                InstallKind::SystemWide,
                "path: {path}"
            );
        }
    }

    #[test]
    fn system_wide_when_home_dir_unavailable() {
        assert_eq!(
            install_kind_for_path("/a/.local/bin/foo", None),
            InstallKind::SystemWide,
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn user_local_detected_from_app_bundle() {
        assert_eq!(
            install_kind_for_path("/a/Applications/Foo.app/Contents/MacOS/foo", Some("/a")),
            InstallKind::UserLocal,
        );
    }
}
