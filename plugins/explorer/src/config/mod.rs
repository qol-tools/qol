use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::cli::PLUGIN_ID;

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewSize {
    Compact,
    #[default]
    List,
    Roomy,
    Icons,
    BigIcons,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ExplorerConfig {
    #[serde(default)]
    pub view_size: ViewSize,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default = "shows_title_bar")]
    pub title_bar: bool,
}

impl Default for ExplorerConfig {
    fn default() -> Self {
        Self {
            view_size: ViewSize::default(),
            show_hidden: false,
            title_bar: shows_title_bar(),
        }
    }
}

fn shows_title_bar() -> bool {
    true
}

pub fn load() -> ExplorerConfig {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

pub fn inspect() -> Result<
    qol_config::PluginConfigInspection<ExplorerConfig>,
    qol_config::PluginConfigInspectionError,
> {
    qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

pub struct ConfigSaver {
    keys: Option<Sender<(&'static str, Value)>>,
    worker: Option<JoinHandle<()>>,
}

impl ConfigSaver {
    pub fn start() -> Self {
        Self::with_writer(|keys| {
            let id = qol_config::plugin_id_from_env(PLUGIN_ID);
            if let Err(error) = qol_plugin_api::host_exec::save_plugin_config_keys(&id, keys) {
                log::warn!("[explorer] view settings were not saved: {error}");
            }
        })
    }

    fn with_writer(write: impl Fn(&Value) + Send + 'static) -> Self {
        let (keys, received) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("explorer-config-save".into())
            .spawn(move || save_until_closed(&received, write))
            .map_err(|error| log::warn!("[explorer] config saver did not start: {error}"))
            .ok();
        Self {
            keys: worker.is_some().then_some(keys),
            worker,
        }
    }

    pub fn save(&self, key: &'static str, value: impl Serialize) {
        let (Some(keys), Ok(value)) = (&self.keys, serde_json::to_value(value)) else {
            return;
        };
        let _ = keys.send((key, value));
    }
}

impl Drop for ConfigSaver {
    fn drop(&mut self) {
        self.keys.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn save_until_closed(received: &Receiver<(&'static str, Value)>, write: impl Fn(&Value)) {
    while let Ok((key, value)) = received.recv() {
        let mut keys = Map::new();
        keys.insert(key.to_string(), value);
        while let Ok((key, value)) = received.try_recv() {
            keys.insert(key.to_string(), value);
        }
        write(&Value::Object(keys));
    }
}

#[cfg(test)]
mod tests;
