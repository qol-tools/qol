pub(crate) fn show_already_running() {}

pub(crate) fn show_first_run() {}

pub(crate) fn show_plugin_notification(
    _title: &str,
    _body: &str,
    _level: qol_runtime::protocol::NotificationLevel,
    _action: Option<(&str, &str)>,
) {
}
