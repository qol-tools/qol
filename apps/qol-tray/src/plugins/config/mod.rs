mod resolver;
mod scope;
mod store;

pub use resolver::{classify_os_bucket, resolve_plugin_config, PluginConfigResolution};
pub use scope::{merge_slices, split_by_declarations, split_by_scope, ConfigSlices};

#[cfg(test)]
mod tests;

use crate::features::profile::scope_store::PluginConfigSlicePaths;
use crate::features::profile::ProfileScopeStore;
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
    scope_store: ProfileScopeStore,
}

impl PluginConfigManager {
    pub fn new() -> Result<Self> {
        Ok(Self::with_store(ProfileScopeStore::from_active()?))
    }

    pub fn with_store(scope_store: ProfileScopeStore) -> Self {
        Self { scope_store }
    }

    pub fn store(&self) -> &ProfileScopeStore {
        &self.scope_store
    }

    fn plugin_config_path(plugin_id: &str) -> Result<PathBuf> {
        if !is_safe_path_component(plugin_id) {
            anyhow::bail!("Invalid plugin ID: {}", plugin_id);
        }
        Ok(paths::plugins_dir()?.join(plugin_id).join("config.json"))
    }

    pub fn load_configs(&self) -> Result<PluginConfigs> {
        let configs = store::load_configs(&self.scope_store.core_plugin_configs_dir())?;
        Ok(PluginConfigs { configs })
    }

    pub fn save_configs(&self, configs: &PluginConfigs) -> Result<()> {
        store::save_configs(
            &self.scope_store.core_plugin_configs_dir(),
            &configs.configs,
        )
    }

    pub fn get_config(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let lock = load_lock_entry_for(plugin_id);
        let manifest = try_load_plugin_manifest(plugin_id);
        self.get_config_with(plugin_id, lock.as_ref(), manifest.as_ref())
    }

    pub fn get_config_with(
        &self,
        plugin_id: &str,
        lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
        manifest: Option<&crate::plugins::manifest::PluginManifest>,
    ) -> Result<Option<serde_json::Value>> {
        let runtime_path = Self::plugin_config_path(plugin_id)?;
        if runtime_path.exists() {
            return store::load_plugin_config(&runtime_path).map(Some);
        }
        let merged = load_plugin_config_merged(&self.scope_store, plugin_id, lock_entry, manifest)?;
        if merged.as_object().is_some_and(|m| m.is_empty()) {
            return Ok(None);
        }
        log::info!("Restoring config for plugin from scoped profile slices: {plugin_id}");
        store::write_plugin_config(&runtime_path, &merged)?;
        Ok(Some(merged))
    }

    pub fn set_config(&self, plugin_id: &str, config: serde_json::Value) -> Result<()> {
        let lock = load_lock_entry_for(plugin_id);
        let manifest = try_load_plugin_manifest(plugin_id);
        self.set_config_with(plugin_id, config, lock.as_ref(), manifest.as_ref())
    }

    pub fn set_config_with(
        &self,
        plugin_id: &str,
        config: serde_json::Value,
        lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
        manifest: Option<&crate::plugins::manifest::PluginManifest>,
    ) -> Result<()> {
        let runtime_path = Self::plugin_config_path(plugin_id)?;
        store::write_plugin_config(&runtime_path, &config)?;
        save_plugin_config_split(&self.scope_store, plugin_id, &config, lock_entry, manifest)
    }
}

fn load_lock_entry_for(plugin_id: &str) -> Option<crate::features::profile::core::PluginLockEntry> {
    let lock = crate::features::profile::core::load_plugins_lock().ok()?;
    lock.plugins.into_iter().find(|entry| entry.id == plugin_id)
}

fn try_load_plugin_manifest(plugin_id: &str) -> Option<crate::plugins::manifest::PluginManifest> {
    let plugin_root = plugin_paths::resolve_plugin_root(plugin_id).ok()?;
    let manifest_path = plugin_root.join("plugin.toml");
    let content = std::fs::read_to_string(&manifest_path).ok()?;
    toml::from_str::<crate::plugins::manifest::PluginManifest>(&content).ok()
}

pub fn load_plugin_config_merged(
    scope_store: &ProfileScopeStore,
    plugin_id: &str,
    lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
    manifest: Option<&crate::plugins::manifest::PluginManifest>,
) -> Result<serde_json::Value> {
    let paths = scope_store.plugin_config_slice_paths(plugin_id, lock_entry, manifest)?;
    let slices = store::read_scoped_slices(&paths)?;
    Ok(merge_slices(&slices))
}

pub fn save_plugin_config_split(
    scope_store: &ProfileScopeStore,
    plugin_id: &str,
    config: &serde_json::Value,
    lock_entry: Option<&crate::features::profile::core::PluginLockEntry>,
    manifest: Option<&crate::plugins::manifest::PluginManifest>,
) -> Result<()> {
    let paths: PluginConfigSlicePaths =
        scope_store.plugin_config_slice_paths(plugin_id, lock_entry, manifest)?;
    let default_decl = crate::plugins::manifest::ConfigDeclarations::default();
    let decl = manifest.map(|m| &m.config).unwrap_or(&default_decl);
    let slices = split_by_declarations(config, decl);
    store::write_scoped_slices(&paths, &slices)
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

/// Default traits served when a plugin manifest does not declare any.
/// Matches the frontend fallback in `ui/components/App.js`.
pub(crate) fn default_plugin_traits() -> serde_json::Value {
    serde_json::json!({ "confined": {} })
}

pub(crate) fn load_plugin_traits_from_root(plugin_root: &std::path::Path) -> serde_json::Value {
    read_manifest_traits(plugin_root).unwrap_or_else(default_plugin_traits)
}

pub(crate) fn load_plugin_traits(plugin_id: &str) -> serde_json::Value {
    let plugin_root = match plugin_paths::resolve_plugin_root(plugin_id) {
        Ok(root) => root,
        Err(_) => return default_plugin_traits(),
    };
    load_plugin_traits_from_root(&plugin_root)
}

fn read_manifest_traits(plugin_root: &std::path::Path) -> Option<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct TraitsOnly {
        traits: Option<serde_json::Value>,
    }
    let manifest_path = plugin_root.join("plugin.toml");
    let content = std::fs::read_to_string(&manifest_path).ok()?;
    let parsed: TraitsOnly = toml::from_str(&content).ok()?;
    let traits = parsed.traits?;
    if !traits.is_object() {
        return None;
    }
    Some(traits)
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
    use qol_config::contract::{FieldDefault, FieldKind};
    match kind {
        FieldKind::Boolean => matches!(value, FieldDefault::Boolean(_)),
        FieldKind::String | FieldKind::Select | FieldKind::Color => {
            matches!(value, FieldDefault::String(_))
        }
        FieldKind::Number => matches!(value, FieldDefault::Number(_)),
        FieldKind::StringArray => matches!(value, FieldDefault::StringArray(_)),
        FieldKind::ObjectArray => matches!(value, FieldDefault::ObjectArray(_)),
        FieldKind::ObjectMap => matches!(value, FieldDefault::ObjectMap(_)),
        FieldKind::Action | FieldKind::List | FieldKind::Status | FieldKind::QrCode => false,
    }
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
