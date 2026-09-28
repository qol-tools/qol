use serde::{Deserialize, Serialize};

use super::{ActivationId, PageCursor};
use crate::{network::NetworkStatus, session::SessionGeneration, PeerId};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct NetworkRevision(pub crate::StoreRevision);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCursor {
    pub authority_id: PeerId,
    pub activation_id: ActivationId,
    pub revision: crate::StoreRevision,
    pub network_revision: NetworkRevision,
    pub offset: u32,
}

impl SessionCursor {
    pub fn authority_cursor(self) -> PageCursor {
        PageCursor {
            authority_id: self.authority_id,
            activation_id: self.activation_id,
            revision: self.revision,
            offset: self.offset,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSummary {
    pub peer_id: PeerId,
    pub name: String,
    pub generation: SessionGeneration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPage {
    pub cursor: SessionCursor,
    pub total: u32,
    pub items: Vec<SessionSummary>,
    pub next: Option<SessionCursor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSummary {
    pub authority: super::ExpectedAuthority,
    pub network_revision: NetworkRevision,
    pub status: NetworkStatus,
}
