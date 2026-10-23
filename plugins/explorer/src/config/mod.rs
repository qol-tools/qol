use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use futures::channel::oneshot;
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
pub struct Pin {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ExplorerConfig {
    #[serde(default)]
    pub view_size: ViewSize,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default = "default_pins")]
    pub pins: Vec<Pin>,
    #[serde(default = "shows_title_bar")]
    pub title_bar: bool,
}

impl Default for ExplorerConfig {
    fn default() -> Self {
        Self {
            view_size: ViewSize::default(),
            show_hidden: false,
            pins: default_pins(),
            title_bar: shows_title_bar(),
        }
    }
}

pub fn default_pins() -> Vec<Pin> {
    [
        ("Home", "~"),
        ("Desktop", "~/Desktop"),
        ("Documents", "~/Documents"),
        ("Downloads", "~/Downloads"),
        ("Music", "~/Music"),
        ("Pictures", "~/Pictures"),
        ("Videos", "~/Videos"),
    ]
    .into_iter()
    .map(|(name, path)| Pin {
        name: name.to_string(),
        path: path.to_string(),
    })
    .collect()
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

enum Job {
    Save(&'static str, Value),
    Reload(oneshot::Sender<ExplorerConfig>),
}

pub struct ConfigSaver {
    jobs: Option<Sender<Job>>,
    worker: Option<JoinHandle<()>>,
    failures: Arc<Mutex<Vec<&'static str>>>,
}

impl ConfigSaver {
    pub fn start() -> Self {
        Self::with_writer(
            |keys| {
                let id = qol_config::plugin_id_from_env(PLUGIN_ID);
                qol_plugin_api::host_exec::save_plugin_config_keys(&id, keys).map_err(|error| {
                    let error = error.to_string();
                    log::warn!("[explorer] settings were not saved: {error}");
                    error
                })
            },
            load,
        )
    }

    fn with_writer(
        write: impl Fn(&Value) -> Result<(), String> + Send + 'static,
        read: impl Fn() -> ExplorerConfig + Send + 'static,
    ) -> Self {
        let (jobs, received) = mpsc::channel();
        let failures = Arc::new(Mutex::new(Vec::new()));
        let failed = failures.clone();
        let worker = thread::Builder::new()
            .name("explorer-config-save".into())
            .spawn(move || work_until_closed(&received, write, read, &failed))
            .map_err(|error| log::warn!("[explorer] config saver did not start: {error}"))
            .ok();
        Self {
            jobs: worker.is_some().then_some(jobs),
            worker,
            failures,
        }
    }

    pub fn save(&self, key: &'static str, value: impl Serialize) {
        let (Some(jobs), Ok(value)) = (&self.jobs, serde_json::to_value(value)) else {
            return;
        };
        let _ = jobs.send(Job::Save(key, value));
    }

    pub fn take_failures(&self) -> Vec<&'static str> {
        self.failures
            .lock()
            .map(|mut failures| std::mem::take(&mut *failures))
            .unwrap_or_default()
    }

    pub fn reload(&self) -> Option<oneshot::Receiver<ExplorerConfig>> {
        let jobs = self.jobs.as_ref()?;
        let (answer, receiver) = oneshot::channel();
        jobs.send(Job::Reload(answer)).ok()?;
        Some(receiver)
    }
}

impl Drop for ConfigSaver {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn work_until_closed(
    received: &Receiver<Job>,
    write: impl Fn(&Value) -> Result<(), String>,
    read: impl Fn() -> ExplorerConfig,
    failures: &Mutex<Vec<&'static str>>,
) {
    let mut next = received.recv().ok();
    while let Some(job) = next {
        next = match job {
            Job::Reload(answer) => {
                let _ = answer.send(read());
                received.recv().ok()
            }
            Job::Save(key, value) => {
                let (batch, pending) = batch_saves(key, value, received);
                if write(&Value::Object(batch.values)).is_err() {
                    record_failures(failures, &batch.names);
                }
                pending.or_else(|| received.recv().ok())
            }
        };
    }
}

fn record_failures(failures: &Mutex<Vec<&'static str>>, names: &[&'static str]) {
    if let Ok(mut failures) = failures.lock() {
        for name in names {
            if !failures.contains(name) {
                failures.push(name);
            }
        }
    }
}

struct Batch {
    values: Map<String, Value>,
    names: Vec<&'static str>,
}

impl Batch {
    fn add(&mut self, key: &'static str, value: Value) {
        self.values.insert(key.to_string(), value);
        if !self.names.contains(&key) {
            self.names.push(key);
        }
    }
}

fn batch_saves(key: &'static str, value: Value, received: &Receiver<Job>) -> (Batch, Option<Job>) {
    let mut batch = Batch {
        values: Map::new(),
        names: Vec::new(),
    };
    batch.add(key, value);
    while let Ok(job) = received.try_recv() {
        match job {
            Job::Save(key, value) => batch.add(key, value),
            reload => return (batch, Some(reload)),
        }
    }
    (batch, None)
}

#[cfg(test)]
mod tests;
