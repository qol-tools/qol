use anyhow::Result;
use std::sync::Arc;

use crate::daemon::EventBus;

use super::InstallKind;

pub(super) fn detect_install_kind() -> InstallKind {
    let executable = std::env::current_exe()
        .and_then(|path| std::fs::canonicalize(&path).or(Ok(path)))
        .ok();
    let executable = executable
        .as_deref()
        .and_then(|path| path.to_str())
        .unwrap_or_default();
    let home = dirs::home_dir().and_then(|path| path.to_str().map(String::from));
    InstallKind::for_path(executable, home.as_deref(), false)
}

#[allow(clippy::unused_async)]
pub(super) async fn download_and_install(_events: Arc<EventBus>) -> Result<()> {
    log::info!("Install kind: {:?}", InstallKind::detect());
    let url = format!(
        "https://github.com/{}/releases/latest",
        super::super::GITHUB_REPO
    );
    crate::paths::open_url(&url)?;
    Ok(())
}
