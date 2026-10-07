use std::fmt;

use sha2::{Digest, Sha256};

use crate::PeerId;

use super::PeerError;

const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct PeerPin {
    der: [u8; 91],
    id: PeerId,
}

impl PeerPin {
    pub fn from_spki_der(der: &[u8]) -> Result<Self, PeerError> {
        if der.len() != 91 || !der.starts_with(&P256_SPKI_PREFIX) || der[26] != 4 {
            return Err(PeerError::InvalidPin);
        }
        p256::PublicKey::from_sec1_bytes(&der[26..]).map_err(|_| PeerError::InvalidPin)?;
        let der: [u8; 91] = der.try_into().map_err(|_| PeerError::InvalidPin)?;
        Ok(Self {
            id: PeerId::from_spki_digest(Sha256::digest(der).into()),
            der,
        })
    }

    pub fn spki_der(&self) -> &[u8] {
        &self.der
    }

    pub fn peer_id(&self) -> PeerId {
        self.id
    }
}

impl fmt::Debug for PeerPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PeerPin").field(&self.peer_id()).finish()
    }
}
