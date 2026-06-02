use axum::{http::StatusCode, response::IntoResponse, response::Response};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct CombinedPluginForm {
    #[serde(flatten)]
    pub form: qol_config::normalized::ResolvedConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<qol_config::contract::RuntimeSpec>,
    pub traits: serde_json::Value,
}

pub(super) fn load_plugin_config_form(
    plugin_id: &str,
) -> Result<CombinedPluginForm, Box<Response>> {
    let traits = crate::plugins::config::load_plugin_traits(plugin_id);
    let combined = crate::plugins::config::load_combined_contracts(plugin_id)
        .map_err(|_| Box::new(contract_unavailable_response()))?;
    let Some((spec, runtime)) = combined else {
        return Ok(CombinedPluginForm {
            form: empty_resolved_config(),
            runtime: None,
            traits,
        });
    };
    let config = load_plugin_config_value(plugin_id)?;
    let form = qol_config::normalized::resolve_config(&spec, &config)
        .map_err(|errors| Box::new(contract_validation_response(errors)))?;
    Ok(CombinedPluginForm {
        form,
        runtime,
        traits,
    })
}

fn empty_resolved_config() -> qol_config::normalized::ResolvedConfig {
    qol_config::normalized::ResolvedConfig {
        title: None,
        description: None,
        fields: Vec::new(),
        sections: Vec::new(),
    }
}

pub(super) fn validate_plugin_config(
    plugin_id: &str,
    config: &serde_json::Value,
) -> Result<(), Box<Response>> {
    let spec = match crate::plugins::config::load_config_contract(plugin_id)
        .map_err(|_| Box::new(contract_unavailable_response()))?
    {
        Some(spec) => spec,
        None => return Ok(()),
    };
    crate::plugins::config::validate_config_value(&spec, config)
        .map_err(|errors| Box::new(invalid_config_response(errors)))
}

fn load_plugin_config_value(plugin_id: &str) -> Result<serde_json::Value, Box<Response>> {
    let manager = crate::plugins::PluginConfigManager::new()
        .map_err(|_| Box::new(config_read_failed_response()))?;
    let config = manager
        .get_config(plugin_id)
        .map_err(|_| Box::new(config_read_failed_response()))?;
    Ok(config.unwrap_or(serde_json::Value::Null))
}

fn config_read_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read config").into_response()
}

fn contract_unavailable_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to load config contract",
    )
        .into_response()
}

