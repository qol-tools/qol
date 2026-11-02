use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use super::compile::Compiled;
use super::document::{self, Removed, Sidecar, Synced};

const CONFIG_PATH: &str = ".config/karabiner/karabiner.json";
const BACKUP_SUFFIX: &str = ".qol-keyremap.bak";
const SIDECAR_FILE: &str = "karabiner-sidecar.json";

pub(crate) struct Store {
    config: PathBuf,
    sidecar: PathBuf,
}

impl Store {
    pub(crate) fn locate() -> Result<Self> {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        let id = qol_config::plugin_id_from_env(crate::cli::PLUGIN_ID);
        let state = qol_config::data_dir()
            .context("no data directory for keyremap's Karabiner record")?
            .join("plugins")
            .join(id);
        Ok(Self {
            config: Path::new(&home).join(CONFIG_PATH),
            sidecar: state.join(SIDECAR_FILE),
        })
    }

    pub(crate) fn config_path(&self) -> &Path {
        &self.config
    }

    pub(crate) fn sync(&self, compiled: &Compiled) -> Result<Synced> {
        let (target, current) = self.read_config()?;
        let sidecar = self.read_sidecar()?;
        let synced = document::sync(&current, compiled, &sidecar)?;
        if synced.changed {
            back_up_once(&target)?;
            write_json(&target, &synced.document)?;
        }
        if synced.sidecar != sidecar {
            self.write_sidecar(&synced.sidecar)?;
        }
        Ok(synced)
    }

    pub(crate) fn plan(&self, compiled: &Compiled) -> Result<Synced> {
        let (_, current) = self.read_config()?;
        document::sync(&current, compiled, &self.read_sidecar()?)
    }

    pub(crate) fn remove(&self) -> Result<Removed> {
        let (target, current) = self.read_config()?;
        let sidecar = self.read_sidecar()?;
        let removed = document::remove(&current, &sidecar);
        if removed.changed {
            write_json(&target, &removed.document)?;
        }
        if sidecar != Sidecar::default() {
            self.write_sidecar(&Sidecar::default())?;
        }
        Ok(removed)
    }

    pub(crate) fn read_document(&self) -> Result<Value> {
        Ok(self.read_config()?.1)
    }

    fn read_config(&self) -> Result<(PathBuf, Value)> {
        let target = std::fs::canonicalize(&self.config).with_context(|| {
            format!(
                "{} does not exist; open Karabiner-Elements once so it writes it",
                self.config.display()
            )
        })?;
        let text = std::fs::read_to_string(&target)
            .with_context(|| format!("could not read {}", target.display()))?;
        let value = serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON", target.display()))?;
        Ok((target, value))
    }

    fn read_sidecar(&self) -> Result<Sidecar> {
        match std::fs::read_to_string(&self.sidecar) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid JSON", self.sidecar.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Sidecar::default()),
            Err(error) => {
                Err(error).with_context(|| format!("could not read {}", self.sidecar.display()))
            }
        }
    }

    fn write_sidecar(&self, sidecar: &Sidecar) -> Result<()> {
        if let Some(parent) = self.sidecar.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        write_json(&self.sidecar, sidecar)
    }
}

fn back_up_once(target: &Path) -> Result<()> {
    let mut backup = target.as_os_str().to_owned();
    backup.push(BACKUP_SUFFIX);
    let backup = PathBuf::from(backup);
    if backup.exists() {
        return Ok(());
    }
    std::fs::copy(target, &backup)
        .with_context(|| format!("could not back up {} first", target.display()))?;
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, formatter);
    value.serialize(&mut serializer)?;
    bytes.push(b'\n');
    qol_fs::atomic_write(path, &bytes)
        .with_context(|| format!("could not write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::platform::macos::karabiner::compile::RULE_DESCRIPTION;

    fn store(dir: &Path) -> Store {
        Store {
            config: dir.join("karabiner").join("karabiner.json"),
            sidecar: dir.join("state").join(SIDECAR_FILE),
        }
    }

    fn compiled() -> Compiled {
        Compiled {
            rule: json!({ "description": RULE_DESCRIPTION, "manipulators": [] }),
            simple_modifications: vec![
                json!({ "from": { "key_code": "caps_lock" }, "to": [{ "key_code": "escape" }] }),
            ],
            gaps: Vec::new(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qol-keyremap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        std::fs::create_dir_all(dir.join("karabiner")).unwrap();
        dir
    }

    #[test]
    fn a_symlinked_config_is_written_through_and_backed_up_once() {
        let dir = scratch("symlink");
        let real = dir.join("dotfiles").join("karabiner.json");
        let original = r#"{"profiles":[{"name":"Default profile","selected":true}]}"#;
        std::fs::write(&real, original).unwrap();
        std::os::unix::fs::symlink(&real, dir.join("karabiner").join("karabiner.json")).unwrap();
        let store = store(&dir);

        assert!(store.sync(&compiled()).unwrap().changed);
        assert!(std::fs::symlink_metadata(store.config_path())
            .unwrap()
            .file_type()
            .is_symlink());
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&real).unwrap()).unwrap();
        assert_eq!(
            written["profiles"][0]["complex_modifications"]["rules"][0]["description"],
            RULE_DESCRIPTION
        );
        let backup = dir.join("dotfiles").join("karabiner.json.qol-keyremap.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);

        assert!(store.remove().unwrap().changed);
        let removed: Value =
            serde_json::from_str(&std::fs::read_to_string(&real).unwrap()).unwrap();
        assert_eq!(removed["profiles"][0]["simple_modifications"], json!([]));
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_config_says_to_open_karabiner() {
        let dir = scratch("missing");
        let error = store(&dir).sync(&compiled()).unwrap_err();
        assert!(
            format!("{error:#}").contains("open Karabiner-Elements once"),
            "{error:#}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
