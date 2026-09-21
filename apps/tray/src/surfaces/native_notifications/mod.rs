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

fn native_gate_open() -> bool {
    crate::features::notifications::native_handler() != NativeHandler::Qol
}