fn contract_validation_response(errors: Vec<qol_config::validation::ValidationError>) -> Response {
    let message = crate::plugins::config::format_validation_errors(errors);
    (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
}

fn invalid_config_response(errors: Vec<qol_config::validation::ValidationError>) -> Response {
    let message = crate::plugins::config::format_validation_errors(errors);
    (StatusCode::BAD_REQUEST, message).into_response()
}

#[cfg(test)]
mod tests {
    use super::{load_plugin_config_form, validate_plugin_config};
    use qol_config::contract::FieldDefault;
    use serde_json::json;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct ConfigEnvGuard {
        home: Option<OsString>,
        xdg_config_home: Option<OsString>,
    }

    impl ConfigEnvGuard {
        fn new(root: &std::path::Path) -> Self {
            let home = std::env::var_os("HOME");
            let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
            let home_dir = root.join("home");
            let xdg_dir = root.join("xdg-config");
            std::fs::create_dir_all(&home_dir).unwrap();
            std::fs::create_dir_all(&xdg_dir).unwrap();
            std::env::set_var("HOME", &home_dir);
            std::env::set_var("XDG_CONFIG_HOME", &xdg_dir);
            Self {
                home,
                xdg_config_home,
            }
        }
    }

    impl Drop for ConfigEnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.home {
                std::env::set_var("HOME", value);
            }
            if self.home.is_none() {
                std::env::remove_var("HOME");
            }
            if let Some(value) = &self.xdg_config_home {
                std::env::set_var("XDG_CONFIG_HOME", value);
            }
            if self.xdg_config_home.is_none() {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }
    }

    fn setup_profile_env() -> (
        tokio::sync::MutexGuard<'static, ()>,
        TempDir,
        ConfigEnvGuard,
    ) {
        let guard = crate::test_support::env_lock().blocking_lock();
        let root = TempDir::new().unwrap();
        let env = ConfigEnvGuard::new(root.path());
        (guard, root, env)
    }

    fn parse_spec(input: &str) -> qol_config::contract::ConfigSpec {
        qol_config::contract::parse_spec_str(input).unwrap()
    }

    fn field_value<'a>(
        resolved: &'a qol_config::normalized::ResolvedConfig,
        field_id: &str,
    ) -> &'a FieldDefault {
        resolved
            .fields
            .iter()
            .find(|field| field.id == field_id)
            .map(|field| &field.value)
            .unwrap()
    }

    #[test]
    fn newer_plugin_contracts_handle_older_saved_configs() {
        let newer_spec = parse_spec(
            r#"
schema_version = 1

[field.enabled]
type = "boolean"
default = true

[field.mode]
type = "string"
default = "auto"
"#,
        );
        let resolved = qol_config::normalized::resolve_config(
            &newer_spec,
            &json!({
                "enabled": false,
                "removed": "legacy"
            }),
        )
        .unwrap();

        assert_eq!(
            field_value(&resolved, "enabled"),
            &FieldDefault::Boolean(false)
        );
        assert_eq!(
            field_value(&resolved, "mode"),
            &FieldDefault::String("auto".to_string())
        );
        assert!(resolved.fields.iter().all(|field| field.id != "removed"));

        let changed_type_spec = parse_spec(
            r#"
schema_version = 1

[field.threshold]
type = "number"
default = 3
"#,
        );
        let resolved = qol_config::normalized::resolve_config(
            &changed_type_spec,
            &json!({
                "threshold": "3"
            }),
        )
        .unwrap();

        assert!(
            matches!(field_value(&resolved, "threshold"), FieldDefault::Number(value) if *value == 3.0)
        );
    }

    #[test]
    fn validate_plugin_config_rejects_wrong_typed_values() {
        let (_guard, _root, _env) = setup_profile_env();
        let plugin_dir = crate::paths::shared_config_dir()
            .unwrap()
            .join("plugins")
            .join("plugin-test");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("qol-config.toml"),
            r#"
schema_version = 1

[field.threshold]
type = "number"
default = 3
"#,
        )
        .unwrap();

        let result = validate_plugin_config("plugin-test", &json!({"threshold": "3"}));

        assert!(result.is_err());
    }

    fn empty_resolved() -> qol_config::normalized::ResolvedConfig {
        qol_config::normalized::ResolvedConfig {
            title: None,
            description: None,
            fields: Vec::new(),
            sections: Vec::new(),
        }
    }

    fn serialized_traits(form: &super::CombinedPluginForm) -> serde_json::Value {
        serde_json::to_value(form).unwrap()["traits"].clone()
    }

    #[test]
    fn combined_plugin_form_serializes_traits_from_manifest() {
        let traits = json!({
            "confined": {},
            "peripheral-preview": { "neighbors": 1 },
            "atmosphere": { "preset": "terminal" },
        });
        let form = super::CombinedPluginForm {
            form: empty_resolved(),
            runtime: None,
            traits: traits.clone(),
        };
        assert_eq!(serialized_traits(&form), traits);
    }

    #[test]
    fn combined_plugin_form_serializes_default_traits_when_manifest_absent() {
        let default_traits = crate::plugins::config::default_plugin_traits();
        let form = super::CombinedPluginForm {
            form: empty_resolved(),
            runtime: None,
            traits: default_traits,
        };
        assert_eq!(serialized_traits(&form), json!({ "confined": {} }));
    }

    #[test]
    fn combined_plugin_form_traits_key_always_present() {
        let form = super::CombinedPluginForm {
            form: empty_resolved(),
            runtime: None,
            traits: crate::plugins::config::default_plugin_traits(),
        };
        let value = serde_json::to_value(&form).unwrap();
        assert!(
            value.get("traits").is_some(),
            "traits must always be serialized, never skipped"
        );
    }

    fn create_plugin_dir(plugin_id: &str) -> std::path::PathBuf {
        let plugin_dir = crate::paths::shared_config_dir()
            .unwrap()
            .join("plugins")
            .join(plugin_id);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        plugin_dir
    }

    #[test]
    fn load_plugin_config_form_returns_traits_only_when_no_config_contract() {
        let (_guard, _root, _env) = setup_profile_env();
        create_plugin_dir("plugin-no-config");

        let form = load_plugin_config_form("plugin-no-config").expect("should not error");

        assert!(form.form.fields.is_empty(), "fields must be empty");
        assert!(form.form.sections.is_empty(), "sections must be empty");
        assert!(form.form.title.is_none());
        assert!(form.form.description.is_none());
        assert!(form.runtime.is_none());
        assert_eq!(form.traits, json!({ "confined": {} }));
    }

    #[test]
    fn load_plugin_config_form_returns_manifest_traits_when_no_config_contract() {
        let (_guard, _root, _env) = setup_profile_env();
        let plugin_dir = create_plugin_dir("plugin-traits-only");
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            r#"
manifest_version = 2

[plugin]
id = "test-plugin"
name = "Traits Only"
description = "No config, traits only"
version = "0.0.0"

[menu]
label = "Traits Only"
items = []

[traits]
confined = {}

[traits.peripheral-preview]
neighbors = 2

[traits.atmosphere]
preset = "wood"
"#,
        )
        .unwrap();

        let form = load_plugin_config_form("plugin-traits-only").expect("should not error");

        assert!(form.form.fields.is_empty());
        assert!(form.form.sections.is_empty());
        assert!(form.runtime.is_none());
        assert_eq!(
            form.traits,
            json!({
                "confined": {},
                "peripheral-preview": { "neighbors": 2 },
                "atmosphere": { "preset": "wood" },
            })
        );
    }

    #[test]
    fn load_plugin_config_form_serves_both_traits_and_config_when_both_present() {
        let (_guard, _root, _env) = setup_profile_env();
        let plugin_dir = create_plugin_dir("plugin-both");
        std::fs::write(
            plugin_dir.join("qol-config.toml"),
            r#"
schema_version = 1

[field.enabled]
type = "boolean"
default = true
"#,
        )
        .unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            r#"
manifest_version = 2

[plugin]
id = "test-plugin"
name = "Both"
description = "Config + traits"
version = "0.0.0"

[menu]
label = "Both"
items = []

[traits.atmosphere]
preset = "terminal"
"#,
        )
        .unwrap();

        let form = load_plugin_config_form("plugin-both").expect("should not error");

        assert_eq!(form.form.fields.len(), 1);
        assert_eq!(form.form.fields[0].id, "enabled");
        assert_eq!(
            form.traits,
            json!({
                "atmosphere": { "preset": "terminal" },
            })
        );
    }

    #[test]
    fn load_plugin_config_form_serialized_always_has_traits_key() {
        let (_guard, _root, _env) = setup_profile_env();
        create_plugin_dir("plugin-serialize");

        let form = load_plugin_config_form("plugin-serialize").expect("should not error");
        let value = serde_json::to_value(&form).unwrap();

        assert!(
            value.get("traits").is_some(),
            "traits key must always appear even for contract-less plugins"
        );
        assert_eq!(value["traits"], json!({ "confined": {} }));
        assert!(
            value.get("fields").is_some(),
            "ResolvedConfig is flattened; fields key must be present"
        );
        assert_eq!(value["fields"], json!([]));
        assert_eq!(value["sections"], json!([]));
    }
}
