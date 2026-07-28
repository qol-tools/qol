use super::super::{AppKeyWriter, GsettingsBackend, SymbolicHotkeyWriter};
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

impl GsettingsBackend for Platform {
    fn read(&mut self, _schema: &str, _key: &str) -> Result<String> {
        Err(anyhow!("gsettings mutation is only supported on Linux"))
    }

    fn write(&mut self, _schema: &str, _key: &str, _value: &str) -> Result<()> {
        Err(anyhow!("gsettings mutation is only supported on Linux"))
    }
}
