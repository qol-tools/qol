use std::{
    fmt,
    io::{self, Write},
    marker::PhantomData,
    sync::Arc,
    time::SystemTime,
};

use qol_conventions::operations::OperationKey;
use serde::{
    de::{SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use zeroize::Zeroizing;

use super::enrollment::{InboundReceipt, OutboundJoin, MAX_OUTBOUND, MAX_RECEIPTS};
use super::{
    state::{LinkedPeer, State, MAX_GRANTS, MAX_PEERS, MAX_SNAPSHOT_BYTES, MAX_TOMBSTONES},
    AuthorityError,
};
use crate::enrollment::{EnrollmentReceipt, InvitationId, OutboundEnrollmentState, TransactionId};
use crate::service::{Identity, PeerPin, SecretKeyBytes, MAX_CERTIFICATE_BYTES};
use crate::{AuthorityLifetime, PeerId, StoreRevision};

const VERSION: u32 = 4;
const MAX_SECRET_BYTES: usize = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    revision: StoreRevision,
    identity: StoredIdentity,
    name: String,
    peers: Bounded<StoredPeer, MAX_PEERS>,
    tombstones: Bounded<StoredPin, MAX_TOMBSTONES>,
    receipts: Bounded<StoredReceipt, MAX_RECEIPTS>,
    outbound: Bounded<StoredOutbound, MAX_OUTBOUND>,
    #[serde(default)]
    operations: Option<Bounded<super::operations::LinkOperations, 512>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredIdentity {
    peer_id: PeerId,
    certificate: Bounded<u8, MAX_CERTIFICATE_BYTES>,
    key: Secret,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPin {
    peer_id: PeerId,
    spki: Bounded<u8, 91>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPeer {
    pin: StoredPin,
    name: String,
    grants: Bounded<OperationKey, MAX_GRANTS>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReceipt {
    pin: StoredPin,
    receipt: EnrollmentReceipt,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOutbound {
    invitation: InvitationId,
    transaction: TransactionId,
    pin: StoredPin,
    name: String,
    local_lifetime: AuthorityLifetime,
    remote_lifetime: AuthorityLifetime,
    state: OutboundEnrollmentState,
}

#[derive(Serialize)]
#[serde(transparent)]
struct Bounded<T, const N: usize>(Vec<T>);

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Bounded<T, N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BoundedVisitor<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for BoundedVisitor<T, N> {
            type Value = Bounded<T, N>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded sequence")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element()? {
                    if values.len() == N {
                        return Err(serde::de::Error::custom("sequence capacity exceeded"));
                    }
                    values.push(value);
                }
                Ok(Bounded(values))
            }
        }
        deserializer.deserialize_seq(BoundedVisitor::<T, N>(PhantomData))
    }
}

struct Secret(Zeroizing<Vec<u8>>);

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter())
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SecretVisitor;
        impl<'de> Visitor<'de> for SecretVisitor {
            type Value = Secret;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("bounded private key bytes")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Secret, A::Error> {
                let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_SECRET_BYTES));
                while let Some(byte) = sequence.next_element::<u8>()? {
                    if bytes.len() == MAX_SECRET_BYTES {
                        return Err(serde::de::Error::custom("private key capacity exceeded"));
                    }
                    bytes.push(byte);
                }
                Ok(Secret(bytes))
            }
        }
        deserializer.deserialize_seq(SecretVisitor)
    }
}

impl StoredPin {
    fn from_pin(pin: &PeerPin) -> Self {
        Self {
            peer_id: pin.peer_id(),
            spki: Bounded(pin.spki_der().to_vec()),
        }
    }

    fn into_pin(self) -> Result<PeerPin, AuthorityError> {
        let pin =
            PeerPin::from_spki_der(&self.spki.0).map_err(|_| AuthorityError::InvalidSnapshot)?;
        if pin.peer_id() != self.peer_id {
            return Err(AuthorityError::InvalidSnapshot);
        }
        Ok(pin)
    }
}

