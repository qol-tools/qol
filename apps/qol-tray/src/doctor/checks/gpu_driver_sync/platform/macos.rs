pub(crate) fn watch_supported() -> bool {
    false
}

pub(crate) fn loaded_version() -> Option<String> {
    None
}

pub(crate) fn on_disk_version() -> Option<String> {
    None
}

pub(crate) fn notify_mismatch(_loaded: &str, _on_disk: &str) {}
