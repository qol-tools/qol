use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Clone)]
pub(crate) struct Seed(Zeroizing<[u8; 32]>);

impl Seed {
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub(crate) fn generate() -> Option<Self> {
        crate::service::enrollment::random::<32>()
            .ok()
            .map(Self::from_bytes)
    }

    pub(crate) fn expose(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn server_id(&self) -> String {
        let digest = Sha256::digest(self.0.as_slice());
        URL_SAFE_NO_PAD.encode(&digest[..12])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_id_matches_the_legacy_pointz_derivation() {
        let seed = Seed::from_bytes([7; 32]);
        let digest = Sha256::digest([7_u8; 32]);

        assert_eq!(seed.server_id(), URL_SAFE_NO_PAD.encode(&digest[..12]));
        assert_eq!(seed.server_id().len(), 16);
    }
}
