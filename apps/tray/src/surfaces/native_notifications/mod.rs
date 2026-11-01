mod platform;

use qol_plugin_daemon::notification::gate::NativeHandler;
use qol_runtime::protocol::NotificationLevel;

pub fn show_already_running() {
    if native_gate_open() {
        platform::show_already_running();
    }
}

pub fn show_first_run() {
    if native_gate_open() {
        platform::show_first_run();
    }
}

pub fn show_plugin_notification(
    title: &str,
    body: &str,
    level: NotificationLevel,
    action: Option<(&str, &str)>,
) {
    if native_gate_open() {
        platform::show_plugin_notification(title, body, level, action);
    }
}

pub fn show_startup_failed(reason: &str) {
    if crate::headless_mode::active() {
        return;
    }
    platform::show_startup_failed(
        STARTUP_FAILED_TITLE,
        &startup_failure_body(reason, &qol_log::log_dir()),
    );
}

const STARTUP_FAILED_TITLE: &str = "QoL Tray could not start";

fn startup_failure_body(reason: &str, log_dir: &std::path::Path) -> String {
    format!(
        "{reason}\n\nThe log in {} has the details.",
        log_dir.display()
    )
}

fn native_gate_open() -> bool {
    !crate::headless_mode::active()
        && crate::features::notifications::native_handler() != NativeHandler::Qol
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_startup_failure_names_the_reason_and_where_the_log_is() {
        assert_eq!(
            startup_failure_body(
                "port 43210 is already in use by qol-tray.exe (pid 4242)",
                std::path::Path::new("/logs")
            ),
            "port 43210 is already in use by qol-tray.exe (pid 4242)\n\nThe log in /logs has the details."
        );
    }
}
