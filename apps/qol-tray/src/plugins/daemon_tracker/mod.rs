use super::Plugin;
use crate::paths;

pub mod platform;

pub(super) fn daemon_pids_path() -> Option<std::path::PathBuf> {
    paths::config_dir().ok().map(|p| p.join(".daemon-pids"))
}

pub fn kill_orphan_daemons() {
    platform::kill_orphan_daemons();
}

pub fn clean_stale_sockets(plugins: &[Plugin]) {
    platform::clean_stale_sockets(plugins);
}

pub fn save_daemon_pids(pids: &[u32]) {
    let Some(path) = daemon_pids_path() else {
        return;
    };
    let content = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&path, content);
}
