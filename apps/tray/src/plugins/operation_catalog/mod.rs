#[cfg(test)]
mod tests;

use crate::plugins::{ActionType, Plugin, PluginId, PluginManager, PluginManifest, PluginSource};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use qol_peers::operations::Failure;
use qol_plugin_api::operations::{Operation, OperationCatalog, OperationKey, OperationKind};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone)]
pub(crate) struct PluginDeclaration {
    pub(crate) id: PluginId,
    pub(crate) manifest: PluginManifest,
    pub(crate) path: PathBuf,
    pub(crate) source: PluginSource,
}

impl PluginDeclaration {
    pub(crate) fn capture(plugin: &Plugin) -> Self {
        Self {
            id: plugin.id.clone(),
            manifest: plugin.manifest.clone(),
            path: plugin.path.clone(),
            source: plugin.source.clone(),
        }
    }

    fn matches(&self, plugin: &Plugin) -> bool {
        self.id == plugin.id
            && self.path == plugin.path
            && self.source == plugin.source
            && serde_json::to_vec(&self.manifest).is_ok_and(|manifest| {
                serde_json::to_vec(&plugin.manifest).is_ok_and(|live| live == manifest)
            })
    }
}

pub(crate) struct CatalogCapture {
    plugins: Vec<PluginDeclaration>,
}

impl CatalogCapture {
    pub(crate) fn capture(manager: &PluginManager) -> Self {
        Self {
            plugins: manager.plugins().map(PluginDeclaration::capture).collect(),
        }
    }

    pub(crate) fn resolve(self) -> ResolvedCatalog {
        let (catalog, _) = resolve_catalog(&self.plugins);
        ResolvedCatalog {
            capture: self,
            catalog,
        }
    }

    pub(crate) fn try_resolve(self) -> anyhow::Result<ResolvedCatalog> {
        let (catalog, error) = resolve_catalog(&self.plugins);
        if let Some(error) = error {
            return Err(error);
        }
        Ok(ResolvedCatalog {
            capture: self,
            catalog,
        })
    }
}

pub(crate) struct ResolvedCatalog {
    capture: CatalogCapture,
    catalog: OperationCatalog,
}

impl ResolvedCatalog {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Operation> {
        self.catalog.iter()
    }

    pub(crate) fn select(&self, key: &OperationKey) -> Option<SelectedOperation> {
        let operation = self.catalog.get(key)?.clone();
        let plugin = self
            .capture
            .plugins
            .iter()
            .find(|plugin| plugin.id == operation.plugin_id)?;
        Some(SelectedOperation {
            operation,
            plugin: plugin.clone(),
        })
    }
}

pub(crate) struct SelectedOperation {
    pub(crate) operation: Operation,
    pub(crate) plugin: PluginDeclaration,
}

impl SelectedOperation {
    pub(crate) fn matches_manager(&self, manager: &PluginManager) -> bool {
        let Some(plugin) = manager.get(self.plugin.id.as_str()) else {
            return false;
        };
        self.plugin.matches(plugin)
            && !plugin.manifest.plugin.uid.as_ref().is_some_and(|uid| {
                manager
                    .plugins()
                    .filter(|candidate| candidate.manifest.plugin.uid.as_ref() == Some(uid))
                    .count()
                    != 1
            })
    }

    pub(crate) fn declaration_digest(
        &self,
        artifact: &qol_artifact::InspectedArtifact,
    ) -> Result<String, Failure> {
        let operation = self
            .operation
            .declaration_digest()
            .map_err(|_| Failure::ChangedDeclaration)?;
        let identities: Vec<_> = artifact
            .slices
            .iter()
            .map(|slice| &slice.identity)
            .collect();
        let bytes = serde_json::to_vec(&(
            "qol-selected-operation-v1",
            operation,
            &self.plugin.manifest,
            identities,
        ))
        .map_err(|_| Failure::ChangedDeclaration)?;
        Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)))
    }
}

pub(crate) fn installed_catalog(manager: &PluginManager) -> OperationCatalog {
    CatalogCapture::capture(manager).resolve().catalog
}

