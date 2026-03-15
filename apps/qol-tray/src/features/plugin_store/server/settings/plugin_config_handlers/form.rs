use axum::{http::StatusCode, response::IntoResponse, response::Response};

use crate::plugins::paths as plugin_paths;

pub(super) fn load_plugin_config_form(
    plugin_id: &str,
) -> Result<Option<qol_config::normalized::ResolvedConfig>, Box<Response>> {
    let spec = match load_plugin_contract(plugin_id)? {
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
    let spec = match load_plugin_contract(plugin_id)? {
        Some(spec) => spec,
        None => return Ok(()),
    };
    let errors = match qol_config::normalized::resolve_config(&spec, config) {
        Ok(_) => return Ok(()),
        Err(errors) => errors,
    };
    Err(Box::new(invalid_config_response(errors)))
}

fn load_plugin_contract(
    plugin_id: &str,
) -> Result<Option<qol_config::contract::ConfigSpec>, Box<Response>> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id)
        .map_err(|_| Box::new(contract_unavailable_response()))?;
    let contract_path = plugin_paths::config_contract_path(&plugin_root);
    if !is_regular_contract_file(&contract_path) {
        return Ok(None);
    }
    qol_config::contract::parse_spec(&contract_path)
        .map(Some)
        .map_err(|error| {
            log::error!(
                "Failed to parse config contract for {} at {}: {:?}",
                plugin_id,
                contract_path.display(),
                error
            );
            Box::new(contract_unavailable_response())
        })
}

fn is_regular_contract_file(path: &std::path::Path) -> bool {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if metadata.file_type().is_symlink() {
        return false;
    }
    metadata.is_file()
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
    let message = format_validation_errors(errors);
    (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
}

fn invalid_config_response(errors: Vec<qol_config::validation::ValidationError>) -> Response {
    let message = format_validation_errors(errors);
    (StatusCode::BAD_REQUEST, message).into_response()
}

fn format_validation_errors(errors: Vec<qol_config::validation::ValidationError>) -> String {
    errors
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