pub(super) fn decode(bytes: &[u8], now: SystemTime) -> Result<(State, bool), AuthorityError> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(AuthorityError::Capacity);
    }
    let snapshot: Snapshot =
        serde_json::from_slice(bytes).map_err(|_| AuthorityError::InvalidSnapshot)?;
    if !matches!(snapshot.version, 3 | VERSION) {
        return Err(AuthorityError::UnsupportedVersion);
    }
    let (identity, renewed) = Identity::restore_or_renew(
        snapshot.identity.certificate.0,
        SecretKeyBytes::from_pkcs8(snapshot.identity.key.0),
        now,
    )
    .map_err(|_| AuthorityError::Identity)?;
    if identity.pin().peer_id() != snapshot.identity.peer_id {
        return Err(AuthorityError::InvalidSnapshot);
    }
    let migrating = snapshot.version == 3;
    if migrating && snapshot.operations.is_some() {
        return Err(AuthorityError::InvalidSnapshot);
    }
    if !migrating && snapshot.operations.is_none() {
        return Err(AuthorityError::InvalidSnapshot);
    }
    let mut state = State {
        operations: snapshot
            .operations
            .map(|stored| stored.0)
            .unwrap_or_default(),
        identity: Arc::new(identity),
        name: snapshot.name,
        revision: snapshot.revision,
        peers: snapshot
            .peers
            .0
            .into_iter()
            .map(|peer| {
                Ok(LinkedPeer {
                    pin: peer.pin.into_pin()?,
                    name: peer.name,
                    grants: peer.grants.0,
                })
            })
            .collect::<Result<_, AuthorityError>>()?,
        receipts: snapshot
            .receipts
            .0
            .into_iter()
            .map(|stored| {
                Ok(InboundReceipt {
                    pin: stored.pin.into_pin()?,
                    receipt: stored.receipt,
                })
            })
            .collect::<Result<_, AuthorityError>>()?,
        outbound: snapshot
            .outbound
            .0
            .into_iter()
            .map(|stored| {
                Ok(OutboundJoin {
                    invitation: stored.invitation,
                    transaction: stored.transaction,
                    pin: stored.pin.into_pin()?,
                    name: stored.name,
                    local_lifetime: stored.local_lifetime,
                    remote_lifetime: stored.remote_lifetime,
                    state: stored.state,
                })
            })
            .collect::<Result<_, AuthorityError>>()?,
        tombstones: snapshot
            .tombstones
            .0
            .into_iter()
            .map(StoredPin::into_pin)
            .collect::<Result<_, _>>()?,
    };
    if migrating {
        state.synchronize_operations()?;
    }
    state.validate()?;
    if state
        .receipts
        .iter()
        .any(|entry| entry.receipt.inviter_lifetime != AuthorityLifetime::Persistent)
        || state
            .outbound
            .iter()
            .any(|entry| entry.local_lifetime != AuthorityLifetime::Persistent)
    {
        return Err(AuthorityError::InvalidSnapshot);
    }
    Ok((state, renewed || migrating))
}

pub(super) fn encode(state: &State) -> Result<Zeroizing<Vec<u8>>, AuthorityError> {
    state.validate()?;
    let secret = state.identity.export_secret();
    let snapshot = Snapshot {
        version: VERSION,
        operations: Some(Bounded(state.operations.clone())),
        revision: state.revision,
        identity: StoredIdentity {
            peer_id: state.identity.pin().peer_id(),
            certificate: Bounded(state.identity.certificate_der().to_vec()),
            key: Secret(Zeroizing::new(secret.expose_pkcs8().to_vec())),
        },
        name: state.name.clone(),
        peers: Bounded(
            state
                .peers
                .iter()
                .map(|peer| StoredPeer {
                    pin: StoredPin::from_pin(&peer.pin),
                    name: peer.name.clone(),
                    grants: Bounded(peer.grants.clone()),
                })
                .collect(),
        ),
        tombstones: Bounded(state.tombstones.iter().map(StoredPin::from_pin).collect()),
        receipts: Bounded(
            state
                .receipts
                .iter()
                .map(|entry| StoredReceipt {
                    pin: StoredPin::from_pin(&entry.pin),
                    receipt: entry.receipt.clone(),
                })
                .collect(),
        ),
        outbound: Bounded(
            state
                .outbound
                .iter()
                .map(|entry| StoredOutbound {
                    invitation: entry.invitation,
                    transaction: entry.transaction,
                    pin: StoredPin::from_pin(&entry.pin),
                    name: entry.name.clone(),
                    local_lifetime: entry.local_lifetime,
                    remote_lifetime: entry.remote_lifetime,
                    state: entry.state.clone(),
                })
                .collect(),
        ),
    };
    let mut counter = SizeCounter(0);
    serde_json::to_writer(&mut counter, &snapshot).map_err(|_| AuthorityError::Capacity)?;
    let reserved = state.validate_operations()?;
    if counter.0 + reserved > MAX_SNAPSHOT_BYTES - 1024 * 1024 {
        return Err(AuthorityError::Capacity);
    }
    let mut bytes = Zeroizing::new(vec![0; counter.0]);
    serde_json::to_writer(io::Cursor::new(bytes.as_mut_slice()), &snapshot)
        .map_err(|_| AuthorityError::InvalidSnapshot)?;
    Ok(bytes)
}

struct SizeCounter(usize);

impl Write for SizeCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let size = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_SNAPSHOT_BYTES)
            .ok_or_else(|| io::Error::other("snapshot capacity exceeded"))?;
        self.0 = size;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
