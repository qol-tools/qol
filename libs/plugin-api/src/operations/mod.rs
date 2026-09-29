mod digest;
#[cfg(test)]
mod shipped_tests;
#[cfg(test)]
mod tests;

pub use qol_conventions::operations::{Invocation, OperationIdentity, OperationKey, OperationKind};

#[cfg(test)]
use crate::manifest::PluginUid;
use crate::manifest::{ActionType, PluginId, PluginManifest};
use indexmap::IndexMap;
use qol_config::contract::{PeerExposure, RuntimeSpec};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub key: OperationKey,
    pub plugin_id: PluginId,
    pub description: String,
    pub tool_description: Option<String>,
    pub input: Option<IndexMap<String, String>>,
    pub runtime_args: Option<Vec<String>>,
    pub action_kind: Option<ActionType>,
    pub agent_tool: bool,
    pub peer: Option<PeerExposure>,
}

impl Operation {
    pub fn tool_description(&self) -> &str {
        match self.tool_description.as_deref() {
            Some(text) if !text.trim().is_empty() => text,
            _ => &self.description,
        }
    }

    pub fn invocation(&self) -> Option<Invocation> {
        self.key.kind.invocation()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationCatalogError {
    PeerExposureWithoutStableUid { plugin_id: PluginId, name: String },
    ActivationOwnsRuntimePeerExposure { plugin_id: PluginId, name: String },
    InvalidContract { plugin_id: PluginId, reason: String },
    PeerExposureOnNonRunnable { plugin_id: PluginId, name: String },
    RuntimeActionNotExecutable { plugin_id: PluginId, name: String },
}

impl Display for OperationCatalogError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PeerExposureWithoutStableUid { plugin_id, name } => write!(
                f,
                "operation {name} in {plugin_id} declares peer exposure without a frozen plugin uid"
            ),
            Self::ActivationOwnsRuntimePeerExposure { plugin_id, name } => write!(
                f,
                "operation {name} in {plugin_id} must declare peer exposure only in its typed runtime entry"
            ),
            Self::InvalidContract { plugin_id, reason } => write!(
                f,
                "invalid operation contract in {plugin_id}: {reason}"
            ),
            Self::PeerExposureOnNonRunnable { plugin_id, name } => write!(
                f,
                "operation {name} in {plugin_id} cannot expose peer access unless kind is run"
            ),
            Self::RuntimeActionNotExecutable { plugin_id, name } => write!(
                f,
                "runtime action {name} in {plugin_id} shadows a non-executable action declaration"
            ),
        }
    }
}

impl std::error::Error for OperationCatalogError {}

#[derive(Debug, Clone, Default)]
pub struct OperationCatalog {
    entries: IndexMap<OperationKey, Operation>,
}

