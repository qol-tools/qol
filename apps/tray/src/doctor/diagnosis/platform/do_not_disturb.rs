use super::super::NATIVE_NOTIFICATIONS_ID;
use crate::doctor::framework::{CheckReport, DoctorIssue, Severity};
use qol_plugin_daemon::notification::gate::NativeHandler;

pub(crate) fn native_notifications_diagnosis() -> CheckReport {
    if crate::features::notifications::native_handler() == NativeHandler::Qol {
        return CheckReport::ok("native notifications are disabled; qol toasts are used");
    }
    match qol_plugin_daemon::notification::platform::os_do_not_disturb() {
        Some(true) => CheckReport::warn(
            "OS do-not-disturb is enabled; native notifications will be suppressed",
            NATIVE_NOTIFICATIONS_ID,
            Vec::new(),
        ),
        Some(false) => {
            CheckReport::ok("OS do-not-disturb is off; native notifications will be delivered")
        }
        None => CheckReport {
            summary: "OS do-not-disturb state is unknown".to_string(),
            issues: vec![DoctorIssue::new(
                NATIVE_NOTIFICATIONS_ID,
                Severity::Info,
                "OS do-not-disturb state is unknown",
            )],
            advice: Vec::new(),
            fixes: Vec::new(),
        },
    }
}
