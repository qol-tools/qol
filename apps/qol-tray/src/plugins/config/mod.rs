mod store;

#[cfg(test)]
mod tests;

use crate::paths;
use crate::paths::is_safe_path_component;
use crate::plugins::paths as plugin_paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfigs {
    #[serde(flatten)]
    pub configs: HashMap<String, serde_json::Value>,
}

pub struct PluginConfigManager {
    configs_dir: PathBuf,
}

impl PluginConfigManager {
    pub fn new() -> Result<Self> {
        let configs_dir = paths::profile_plugin_configs_dir()?;
        Ok(Self { configs_dir })
    }

    fn plugin_config_path(plugin_id: &str) -> Result<PathBuf> {
        if !is_safe_path_component(plugin_id) {
            anyhow::bail!("Invalid plugin ID: {}", plugin_id);
        }
        Ok(paths::config_dir()?
            .join("plugins")
            .join(plugin_id)
            .join("config.json"))
    }

    pub fn load_configs(&self) -> Result<PluginConfigs> {
        let configs = store::load_configs(&self.configs_dir)?;
        Ok(PluginConfigs { configs })
    }

    pub fn save_configs(&self, configs: &PluginConfigs) -> Result<()> {
        store::save_configs(&self.configs_dir, &configs.configs)
    }

    pub fn get_config(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let plugin_path = Self::plugin_config_path(plugin_id)?;
        if plugin_path.exists() {
            return store::load_plugin_config(&plugin_path).map(Some);
        }
        self.restore_from_backup(plugin_id)
    }

    pub fn set_config(&self, plugin_id: &str, config: serde_json::Value) -> Result<()> {
        let plugin_path = Self::plugin_config_path(plugin_id)?;
        store::write_plugin_config(&plugin_path, &config)?;
        store::write_profile_plugin_config(&self.configs_dir, plugin_id, &config)
    }

    fn restore_from_backup(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let config = match store::load_profile_plugin_config(&self.configs_dir, plugin_id)? {
            Some(config) => config,
            None => return Ok(None),
        };
        let plugin_path = Self::plugin_config_path(plugin_id)?;
        log::info!("Restoring config for plugin from backup: {}", plugin_id);
        store::write_plugin_config(&plugin_path, &config)?;
        log::info!("Config restored for plugin: {}", plugin_id);
        Ok(Some(config))
    }
}

pub(crate) fn load_config_contract(
    plugin_id: &str,
) -> Result<Option<qol_config::contract::ConfigSpec>> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id)?;
    load_config_contract_from_root(&plugin_root)
}

pub(crate) fn load_config_contract_from_root(
    plugin_root: &std::path::Path,
) -> Result<Option<qol_config::contract::ConfigSpec>> {
    let contract_path = plugin_paths::config_contract_path(plugin_root);
    if !is_regular_contract_file(&contract_path) {
        return Ok(None);
    }
    qol_config::contract::parse_spec(&contract_path)
        .map(Some)
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

pub(crate) fn load_runable_contract_from_root(
    plugin_root: &std::path::Path,
) -> Result<Option<qol_config::contract::RuntimeSpec>> {
    let runtime_path = plugin_paths::runable_contract_path(plugin_root);
    if !is_regular_contract_file(&runtime_path) {
        return Ok(None);
    }
    qol_config::contract::parse_runtime_spec(&runtime_path)
        .map(Some)
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

pub(crate) fn load_runable_contract(
    plugin_id: &str,
) -> Result<Option<qol_config::contract::RuntimeSpec>> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id)?;
    load_runable_contract_from_root(&plugin_root)
}

pub(crate) fn load_combined_contracts_from_root(
    plugin_root: &std::path::Path,
) -> Result<
    Option<(
        qol_config::contract::ConfigSpec,
        Option<qol_config::contract::RuntimeSpec>,
    )>,
> {
    let Some(config) = load_config_contract_from_root(plugin_root)? else {
        return Ok(None);
    };
    let runtime = load_runable_contract_from_root(plugin_root)?;
    qol_config::contract::validate_contracts(&config, runtime.as_ref()).map_err(|errors| {
        anyhow::anyhow!(
            "contract validation failed:\n{}",
            format_validation_errors(errors)
        )
    })?;
    Ok(Some((config, runtime)))
}

pub(crate) fn load_combined_contracts(
    plugin_id: &str,
) -> Result<
    Option<(
        qol_config::contract::ConfigSpec,
        Option<qol_config::contract::RuntimeSpec>,
    )>,
> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id)?;
    load_combined_contracts_from_root(&plugin_root)
}

pub(crate) fn validate_config_value(
    spec: &qol_config::contract::ConfigSpec,
    config: &serde_json::Value,
) -> std::result::Result<(), Vec<qol_config::validation::ValidationError>> {
    let errors = match qol_config::normalized::resolve_config(spec, config) {
        Ok(_) => strict_validation_errors(spec, config),
        Err(errors) => errors,
    };
    if errors.is_empty() {
        return Ok(());
    }
    Err(errors)
}

pub(crate) fn format_validation_errors(
    errors: Vec<qol_config::validation::ValidationError>,
) -> String {
    errors
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n")
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

fn strict_validation_errors(
    spec: &qol_config::contract::ConfigSpec,
    config: &serde_json::Value,
) -> Vec<qol_config::validation::ValidationError> {
    let mut errors = Vec::new();
    for (id, field) in &spec.fields {
        let config_key = field.config_key.as_deref().unwrap_or(id.as_str());
        let Some(raw) = config_override_value(config, config_key) else {
            continue;
        };
        let Some(value) = field_default_from_override(field.kind, raw) else {
            errors.push(qol_config::validation::ValidationError::new(
                format!("overrides.{id}"),
                format!(
                    "value does not match field type {}",
                    field_kind_name(field.kind)
                ),
            ));
            continue;
        };
        errors.extend(qol_config::validation::validate_field_value(
            &format!("overrides.{id}"),
            field,
            &value,
        ));
    }
    errors
}

fn config_override_value<'a>(
    overrides: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = overrides;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn field_default_from_override(
    kind: qol_config::contract::FieldKind,
    raw: &serde_json::Value,
) -> Option<qol_config::contract::FieldDefault> {
    let value = serde_json::from_value::<qol_config::contract::FieldDefault>(raw.clone()).ok()?;
    field_default_matches_kind(kind, &value).then_some(value)
}

fn field_default_matches_kind(
    kind: qol_config::contract::FieldKind,
    value: &qol_config::contract::FieldDefault,
) -> bool {
    matches!(
        (kind, value),
        (
            qol_config::contract::FieldKind::Boolean,
            qol_config::contract::FieldDefault::Boolean(_),
        ) | (
            qol_config::contract::FieldKind::String,
            qol_config::contract::FieldDefault::String(_),
        ) | (
            qol_config::contract::FieldKind::Select,
            qol_config::contract::FieldDefault::String(_),
        ) | (
            qol_config::contract::FieldKind::Number,
            qol_config::contract::FieldDefault::Number(_),
        ) | (
            qol_config::contract::FieldKind::StringArray,
            qol_config::contract::FieldDefault::StringArray(_),
        ) | (
            qol_config::contract::FieldKind::ObjectArray,
            qol_config::contract::FieldDefault::ObjectArray(_),
        ) | (
            qol_config::contract::FieldKind::ObjectMap,
            qol_config::contract::FieldDefault::ObjectMap(_),
        ) | (
            qol_config::contract::FieldKind::Color,
            qol_config::contract::FieldDefault::String(_),
        ) | (
            qol_config::contract::FieldKind::Action,
            qol_config::contract::FieldDefault::String(_),
        ) | (
            qol_config::contract::FieldKind::List,
            qol_config::contract::FieldDefault::String(_),
        ) | (
            qol_config::contract::FieldKind::Status,
            qol_config::contract::FieldDefault::String(_),
        ) | (
            qol_config::contract::FieldKind::QrCode,
            qol_config::contract::FieldDefault::String(_),
        )
    )
}

fn field_kind_name(kind: qol_config::contract::FieldKind) -> &'static str {
    match kind {
        qol_config::contract::FieldKind::Boolean => "boolean",
        qol_config::contract::FieldKind::String => "string",
        qol_config::contract::FieldKind::Number => "number",
        qol_config::contract::FieldKind::Select => "select",
        qol_config::contract::FieldKind::StringArray => "string_array",
        qol_config::contract::FieldKind::ObjectArray => "object_array",
        qol_config::contract::FieldKind::ObjectMap => "object_map",
        qol_config::contract::FieldKind::Color => "color",
        qol_config::contract::FieldKind::Action => "action",
        qol_config::contract::FieldKind::List => "list",
        qol_config::contract::FieldKind::Status => "status",
        qol_config::contract::FieldKind::QrCode => "qr_code",
    }
}
