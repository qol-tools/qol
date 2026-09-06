use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
pub(crate) type ConfigInspection = qol_config::PluginConfigInspection<Config>;

pub const TUNING_REVISION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub pause_in_games: bool,
    pub pause_in_fullscreen: bool,
    pub shake_strictness: f64,
    pub regrow_strictness: f64,
    pub shake_min_extent_px: u32,
    pub regrow_min_extent_px: u32,
    pub shake_window_ms: u64,
    pub scale_factor: u32,
    pub calm_duration_ms: u64,
    pub grow_ms: u32,
    pub shrink_ms: u32,
    #[serde(default)]
    pub tuning_revision: u32,
}

pub fn load() -> Config {
    let mut config = load_from_disk();
    if config.tuning_revision < TUNING_REVISION {
        reset_tuning(&mut config);
        persist_tuning_reset(&config);
    }
    log_config(&config);
    config
}

pub(crate) fn inspect() -> Result<ConfigInspection, qol_config::PluginConfigInspectionError> {
    qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

fn load_from_disk() -> Config {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

fn reset_tuning(config: &mut Config) {
    let defaults = qol_config::typed_defaults_from_contract::<Config>(CONFIG_CONTRACT)
        .expect("config contract defaults must deserialize");
    config.shake_strictness = defaults.shake_strictness;
    config.regrow_strictness = defaults.regrow_strictness;
    config.shake_min_extent_px = defaults.shake_min_extent_px;
    config.regrow_min_extent_px = defaults.regrow_min_extent_px;
    config.shake_window_ms = defaults.shake_window_ms;
    config.scale_factor = defaults.scale_factor;
    config.calm_duration_ms = defaults.calm_duration_ms;
    config.grow_ms = defaults.grow_ms;
    config.shrink_ms = defaults.shrink_ms;
    config.tuning_revision = TUNING_REVISION;
    eprintln!(
        "[shake-to-grow] tuning reset to revision {TUNING_REVISION}: stored shake tuning predates the fast-shake defaults"
    );
}

fn persist_tuning_reset(config: &Config) {
    let id = qol_config::plugin_id_from_env(PLUGIN_ID);
    let Ok(value) = serde_json::to_value(config) else {
        eprintln!("[shake-to-grow] tuning reset could not be serialized and was not persisted");
        return;
    };
    if qol_runtime::PlatformStateClient::from_env().set_plugin_config(&id, &value) {
        eprintln!("[shake-to-grow] tuning reset persisted via the state socket");
        return;
    }
    let Some(path) = qol_config::plugin_config_write_path(&id) else {
        eprintln!(
            "[shake-to-grow] tuning reset could not be persisted, it re-applies on next start"
        );
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_string_pretty(config) else {
        eprintln!(
            "[shake-to-grow] tuning reset could not be persisted to {}, it re-applies on next start",
            path.display()
        );
        return;
    };
    if std::fs::write(&path, json).is_ok() {
        eprintln!(
            "[shake-to-grow] tuning reset persisted to {}",
            path.display()
        );
    } else {
        eprintln!(
            "[shake-to-grow] tuning reset could not be persisted to {}, it re-applies on next start",
            path.display()
        );
    }
}

#[cfg(test)]
fn contract_defaults() -> Config {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

fn log_config(config: &Config) {
    eprintln!(
        "[shake-to-grow] config: enabled={} pause_in_games={} fullscreen={} strictness={} regrow={} min_extent={}px regrow_extent={}px window_ms={} scale={} calm_ms={} grow_ms={} shrink_ms={} tuning_rev={}",
        config.enabled,
        config.pause_in_games,
        config.pause_in_fullscreen,
        config.shake_strictness,
        config.regrow_strictness,
        config.shake_min_extent_px,
        config.regrow_min_extent_px,
        config.shake_window_ms,
        config.scale_factor,
        config.calm_duration_ms,
        config.grow_ms,
        config.shrink_ms,
        config.tuning_revision,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONFIG_CONTRACT).unwrap();

        let defaults = contract_defaults();
        assert!(defaults.enabled, "shake-to-grow defaults to enabled");
        assert!(
            defaults.pause_in_games,
            "shake-to-grow defaults to pausing in games"
        );
        assert!(
            defaults.pause_in_fullscreen,
            "shake-to-grow defaults to pausing in fullscreen"
        );
        assert_eq!(defaults.shake_strictness, 5.0);
        assert_eq!(defaults.regrow_strictness, 2.5);
        assert_eq!(defaults.shake_min_extent_px, 150);
        assert_eq!(defaults.regrow_min_extent_px, 60);
        assert_eq!(defaults.shake_window_ms, 600);
        assert_eq!(defaults.calm_duration_ms, 100);
        assert_eq!(defaults.grow_ms, 120);
        assert_eq!(defaults.shrink_ms, 225);
        assert_eq!(
            defaults.tuning_revision, 0,
            "tuning_revision is daemon-owned and serde-defaults to 0 in the contract"
        );
    }

    #[test]
    fn stale_tuning_is_reset_to_contract_defaults() {
        let mut config = Config {
            enabled: true,
            pause_in_games: false,
            pause_in_fullscreen: true,
            shake_strictness: 6.0,
            regrow_strictness: 2.5,
            shake_min_extent_px: 150,
            regrow_min_extent_px: 60,
            shake_window_ms: 300,
            scale_factor: 4,
            calm_duration_ms: 175,
            grow_ms: 200,
            shrink_ms: 225,
            tuning_revision: 0,
        };
        reset_tuning(&mut config);
        let defaults = contract_defaults();
        assert_eq!(config.shake_strictness, defaults.shake_strictness);
        assert_eq!(config.regrow_strictness, defaults.regrow_strictness);
        assert_eq!(config.shake_min_extent_px, defaults.shake_min_extent_px);
        assert_eq!(config.regrow_min_extent_px, defaults.regrow_min_extent_px);
        assert_eq!(config.shake_window_ms, defaults.shake_window_ms);
        assert_eq!(config.scale_factor, defaults.scale_factor);
        assert_eq!(config.calm_duration_ms, defaults.calm_duration_ms);
        assert_eq!(config.grow_ms, defaults.grow_ms);
        assert_eq!(config.shrink_ms, defaults.shrink_ms);
        assert!(
            !config.pause_in_games,
            "reset_tuning must leave pause_in_games as stored"
        );
        assert_eq!(config.tuning_revision, TUNING_REVISION);
    }

    #[test]
    fn current_revision_is_left_alone() {
        let mut config = contract_defaults();
        config.shake_window_ms = 300;
        config.tuning_revision = TUNING_REVISION;
        if config.tuning_revision < TUNING_REVISION {
            reset_tuning(&mut config);
        }
        assert_eq!(
            config.shake_window_ms, 300,
            "a current tuning revision must keep the stored values"
        );
        assert_eq!(config.tuning_revision, TUNING_REVISION);
    }
}
