use anyhow::{bail, Result};

pub(crate) fn service_journal() -> Option<String> {
    None
}

pub(crate) fn audio_server() -> Option<String> {
    None
}

pub(crate) fn process_running(_process: &str) -> bool {
    false
}

pub(crate) fn stop_process(_process: &str) -> Result<()> {
    bail!("Bluetooth host fixes have no macOS backend yet")
}

pub(crate) fn start_process(_process: &str) -> Result<()> {
    bail!("Bluetooth host fixes have no macOS backend yet")
}

pub(crate) fn restart_service() -> Result<()> {
    bail!("Bluetooth host fixes have no macOS backend yet")
}

pub(crate) fn read_autostart() -> Option<String> {
    None
}

pub(crate) fn write_autostart(_content: &str) -> Result<()> {
    bail!("Bluetooth host fixes have no macOS autostart backend")
}

pub(crate) fn remove_autostart() -> Result<()> {
    bail!("Bluetooth host fixes have no macOS autostart backend")
}

pub(crate) fn supports_autostart() -> bool {
    false
}