impl OperationCatalog {
    pub fn derive(
        plugin_id: &PluginId,
        manifest: &PluginManifest,
        runtime: Option<&RuntimeSpec>,
    ) -> Result<Self, OperationCatalogError> {
        manifest
            .validate()
            .map_err(|error| OperationCatalogError::InvalidContract {
                plugin_id: plugin_id.clone(),
                reason: error.to_string(),
            })?;
        if let Some(runtime) = runtime {
            qol_config::contract::validate_runtime_spec(runtime).map_err(|error| {
                OperationCatalogError::InvalidContract {
                    plugin_id: plugin_id.clone(),
                    reason: format!("{error:?}"),
                }
            })?;
        }
        let stable_uid = manifest.plugin.uid.as_ref();
        let identity = match stable_uid {
            Some(uid) => OperationIdentity::Stable(uid.clone()),
            None => OperationIdentity::Local(plugin_id.clone()),
        };
        let key = |kind, name: &str| OperationKey {
            identity: identity.clone(),
            kind,
            name: name.to_string(),
        };
        let mut catalog = Self::default();

        if let Some(runtime) = runtime {
            for (name, spec) in &runtime.queries {
                ensure_peer_identity(stable_uid.is_some(), plugin_id, name, &spec.peer)?;
                catalog.insert(Operation {
                    key: key(OperationKind::Query, name),
                    plugin_id: plugin_id.clone(),
                    description: spec.description.clone(),
                    tool_description: spec.tool_description.clone(),
                    input: spec.input.clone(),
                    runtime_args: None,
                    action_kind: None,
                    agent_tool: spec.agent_tool,
                    peer: spec.peer.clone(),
                });
            }
            for (name, spec) in &runtime.actions {
                let declaration = manifest.actions.get(name.as_str());
                ensure_runtime_action_is_dispatchable(plugin_id, name, declaration)?;
                let activation = manifest
                    .executable_actions()
                    .into_iter()
                    .find(|action| action.id == *name);
                if spec.peer.is_some()
                    && activation
                        .as_ref()
                        .is_some_and(|action| action.kind != ActionType::Run)
                {
                    return Err(OperationCatalogError::PeerExposureOnNonRunnable {
                        plugin_id: plugin_id.clone(),
                        name: name.clone(),
                    });
                }
                if declaration.is_some_and(|declaration| declaration.peer.is_some()) {
                    return Err(OperationCatalogError::ActivationOwnsRuntimePeerExposure {
                        plugin_id: plugin_id.clone(),
                        name: name.clone(),
                    });
                }
                let args = resolved_runtime_args(manifest, name);
                ensure_peer_identity(stable_uid.is_some(), plugin_id, name, &spec.peer)?;
                catalog.insert(Operation {
                    key: key(OperationKind::Action, name),
                    plugin_id: plugin_id.clone(),
                    description: spec.description.clone(),
                    tool_description: spec.tool_description.clone(),
                    input: spec.input.clone(),
                    runtime_args: args,
                    action_kind: Some(activation.map_or(ActionType::Run, |entry| entry.kind)),
                    agent_tool: spec.agent_tool,
                    peer: spec.peer.clone(),
                });
            }
        }

        for action in manifest.executable_actions() {
            let key = key(OperationKind::Action, &action.id);
            if catalog.contains_key(&key) {
                continue;
            }
            let declaration = manifest.actions.get(action.id.as_str());
            let peer = declaration.and_then(|declaration| declaration.peer.clone());
            if peer.is_some()
                && declaration.is_some_and(|declaration| declaration.kind != ActionType::Run)
            {
                return Err(OperationCatalogError::PeerExposureOnNonRunnable {
                    plugin_id: plugin_id.clone(),
                    name: action.id,
                });
            }
            ensure_peer_identity(stable_uid.is_some(), plugin_id, &action.id, &peer)?;
            catalog.insert(Operation {
                key,
                plugin_id: plugin_id.clone(),
                description: action.label,
                tool_description: None,
                input: None,
                runtime_args: resolved_runtime_args(manifest, &action.id),
                action_kind: Some(action.kind),
                agent_tool: false,
                peer,
            });
        }

        if let Some(runtime) = runtime {
            for (name, spec) in &runtime.streams {
                ensure_peer_identity(stable_uid.is_some(), plugin_id, name, &spec.peer)?;
                catalog.insert(Operation {
                    key: key(OperationKind::Stream, name),
                    plugin_id: plugin_id.clone(),
                    description: spec.description.clone(),
                    tool_description: None,
                    input: None,
                    runtime_args: None,
                    action_kind: None,
                    agent_tool: false,
                    peer: spec.peer.clone(),
                });
            }
        }

        Ok(catalog)
    }

    pub fn extend(&mut self, other: OperationCatalog) -> Result<(), OperationKey> {
        for key in other.entries.keys() {
            if self
                .entries
                .keys()
                .any(|existing| existing.identity == key.identity)
            {
                return Err(key.clone());
            }
        }
        self.entries.extend(other.entries);
        Ok(())
    }

    pub fn retain(&mut self, mut available: impl FnMut(&Operation) -> bool) {
        self.entries.retain(|_, operation| available(operation));
    }

    pub fn get(&self, key: &OperationKey) -> Option<&Operation> {
        self.entries.get(key)
    }

    pub fn contains_key(&self, key: &OperationKey) -> bool {
        self.entries.contains_key(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Operation> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn insert(&mut self, operation: Operation) {
        self.entries.insert(operation.key.clone(), operation);
    }
}

fn ensure_peer_identity(
    stable_identity: bool,
    plugin_id: &PluginId,
    name: &str,
    peer: &Option<PeerExposure>,
) -> Result<(), OperationCatalogError> {
    if peer.is_none() || stable_identity {
        return Ok(());
    }
    Err(OperationCatalogError::PeerExposureWithoutStableUid {
        plugin_id: plugin_id.clone(),
        name: name.to_string(),
    })
}

fn ensure_runtime_action_is_dispatchable(
    plugin_id: &PluginId,
    name: &str,
    declaration: Option<&crate::manifest::ActionDeclaration>,
) -> Result<(), OperationCatalogError> {
    let Some(declaration) = declaration else {
        return Ok(());
    };
    if declaration.kind.is_executable() {
        return Ok(());
    }
    Err(OperationCatalogError::RuntimeActionNotExecutable {
        plugin_id: plugin_id.clone(),
        name: name.to_string(),
    })
}

fn resolved_runtime_args(manifest: &PluginManifest, name: &str) -> Option<Vec<String>> {
    if let Some(args) = manifest.catalog_runtime_args(name) {
        return Some(args);
    }
    match manifest
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.actions.as_ref())
    {
        Some(actions) => actions.get(name).cloned(),
        None => Some(vec![name.to_string()]),
    }
}
