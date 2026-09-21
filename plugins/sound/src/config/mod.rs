use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

pub fn contract() -> &'static str {
    CONFIG_CONTRACT
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct OutputConfig {
    pub device: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct SoundConfig {
    pub output: OutputConfig,
}

pub fn inspect(
) -> Result<qol_config::PluginConfigInspection<SoundConfig>, qol_config::PluginConfigInspectionError>
{
    qol_config::inspect_plugin_config_from_env_with_contract(crate::PLUGIN_ID, CONFIG_CONTRACT)
}

pub fn save_output_device(device: &str) -> anyhow::Result<()> {
    let config = SoundConfig {
        output: OutputConfig {
            device: device.to_owned(),
        },
    };
    if qol_runtime::plugin_config::save(&config) {
        Ok(())
    } else {
        anyhow::bail!(
            "{}: failed to persist the sound output choice over the runtime socket",
            crate::PLUGIN_ID
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Option<Self> {
            let path = std::env::temp_dir().join(format!(
                "qol-sound-config-test-{label}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).ok()?;
            Some(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct EnvScope {
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvScope {
        fn capture(keys: &[&'static str]) -> Self {
            let saved = keys
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect();
            Self { saved }
        }

        fn set(&self, key: &'static str, value: impl AsRef<OsStr>) {
            std::env::set_var(key, value);
        }
    }

    impl Drop for EnvScope {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    struct PinnedInstall {
        _env: EnvScope,
        _root: TempRoot,
        config_path: PathBuf,
    }

    impl PinnedInstall {
        fn new(label: &str) -> Option<Self> {
            let root = TempRoot::new(label)?;
            let data_home = root.path().join("data");
            let config_home = root.path().join("config");
            let install_id = format!("sound-config-test-{label}-{}", std::process::id());
            let config_path = data_home
                .join(qol_config::NAMESPACE)
                .join("installs")
                .join(&install_id)
                .join("plugins")
                .join(crate::PLUGIN_ID)
                .join("config.json");
            let env = EnvScope::capture(&[
                qol_conventions::ENV_INSTALL_ID,
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
            ]);
            env.set(qol_conventions::ENV_INSTALL_ID, &install_id);
            env.set("XDG_DATA_HOME", &data_home);
            env.set("XDG_CONFIG_HOME", &config_home);
            Some(Self {
                _env: env,
                _root: root,
                config_path,
            })
        }

        fn write(&self, contents: &str) {
            if let Some(parent) = self.config_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&self.config_path, contents).unwrap();
        }
    }

    #[test]
    fn contract_defaults_match_runtime_type() {
        let _lock = env_lock();
        qol_config::validate_contract_defaults_match_type::<SoundConfig>(CONFIG_CONTRACT).unwrap();
        let config: SoundConfig =
            qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).unwrap();
        assert_eq!(config.output.device, "default");
    }

    #[test]
    fn stored_value_beats_the_contract_default() {
        let _lock = env_lock();
        let Some(pinned) = PinnedInstall::new("stored") else {
            return;
        };

        assert!(
            pinned.config_path.starts_with(std::env::temp_dir()),
            "the test config stays in a temp root"
        );

        pinned.write("{}");
        let defaults = inspect().unwrap();
        assert_eq!(defaults.config.output.device, "default");
        assert_eq!(
            defaults.source.as_deref(),
            Some(pinned.config_path.as_path())
        );

        pinned.write(r#"{"output":{"device":"Luna 2"}}"#);
        let stored = inspect().unwrap();
        assert_eq!(stored.config.output.device, "Luna 2");
        assert_eq!(stored.source.as_deref(), Some(pinned.config_path.as_path()));
    }

    #[test]
    fn unreadable_config_is_an_inspection_error_not_a_silent_default() {
        let _lock = env_lock();
        let Some(pinned) = PinnedInstall::new("unreadable") else {
            return;
        };

        pinned.write("{ not json");

        let error = inspect().unwrap_err();
        assert_eq!(error.path(), pinned.config_path.as_path());
        assert!(!error.to_string().is_empty());
    }
}
