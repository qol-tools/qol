pub(crate) mod platform;

pub use qol_host_session::{RestoreMode, RestoreReport};

pub(crate) use platform::Platform;

use platform::SessionPlatform;

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

pub fn restore_on_exit() {
    restore_exit_when(Platform.exit_restores_host());
}

pub(crate) fn restore_exit_when(restores_host: bool) {
    if restores_host {
        restore_exit();
    }
}

fn restore_exit() {
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
