use crate::platform::engine::config::{RemapConfig, CONFIG_CONTRACT};

pub(super) fn without_macos_profile(config: RemapConfig) -> RemapConfig {
    match qol_config::typed_defaults_from_contract::<RemapConfig>(CONFIG_CONTRACT) {
        Ok(shipped) => strip(config, &shipped),
        Err(errors) => {
            log::warn!("could not read the shipped rules to skip them: {errors:?}");
            config
        }
    }
}

fn strip(mut config: RemapConfig, shipped: &RemapConfig) -> RemapConfig {
    let before = rule_count(&config);
    config
        .key_rules
        .retain(|rule| !shipped.key_rules.contains(rule));
    config
        .mouse_rules
        .retain(|rule| !shipped.mouse_rules.contains(rule));
    config
        .scroll_rules
        .retain(|rule| !shipped.scroll_rules.contains(rule));
    let skipped = before - rule_count(&config);
    if skipped > 0 {
        log::info!("skipped {skipped} shipped macOS rules that Windows already behaves like");
    }
    config
}

fn rule_count(config: &RemapConfig) -> usize {
    config.key_rules.len() + config.mouse_rules.len() + config.scroll_rules.len()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::platform::engine::config::builtin_defaults;

    #[test]
    fn shipped_rules_are_skipped_and_user_rules_stay() {
        let cases = [
            ("the shipped defaults", json!(null), 0, 0, 0),
            (
                "a user rule beside the shipped ones",
                json!({ "key_rules": [{ "from_mods": ["cmd"], "to_mods": ["ctrl"], "keys": ["c", "v"] }] }),
                1,
                0,
                0,
            ),
            (
                "an edited shipped rule",
                json!({ "key_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["c"] }] }),
                1,
                0,
                0,
            ),
            (
                "a shipped rule saved with an explicit global flag",
                json!({
                    "mouse_rules": [{ "from_mods": ["ctrl"], "button": "left", "to_mods": ["cmd"], "global": false }],
                    "scroll_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "global": false }]
                }),
                0,
                0,
                0,
            ),
            (
                "a global copy of a shipped rule",
                json!({ "scroll_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "global": true }] }),
                0,
                0,
                1,
            ),
        ];
        let shipped = builtin_defaults();
        for (name, overrides, keys, mice, scrolls) in cases {
            let mut raw = serde_json::to_value(&shipped).unwrap();
            if let (Some(fields), Some(changes)) = (raw.as_object_mut(), overrides.as_object()) {
                for (field, value) in changes {
                    fields.insert(field.clone(), value.clone());
                }
            }
            let config: RemapConfig = serde_json::from_value(raw).unwrap();
            let stripped = strip(config, &shipped);
            assert_eq!(stripped.key_rules.len(), keys, "{name}");
            assert_eq!(stripped.mouse_rules.len(), mice, "{name}");
            assert_eq!(stripped.scroll_rules.len(), scrolls, "{name}");
        }
    }

    #[test]
    fn the_contract_profile_loads() {
        assert_eq!(rule_count(&without_macos_profile(builtin_defaults())), 0);
    }
}
