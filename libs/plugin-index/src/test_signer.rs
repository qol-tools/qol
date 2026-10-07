use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_compact::{KeyPair, Seed};
use std::sync::atomic::{AtomicU8, Ordering};

const TRUSTED_COMMENT: &str = "qol plugin index";

static NEXT_SEED: AtomicU8 = AtomicU8::new(1);

pub struct TestSigner {
    key_pair: KeyPair,
    key_id: [u8; 8],
}

impl TestSigner {
    pub fn generate() -> Self {
        let seed = NEXT_SEED.fetch_add(1, Ordering::Relaxed);
        Self {
            key_pair: KeyPair::from_seed(Seed::new([seed; 32])),
            key_id: [seed; 8],
        }
    }

    pub fn public_key(&self) -> String {
        STANDARD.encode([b"Ed".as_slice(), &self.key_id, self.key_pair.pk.as_ref()].concat())
    }

    pub fn sign(&self, body: &[u8]) -> String {
        let signature = self
            .key_pair
            .sk
            .sign(blake2b_simd::blake2b(body).as_bytes(), None);
        let global = self.key_pair.sk.sign(
            [signature.as_ref(), TRUSTED_COMMENT.as_bytes()].concat(),
            None,
        );
        format!(
            "untrusted comment: test signature\n{}\ntrusted comment: {TRUSTED_COMMENT}\n{}\n",
            STANDARD.encode([b"ED".as_slice(), &self.key_id, signature.as_ref()].concat()),
            STANDARD.encode(global.as_ref()),
        )
    }

    pub fn sign_index(&self, body: &[u8]) -> Vec<u8> {
        let signed = crate::SignedIndex {
            signature: self.sign(body),
            index: String::from_utf8(body.to_vec()).unwrap_or_default(),
        };
        serde_json::to_vec(&signed).unwrap_or_default()
    }
}
