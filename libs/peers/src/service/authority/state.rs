use std::{collections::HashSet, sync::Arc};

use qol_conventions::{
    operations::{
        is_valid_action_id, is_valid_runable_name, OperationIdentity, OperationKey, OperationKind,
    },
    plugin_id::is_valid_plugin_uid,
};

use zeroize::Zeroizing;

use crate::pointz::{PointzDeviceId, PointzImport};
use crate::service::pointz::{Seed, MAX_DEVICES};
use crate::service::{Identity, PeerPin};
use crate::{PeerId, StoreRevision};

use super::AuthorityError;

pub(super) const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_PEERS: usize = 256;
pub(super) const MAX_GRANTS: usize = 128;
const MAX_OPERATION_BYTES: usize = 256;
const MAX_UID_BYTES: usize = 256;

#[derive(Clone)]
pub(super) struct LinkedPeer {
    pub pin: PeerPin,
    pub name: String,
    pub grants: Vec<OperationKey>,
}

#[derive(Clone)]
pub(super) struct PointzDeviceRecord {
    pub id: PointzDeviceId,
    pub key: Zeroizing<[u8; 32]>,
    pub name: String,
    pub paired_at_ms: u64,
}

#[derive(Clone)]
pub(super) struct PointzState {
    pub seed: Seed,
    pub devices: Vec<PointzDeviceRecord>,
    pub import: PointzImport,
}

impl PointzState {
    fn validate(&self) -> Result<(), AuthorityError> {
        if self.devices.len() > MAX_DEVICES {
            return Err(AuthorityError::Capacity);
        }
        let mut ids = HashSet::new();
        if !self.devices.iter().all(|device| ids.insert(device.id)) {
            return Err(AuthorityError::InvalidSnapshot);
        }
        self.devices
            .iter()
            .try_for_each(|device| validate_name(&device.name))
    }
}

#[derive(Clone)]
pub(super) struct State {
    pub identity: Arc<Identity>,
    pub name: String,
    pub revision: StoreRevision,
    pub peers: Vec<LinkedPeer>,
    pub receipts: Vec<super::enrollment::InboundReceipt>,
    pub outbound: Vec<super::enrollment::OutboundJoin>,
    pub operations: Vec<super::operations::LinkOperations>,
    pub pointz: Option<PointzState>,
}

impl State {
    pub fn peer(&self, id: PeerId) -> Option<&LinkedPeer> {
        self.peers.iter().find(|peer| peer.pin.peer_id() == id)
    }

    pub fn validate(&self) -> Result<(), AuthorityError> {
        validate_name(&self.name)?;
        if self.peers.len() > MAX_PEERS {
            return Err(AuthorityError::Capacity);
        }
        let mut identities = HashSet::new();
        identities.insert(self.identity.pin().peer_id());
        for peer in &self.peers {
            if !identities.insert(peer.pin.peer_id()) {
                return Err(AuthorityError::InvalidSnapshot);
            }
            validate_name(&peer.name)?;
            validate_grants(&peer.grants)?;
        }
        self.validate_operations()?;
        if let Some(pointz) = &self.pointz {
            pointz.validate()?;
        }
        self.validate_enrollment()
    }
}

pub(super) fn validate_name(name: &str) -> Result<(), AuthorityError> {
    if !crate::is_valid_name(name) {
        return Err(AuthorityError::InvalidName);
    }
    Ok(())
}

pub(super) fn sanitize_name(name: &str, fallback: &str) -> String {
    let mut bounded = String::new();
    for character in name.chars().filter(|character| !character.is_control()) {
        if bounded.len() + character.len_utf8() > crate::MAX_NAME_BYTES {
            break;
        }
        bounded.push(character);
    }
    let trimmed = bounded.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed.to_string()
}

pub(super) fn validate_grants(grants: &[OperationKey]) -> Result<(), AuthorityError> {
    if grants.len() > MAX_GRANTS {
        return Err(AuthorityError::Capacity);
    }
    let mut unique = HashSet::new();
    for grant in grants {
        validate_grant(grant)?;
        if !unique.insert(grant) {
            return Err(AuthorityError::DuplicateGrant);
        }
    }
    Ok(())
}

pub(super) fn validate_grant(grant: &OperationKey) -> Result<(), AuthorityError> {
    let OperationIdentity::Stable(uid) = &grant.identity else {
        return Err(AuthorityError::InvalidGrant);
    };
    if uid.as_str().len() > MAX_UID_BYTES
        || !is_valid_plugin_uid(uid.as_str())
        || uid.as_str().contains('*')
        || grant.name.len() > MAX_OPERATION_BYTES
    {
        return Err(AuthorityError::InvalidGrant);
    }
    let valid = match grant.kind {
        OperationKind::Action => is_valid_action_id(&grant.name),
        OperationKind::Query | OperationKind::Stream => is_valid_runable_name(&grant.name),
    };
    if !valid {
        return Err(AuthorityError::InvalidGrant);
    }
    Ok(())
}
