use anyhow::{bail, Result};

pub(super) fn service_journal() -> Option<String> {
    None
}

pub(super) fn audio_server() -> Option<String> {
    None
}

pub(super) fn process_running(_process: &str) -> bool {
    false
}

pub(super) fn stop_process(_process: &str) -> Result<()> {
    bail!("Bluetooth host fixes are not implemented on this platform")
}

pub(super) fn start_process(_process: &str) -> Result<()> {
    bail!("Bluetooth host fixes are not implemented on this platform")
}

pub(super) fn restart_service() -> Result<()> {
    bail!("Bluetooth host fixes are not implemented on this platform")
}

pub(super) fn read_autostart() -> Option<String> {
    None
}

pub(super) fn write_autostart(_content: &str) -> Result<()> {
    bail!("Bluetooth host fixes have no autostart backend on this platform")
}

pub(super) fn remove_autostart() -> Result<()> {
    bail!("Bluetooth host fixes have no autostart backend on this platform")
}

pub(super) fn supports_autostart() -> bool {
    false
}
