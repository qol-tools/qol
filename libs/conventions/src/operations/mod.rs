mod syntax;

#[cfg(test)]
mod tests;

pub use syntax::{is_valid_action_id, is_valid_runable_name};

use crate::plugin_id::{PluginId, PluginUid};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationKind {
    Action,
    Query,
    Stream,
}

impl OperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Query => "query",
            Self::Stream => "stream",
        }
    }

    pub fn invocation(self) -> Option<Invocation> {
        match self {
            Self::Action => Some(Invocation::Action),
            Self::Query => Some(Invocation::Query),
            Self::Stream => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Invocation {
    Action,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "scope",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum OperationIdentity {
    Stable(PluginUid),
    Local(PluginId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationKey {
    pub identity: OperationIdentity,
    pub kind: OperationKind,
    pub name: String,
}

impl OperationKey {
    pub fn new(plugin_uid: PluginUid, kind: OperationKind, name: impl Into<String>) -> Self {
        Self {
            identity: OperationIdentity::Stable(plugin_uid),
            kind,
            name: name.into(),
        }
    }
}
