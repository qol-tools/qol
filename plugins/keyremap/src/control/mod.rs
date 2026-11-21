use std::sync::mpsc::Sender;

use crate::config::{Inspection, RemapConfig};

pub(crate) enum Command {
    Reload,
    Toggle,
    Kill,
    Settings,
}

#[derive(Clone)]
pub(crate) struct Host {
    pub(crate) plugin_id: String,
    pub(crate) read_config: fn() -> RemapConfig,
    pub(crate) inspect_config: fn() -> Result<Inspection, qol_config::PluginConfigInspectionError>,
    pub(crate) flip_enabled: fn() -> Result<bool, &'static str>,
    pub(crate) listen: fn(Sender<Command>) -> bool,
    pub(crate) send_reload: fn() -> bool,
    pub(crate) send_kill: fn() -> bool,
    pub(crate) cleanup: fn(),
    pub(crate) open_settings: fn() -> std::io::Result<()>,
}
