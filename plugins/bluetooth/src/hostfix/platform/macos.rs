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
