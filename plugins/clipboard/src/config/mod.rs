use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::history::Retention;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub max_clips: u32,
    pub max_age_days: u32,
    pub keep_secrets: bool,
    pub secret_seconds: u32,
}

const CONTRACT: &str = qol_config::plugin_config_contract!();
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

pub fn load() -> Config {
    qol_config::load_plugin_config_from_env_with_contract(env!("QOL_PLUGIN_ID"), CONTRACT)
}

impl Config {
    pub fn retention(&self) -> Retention {
        Retention {
            max_unpinned: self.max_clips as usize,
            max_age: Duration::from_secs(u64::from(self.max_age_days) * SECONDS_PER_DAY),
            secret_ttl: Duration::from_secs(u64::from(self.secret_seconds)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_type_matches_the_contract_defaults() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONTRACT).unwrap();
    }

    #[test]
    fn retention_carries_the_configured_limits() {
        let config = Config {
            max_clips: 200,
            max_age_days: 7,
            keep_secrets: true,
            secret_seconds: 30,
        };
        let retention = config.retention();
        assert_eq!(retention.max_unpinned, 200);
        assert_eq!(retention.max_age, Duration::from_secs(7 * SECONDS_PER_DAY));
        assert_eq!(retention.secret_ttl, Duration::from_secs(30));
    }
}
