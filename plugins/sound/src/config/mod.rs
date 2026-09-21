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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct PinnedInstall {
        root: PathBuf,
        config_path: PathBuf,
        previous_install_id: Option<String>,
    }

    impl PinnedInstall {
        fn new(label: &str) -> Option<Self> {
            let data_dir = qol_config::data_dir()?;
            let install_id = format!("sound-config-test-{label}-{}", std::process::id());
            let previous_install_id = std::env::var(qol_conventions::ENV_INSTALL_ID).ok();
            std::env::set_var(qol_conventions::ENV_INSTALL_ID, &install_id);
            let plugin_id = qol_config::plugin_id_from_env(crate::PLUGIN_ID);
            let config_path = qol_config::plugin_config_paths(&[plugin_id.as_str()])
                .into_iter()
                .next()?;
            Some(Self {
                root: data_dir.join("installs").join(&install_id),
                config_path,
                previous_install_id,
            })
        }

        fn write(&self, contents: &str) {
            if let Some(parent) = self.config_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&self.config_path, contents).unwrap();
        }
    }

    impl Drop for PinnedInstall {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
            match self.previous_install_id.take() {
                Some(value) => std::env::set_var(qol_conventions::ENV_INSTALL_ID, value),
                None => std::env::remove_var(qol_conventions::ENV_INSTALL_ID),
            }
        }
    }

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<SoundConfig>(CONFIG_CONTRACT).unwrap();
        let config: SoundConfig =
            qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).unwrap();
        assert_eq!(config.output.device, "default");
    }

    #[test]
    fn stored_value_beats_the_contract_default() {
        let _lock = ENV_LOCK.lock().unwrap();
        let Some(pinned) = PinnedInstall::new("stored") else {
            return;
        };

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
        let _lock = ENV_LOCK.lock().unwrap();
        let Some(pinned) = PinnedInstall::new("unreadable") else {
            return;
        };

        pinned.write("{ not json");

        let error = inspect().unwrap_err();
        assert_eq!(error.path(), pinned.config_path.as_path());
        assert!(!error.to_string().is_empty());
    }
}
