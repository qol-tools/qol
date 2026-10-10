use std::path::PathBuf;

pub(super) fn session_subdir(subdir: &str) -> PathBuf {
    if let Some(base) = qol_config::data_subdir("os-themes-session") {
        let dir = base.join(subdir);
        if let Err(error) = qol_fs::create_private_dir(&dir) {
            log::warn!("cannot secure session dir {}: {error}", dir.display());
        }
        return dir;
    }
    let fallback = std::env::temp_dir()
        .join("qol-os-themes-session")
        .join(subdir);
    if let Err(error) = qol_fs::create_private_dir(&fallback) {
        log::warn!(
            "cannot secure fallback session dir {}: {error}",
            fallback.display()
        );
    }
    fallback
}
