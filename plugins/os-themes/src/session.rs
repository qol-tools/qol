use std::path::PathBuf;

pub use qol_host_session::{RestoreMode, RestoreReport, SessionSnapshot, SessionStore};

pub fn session_subdir(subdir: &str) -> PathBuf {
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

pub fn recover() {
    let mut report = RestoreReport::default();
    crate::theme::restore(RestoreMode::Recovery, &mut report);
    crate::cursor::recover();
    if report.restored > 0 {
        log::warn!(
            "recovered {} pre-qol theme values after an abnormal exit",
            report.restored
        );
    }
    if report.failed > 0 {
        log::warn!("{} theme values could not be recovered", report.failed);
    }
}

pub fn restore_exit() {
    let mut report = RestoreReport::default();
    crate::theme::restore(RestoreMode::Exit, &mut report);
    if report.restored > 0 || report.failed > 0 {
        log::info!(
            "exit restore: restored={} nothing={} failed={}",
            report.restored,
            report.nothing_to_restore,
            report.failed
        );
    }
}
