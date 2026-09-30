use qol_peers::admin::Error;
use qol_runtime::protocol::PeerAdminClientError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct CatalogOperation {
    pub(crate) key: qol_plugin_api::operations::OperationKey,
    pub(crate) plugin_id: crate::plugins::PluginId,
    pub(crate) description: String,
}

pub(crate) fn catalog_operations(
    catalog: &crate::plugins::operation_catalog::ResolvedCatalog,
) -> Vec<CatalogOperation> {
    catalog
        .iter()
        .filter(|operation| operation.peer.is_some())
        .map(|operation| CatalogOperation {
            key: operation.key.clone(),
            plugin_id: operation.plugin_id.clone(),
            description: operation.description.clone(),
        })
        .collect()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "error", rename_all = "snake_case")]
pub(crate) enum Failure {
    Authority(Error),
    Transport(String),
    OutcomeUnknown,
    Inconsistent,
}

impl From<PeerAdminClientError> for Failure {
    fn from(error: PeerAdminClientError) -> Self {
        match error {
            PeerAdminClientError::OutcomeUnknown => Self::OutcomeUnknown,
            other => Self::Transport(other.to_string()),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct InvitationInfo {
    pub invitation: qol_peers::enrollment::InvitationId,
    pub peer: qol_peers::PeerId,
    pub endpoints: Vec<std::net::SocketAddr>,
}

pub(crate) fn invitation_info(
    document: &qol_peers::enrollment::ExportedInvitation,
) -> Result<InvitationInfo, Error> {
    let invitation = qol_peers::service::enrollment::Invitation::import(document.expose())?;
    Ok(InvitationInfo {
        invitation: invitation.id(),
        peer: invitation.inviter_pin().peer_id(),
        endpoints: invitation.endpoints().to_vec(),
    })
}