fn resolve_catalog(plugins: &[PluginDeclaration]) -> (OperationCatalog, Option<anyhow::Error>) {
    let mut owners = HashMap::new();
    for plugin in plugins {
        if let Some(uid) = &plugin.manifest.plugin.uid {
            *owners.entry(uid.clone()).or_insert(0usize) += 1;
        }
    }
    let mut catalog = OperationCatalog::default();
    let mut read_error = None;
    for plugin in plugins {
        if plugin
            .manifest
            .plugin
            .uid
            .as_ref()
            .is_some_and(|uid| owners[uid] > 1)
        {
            qol_runtime::probe!(
                "TRAY_MCP",
                "event=catalog_skipped plugin={} reason=duplicate_uid",
                plugin.id.as_str()
            );
            continue;
        }
        let plugin_catalog = match catalog_for_plugin(plugin) {
            Ok(Some(catalog)) => catalog,
            Ok(None) => continue,
            Err(error) => {
                read_error = Some(error);
                continue;
            }
        };
        if let Err(conflict) = catalog.extend(plugin_catalog) {
            qol_runtime::probe!(
                "TRAY_MCP",
                "event=catalog_conflict plugin={} kind={} operation={}",
                plugin.id.as_str(),
                conflict.kind.as_str(),
                conflict.name
            );
        }
    }
    (catalog, read_error)
}

fn catalog_for_plugin(plugin: &PluginDeclaration) -> anyhow::Result<Option<OperationCatalog>> {
    if !plugin.manifest.plugin.supports_current_platform() {
        return Ok(None);
    }
    let runtime_declared = match std::fs::symlink_metadata(plugin.path.join("qol-runtime.toml")) {
        Ok(metadata) if metadata.is_file() => true,
        Ok(_) => anyhow::bail!("Runtime contract is not a regular file"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let runtime = match crate::plugins::config::load_runable_contract_from_root(&plugin.path) {
        Ok(runtime) => runtime,
        Err(error) => {
            qol_runtime::probe!(
                "TRAY_MCP",
                "event=contract_skipped plugin={} reason=invalid_runtime",
                plugin.id.as_str()
            );
            return Err(error);
        }
    };
    if runtime.is_none() && runtime_declared {
        anyhow::bail!("Runtime contract became unavailable");
    }
    let mut catalog = match OperationCatalog::derive(&plugin.id, &plugin.manifest, runtime.as_ref())
    {
        Ok(catalog) => catalog,
        Err(error) => {
            qol_runtime::probe!(
                "TRAY_MCP",
                "event=catalog_skipped plugin={} reason=invalid_contract",
                plugin.id.as_str()
            );
            return Err(error.into());
        }
    };
    let daemon = plugin.manifest.daemon.as_ref().is_some_and(|daemon| {
        daemon.enabled
            && daemon
                .socket
                .as_ref()
                .is_some_and(|socket| !socket.trim().is_empty())
            && command_available(plugin, &daemon.command)
    });
    let runtime = plugin
        .manifest
        .runtime
        .as_ref()
        .is_some_and(|runtime| command_available(plugin, &runtime.command));
    catalog.retain(|operation| available(plugin, operation, daemon, runtime));
    Ok(Some(catalog))
}

fn command_available(plugin: &PluginDeclaration, command: &str) -> bool {
    crate::plugins::resolve_plugin_command_path_for_source(
        &plugin.path,
        command,
        Some(&plugin.source),
    )
    .is_some()
}

fn available(
    plugin: &PluginDeclaration,
    operation: &Operation,
    daemon: bool,
    runtime: bool,
) -> bool {
    match operation.key.kind {
        OperationKind::Query | OperationKind::Stream => daemon,
        OperationKind::Action => {
            let daemon = plugin
                .manifest
                .daemon
                .as_ref()
                .is_some_and(|daemon| daemon.enabled && daemon.socket.is_some());
            if !plugin.manifest.actions.is_empty()
                && !plugin
                    .manifest
                    .executable_action_ids()
                    .contains(&operation.key.name)
            {
                return false;
            }
            if operation.runtime_args.is_none() && plugin.manifest.runtime.is_some() {
                return false;
            }
            if plugin.manifest.runtime.is_some() && !runtime && !daemon {
                return false;
            }
            let hosted_settings = operation.action_kind == Some(ActionType::Settings)
                && plugin.manifest.capabilities.gpui
                && plugin.path.join("qol-config.toml").is_file();
            if hosted_settings {
                return true;
            }
            if operation
                .input
                .as_ref()
                .is_some_and(|input| !input.is_empty())
            {
                return daemon;
            }
            daemon || runtime
        }
    }
}
