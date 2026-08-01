use super::super::{AppKeyWriter, SymbolicHotkeyWriter};
use anyhow::{anyhow, Result};

pub(crate) struct Platform;

impl SymbolicHotkeyWriter for Platform {
    fn disable(&mut self, _hotkey_id: u32) -> Result<()> {
        Err(anyhow!(
            "symbolic hotkey mutation is only supported on macOS"
        ))
    }
}

impl AppKeyWriter for Platform {
    fn clear(&mut self, _app_key: &str) -> Result<()> {
        Err(anyhow!(
            "Windows AppKey mutation is only supported on Windows"
        ))
    }
}
