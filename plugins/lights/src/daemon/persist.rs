use std::collections::HashMap;
use std::io;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use serde_json::{Map, Value};

use super::looks::{self, normalize};
use crate::config::model::{DeviceEntry, LightLook, PluginConfig, PresetConfig};
use crate::config::store;

const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
const SETTLE: Duration = Duration::from_millis(400);

pub enum Change {
    Join(DeviceEntry),
    Forget {
        ieee: String,
        network_address: Option<u16>,
    },
    Looks(HashMap<String, LightLook>),
    Presets(PresetConfig),
    Flush(Sender<()>),
}

pub trait Backing {
    fn config(&self) -> Result<PluginConfig>;
    fn save(&self, keys: &Value) -> io::Result<()>;
}

struct HostBacking;

impl Backing for HostBacking {
    fn config(&self) -> Result<PluginConfig> {
        store::load()
    }

    fn save(&self, keys: &Value) -> io::Result<()> {
        let id = qol_config::plugin_id_from_env(store::PLUGIN_ID);
        qol_plugin_api::host_exec::save_plugin_config_keys(&id, keys)
    }
}

static WORKER: OnceLock<Sender<Change>> = OnceLock::new();

pub fn submit(change: Change) {
    let sender = WORKER.get_or_init(spawn_worker);
    if sender.send(change).is_err() {
        log::warn!("lights could not queue a config save");
    }
}

pub fn flush() {
    let Some(sender) = WORKER.get() else {
        return;
    };
    let (done, finished) = bounded(1);
    if sender.send(Change::Flush(done)).is_err() {
        return;
    }
    let _ = finished.recv_timeout(FLUSH_TIMEOUT);
}

fn spawn_worker() -> Sender<Change> {
    let (sender, receiver) = unbounded();
    let spawned = thread::Builder::new()
        .name("lights-persist".into())
        .spawn(move || run(&receiver, &HostBacking, SETTLE));
    if let Err(error) = spawned {
        log::warn!("lights could not start its config writer: {error}");
    }
    sender
}

pub fn run(receiver: &Receiver<Change>, backing: &impl Backing, settle: Duration) {
    while let Ok(first) = receiver.recv() {
        thread::sleep(settle);
        let mut batch = vec![first];
        batch.extend(receiver.try_iter());
        write_batch(backing, batch);
    }
}

fn write_batch(backing: &impl Backing, batch: Vec<Change>) {
    let (flushes, changes): (Vec<_>, Vec<_>) = batch
        .into_iter()
        .partition(|change| matches!(change, Change::Flush(_)));
    let keys = keys_for(backing, changes);
    if !keys.is_empty() {
        if let Err(error) = backing.save(&Value::Object(keys)) {
            log::warn!("lights could not save its settings: {error}");
        }
    }
    for flush in flushes {
        if let Change::Flush(done) = flush {
            let _ = done.send(());
        }
    }
}

pub fn keys_for(backing: &impl Backing, changes: Vec<Change>) -> Map<String, Value> {
    let mut keys = Map::new();
    let touches_devices = changes.iter().any(|change| {
        matches!(
            change,
            Change::Join(_) | Change::Forget { .. } | Change::Looks(_)
        )
    });
    let mut config = touches_devices
        .then(|| backing.config())
        .and_then(|loaded| {
            loaded
                .map_err(|error| {
                    log::warn!("lights could not read its devices to save them: {error:#}")
                })
                .ok()
        });
    let mut main_cleared = false;
    for change in changes {
        match change {
            Change::Join(entry) => {
                if let Some(config) = config.as_mut() {
                    upsert(&mut config.devices, entry);
                }
            }
            Change::Forget {
                ieee,
                network_address,
            } => {
                if let Some(config) = config.as_mut() {
                    main_cleared |= forget(config, &ieee, network_address);
                }
            }
            Change::Looks(snapshot) => {
                if let Some(config) = config.as_mut() {
                    looks::write_into(&mut config.devices, &snapshot);
                }
            }
            Change::Presets(presets) => {
                keys.insert("presets".into(), to_value(&presets));
            }
            Change::Flush(_) => {}
        }
    }
    if let Some(config) = config {
        keys.insert("devices".into(), to_value(&config.devices));
    }
    if main_cleared {
        keys.insert("main_target_id".into(), Value::String(String::new()));
    }
    keys
}

fn upsert(devices: &mut HashMap<String, DeviceEntry>, mut entry: DeviceEntry) {
    let key = entry.ieee_address.clone();
    if let Some(known) = devices.get(&key) {
        entry.look = known.look.clone();
    }
    devices.insert(key, entry);
}

fn forget(config: &mut PluginConfig, ieee: &str, network_address: Option<u16>) -> bool {
    config
        .devices
        .retain(|_, entry| normalize(&entry.ieee_address) != normalize(ieee));
    let named = !config.main_target_id.is_empty()
        && looks::names_light(&config.main_target_id, ieee, network_address);
    if named {
        config.main_target_id.clear();
    }
    named
}

fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const DESK: &str = "00:11:22:33:44:55:66:77";
    const SHELF: &str = "00:11:22:33:44:55:66:88";

    struct Fake {
        config: Option<PluginConfig>,
        saved: Mutex<Vec<Value>>,
    }

    impl Fake {
        fn with(config: Option<PluginConfig>) -> Self {
            Self {
                config,
                saved: Mutex::new(Vec::new()),
            }
        }
    }

    impl Backing for Fake {
        fn config(&self) -> Result<PluginConfig> {
            self.config
                .clone()
                .ok_or_else(|| anyhow::anyhow!("config not readable"))
        }

        fn save(&self, keys: &Value) -> io::Result<()> {
            self.saved.lock().unwrap().push(keys.clone());
            Ok(())
        }
    }

    fn entry(ieee: &str, look: Option<LightLook>) -> DeviceEntry {
        DeviceEntry {
            ieee_address: ieee.into(),
            name: "Light".into(),
            endpoints: Vec::new(),
            online: true,
            network_address: Some(0x1A2B),
            look,
        }
    }

    fn config(main: &str, devices: Vec<DeviceEntry>) -> PluginConfig {
        let mut config = PluginConfig {
            main_target_id: main.into(),
            ..PluginConfig::default()
        };
        for device in devices {
            config.devices.insert(device.ieee_address.clone(), device);
        }
        config
    }

    fn off() -> LightLook {
        LightLook {
            on: false,
            color_hex: "#FF0000".into(),
            brightness: 5,
            mirek: 400,
        }
    }

    fn drain(fake: &Fake, changes: Vec<Change>) {
        let (sender, receiver) = unbounded();
        for change in changes {
            sender.send(change).unwrap();
        }
        drop(sender);
        run(&receiver, fake, Duration::ZERO);
    }

    #[test]
    fn queued_looks_are_written_once_with_the_latest_winning() {
        let fake = Fake::with(Some(config("", vec![entry(DESK, None)])));
        let first = HashMap::from([(DESK.to_string(), off())]);
        let mut second = off();
        second.brightness = 77;
        let second = HashMap::from([(DESK.to_string(), second)]);

        drain(&fake, vec![Change::Looks(first), Change::Looks(second)]);

        let saved = fake.saved.lock().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0]["devices"][DESK]["look"]["brightness"], 77);
        assert!(saved[0].get("presets").is_none());
        assert!(saved[0].get("main_target_id").is_none());
    }

    #[test]
    fn a_light_that_rejoins_keeps_its_look() {
        let fake = Fake::with(Some(config("", vec![entry(DESK, Some(off()))])));
        let mut rejoined = entry(DESK, None);
        rejoined.name = "Device 0001".into();

        drain(
            &fake,
            vec![Change::Join(rejoined), Change::Join(entry(SHELF, None))],
        );

        let saved = fake.saved.lock().unwrap();
        assert_eq!(saved[0]["devices"][DESK]["name"], "Device 0001");
        assert_eq!(saved[0]["devices"][DESK]["look"]["on"], false);
        assert!(saved[0]["devices"][SHELF].get("look").is_none());
    }

    #[test]
    fn forgetting_a_light_clears_the_main_target_only_when_it_named_that_light() {
        let cases = [("", false), (DESK, true), ("0x1a2b", true), (SHELF, false)];
        for (main, cleared) in cases {
            let fake = Fake::with(Some(config(
                main,
                vec![entry(DESK, None), entry(SHELF, None)],
            )));

            drain(
                &fake,
                vec![Change::Forget {
                    ieee: DESK.into(),
                    network_address: Some(0x1A2B),
                }],
            );

            let saved = fake.saved.lock().unwrap();
            assert!(saved[0]["devices"].get(DESK).is_none(), "main {main}");
            assert_eq!(
                saved[0].get("main_target_id").is_some(),
                cleared,
                "main {main}"
            );
        }
    }

    #[test]
    fn a_forget_is_not_undone_by_a_look_queued_before_it() {
        let fake = Fake::with(Some(config("", vec![entry(DESK, None)])));
        let stale = HashMap::from([(DESK.to_string(), off())]);

        drain(
            &fake,
            vec![
                Change::Looks(stale),
                Change::Forget {
                    ieee: DESK.into(),
                    network_address: None,
                },
            ],
        );

        assert!(fake.saved.lock().unwrap()[0]["devices"].get(DESK).is_none());
    }

    #[test]
    fn presets_save_without_reading_the_devices() {
        let fake = Fake::with(None);
        let mut presets = PresetConfig::default();
        presets.slot_mut(2).unwrap().enabled = true;

        drain(&fake, vec![Change::Presets(presets)]);

        let saved = fake.saved.lock().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0]["presets"]["preset_2"]["enabled"], true);
        assert!(saved[0].get("devices").is_none());
    }

    #[test]
    fn unreadable_devices_write_nothing_and_a_flush_still_answers() {
        let fake = Fake::with(None);
        let (done, finished) = bounded(1);

        drain(
            &fake,
            vec![Change::Looks(HashMap::new()), Change::Flush(done)],
        );

        assert!(fake.saved.lock().unwrap().is_empty());
        assert!(finished.try_recv().is_ok());
    }
}
