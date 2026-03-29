use axum::{http::StatusCode, response::IntoResponse, response::Response};

pub(super) fn load_plugin_config_form(
    plugin_id: &str,
) -> Result<Option<qol_config::normalized::ResolvedConfig>, Box<Response>> {
    let spec = match crate::plugins::config::load_config_contract(plugin_id)
        .map_err(|_| Box::new(contract_unavailable_response()))?
    {
        Some(spec) => spec,
        None => return Ok(None),
    };
    let config = load_plugin_config_value(plugin_id)?;
    let form = qol_config::normalized::resolve_config(&spec, &config)
        .map_err(|errors| Box::new(contract_validation_response(errors)))?;
    Ok(Some(form))
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
    use super::validate_plugin_config;
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
}
