use super::{HotkeyBinding, HotkeyConfig};
use crate::file_io;
use anyhow::Result;
use qol_migrations::hotkey_layers;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct HotkeyLayers {
    pub(crate) core: PathBuf,
    pub(crate) os: PathBuf,
}

impl HotkeyLayers {
    pub(crate) fn active() -> Result<Self> {
        Ok(Self {
            core: crate::paths::core_hotkeys_path()?,
            os: crate::paths::hotkeys_path()?,
        })
    }

    pub(crate) fn load(&self) -> Result<HotkeyConfig> {
        Ok(HotkeyConfig {
            hotkeys: serde_json::from_value::<Vec<HotkeyBinding>>(Value::Array(
                self.load_values()?,
            ))?,
        })
    }

    pub(crate) fn load_values(&self) -> Result<Vec<Value>> {
        Ok(hotkey_layers::merge(
            &read_layer(&self.core)?,
            &read_layer(&self.os)?,
        ))
    }

    pub(crate) fn save(&self, config: &HotkeyConfig) -> Result<()> {
        let Value::Array(edited) = serde_json::to_value(&config.hotkeys)? else {
            anyhow::bail!("hotkeys did not serialize to a list");
        };
        self.save_values(&edited)
    }

    pub(crate) fn save_values(&self, edited: &[Value]) -> Result<()> {
        let (core, os) =
            hotkey_layers::split(edited, &read_layer(&self.core)?, &read_layer(&self.os)?);
        file_io::write_pretty_json(&self.core, &json!({ "hotkeys": core }))?;
        file_io::write_pretty_json(&self.os, &json!({ "hotkeys": os }))
    }
}

fn read_layer(path: &Path) -> Result<Vec<Value>> {
    Ok(qol_profile_sync::state::read_hotkey_layer(path)?.unwrap_or_default())
}
