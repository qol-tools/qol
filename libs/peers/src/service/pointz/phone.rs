use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::scalar::Scalar;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};

use super::vector::hex_array;

const M_HEX: &str = "d048032c6ea0b6d697ddc2e86bda85a33adac920f1bf18e1b0c6d166a5cecdaf";
const N_HEX: &str = "d3bfb518f44f3430f29d0c92af503865a1ed3281dc69b35dd868ba85f886c4ab";

pub(crate) struct Derived {
    pub(super) shared: [u8; 32],
    pub(super) transcript: Vec<u8>,
    pub(super) ke: [u8; 16],
    pub(super) ka: [u8; 16],
    pub(super) kc_a: [u8; 16],
    pub(super) kc_b: [u8; 16],
    pub(super) confirm_a: [u8; 32],
    pub(super) confirm_b: [u8; 32],
    pub(super) k_wrap: [u8; 32],
}

pub(crate) struct TestClient {
    device: [u8; 16],
    server_id: String,
    pub(super) w: Scalar,
    x: Scalar,
    pub(super) pa: [u8; 32],
}

impl TestClient {
    pub(crate) fn new(device: [u8; 16], pin: &str, server_id: &str) -> Self {
        let wide = crate::service::enrollment::random::<64>().unwrap();
        Self::with_secret(
            device,
            pin,
            server_id,
            Scalar::from_bytes_mod_order_wide(&wide),
        )
    }

    pub(super) fn with_secret(device: [u8; 16], pin: &str, server_id: &str, x: Scalar) -> Self {
        let mut password = b"pointz-spake2-v2".to_vec();
        prefixed(&mut password, pin.as_bytes());
        prefixed(&mut password, &device);
        prefixed(&mut password, server_id.as_bytes());
        let digest = Sha512::digest(&password);
        let mut wide = [0u8; 64];
        wide.copy_from_slice(&digest);
        let w = Scalar::from_bytes_mod_order_wide(&wide);
        let share = ED25519_BASEPOINT_POINT * x + point(M_HEX) * w;
        Self {
            device,
            server_id: server_id.to_string(),
            w,
            x,
            pa: share.compress().to_bytes(),
        }
    }

    pub(crate) fn hello(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "PairHello",
            "protocol": 2,
            "device_id": URL_SAFE_NO_PAD.encode(self.device),
            "name": "TestPhone",
            "pa": URL_SAFE_NO_PAD.encode(self.pa),
        }))
        .unwrap()
    }

    pub(crate) fn derive(&self, offer: &serde_json::Value) -> Derived {
        assert_eq!(offer["type"], "PairOffer");
        assert_eq!(offer["protocol"], 2);
        assert_eq!(offer["server_id"], self.server_id);
        let pb: [u8; 32] = URL_SAFE_NO_PAD
            .decode(offer["pb"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let salt: [u8; 16] = URL_SAFE_NO_PAD
            .decode(offer["salt"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        self.derive_from(&pb, &salt)
    }

    pub(super) fn derive_from(&self, pb: &[u8; 32], salt: &[u8; 16]) -> Derived {
        let pb_point = CompressedEdwardsY(*pb).decompress().unwrap();
        let shared = ((pb_point - point(N_HEX) * self.w) * self.x)
            .mul_by_cofactor()
            .compress()
            .to_bytes();
        let mut transcript = Vec::new();
        prefixed(&mut transcript, &self.device);
        prefixed(&mut transcript, self.server_id.as_bytes());
        prefixed(&mut transcript, &self.pa);
        prefixed(&mut transcript, pb);
        prefixed(&mut transcript, &shared);
        prefixed(&mut transcript, &self.w.to_bytes());
        let digest = Sha256::digest(&transcript);
        let ke: [u8; 16] = digest[..16].try_into().unwrap();
        let ka: [u8; 16] = digest[16..].try_into().unwrap();
        let mut confirmation = [0u8; 32];
        Hkdf::<Sha256>::new(Some(&[]), &ka)
            .expand(b"ConfirmationKeys", &mut confirmation)
            .unwrap();
        let kc_a: [u8; 16] = confirmation[..16].try_into().unwrap();
        let kc_b: [u8; 16] = confirmation[16..].try_into().unwrap();
        let mut k_wrap = [0u8; 32];
        Hkdf::<Sha256>::new(Some(salt), &ke)
            .expand(b"pointz-pair-wrap-v2", &mut k_wrap)
            .unwrap();
        Derived {
            confirm_a: tag(&kc_a, &transcript),
            confirm_b: tag(&kc_b, &transcript),
            shared,
            transcript,
            ke,
            ka,
            kc_a,
            kc_b,
            k_wrap,
        }
    }

    pub(crate) fn confirm(&self, derived: &Derived) -> Vec<u8> {
        self.confirm_bytes(derived.confirm_a)
    }

    pub(crate) fn confirm_bytes(&self, confirm: [u8; 32]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "PairConfirm",
            "protocol": 2,
            "device_id": URL_SAFE_NO_PAD.encode(self.device),
            "client_confirm": URL_SAFE_NO_PAD.encode(confirm),
        }))
        .unwrap()
    }

    pub(crate) fn open_key(
        &self,
        result: &serde_json::Value,
        derived: &Derived,
    ) -> Option<[u8; 32]> {
        if result["protocol"] != 2 {
            return None;
        }
        let server_confirm: [u8; 32] = URL_SAFE_NO_PAD
            .decode(result["server_confirm"].as_str()?)
            .ok()?
            .try_into()
            .ok()?;
        if server_confirm != derived.confirm_b {
            return None;
        }
        let nonce: [u8; 12] = URL_SAFE_NO_PAD
            .decode(result["wrap_nonce"].as_str()?)
            .ok()?
            .try_into()
            .ok()?;
        let sealed = URL_SAFE_NO_PAD.decode(result["sealed"].as_str()?).ok()?;
        let mut aad = self.device.to_vec();
        aad.extend_from_slice(self.server_id.as_bytes());
        ChaCha20Poly1305::new_from_slice(&derived.k_wrap)
            .ok()?
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &sealed,
                    aad: &aad,
                },
            )
            .ok()?
            .try_into()
            .ok()
    }
}

fn point(encoded: &str) -> curve25519_dalek::edwards::EdwardsPoint {
    CompressedEdwardsY(hex_array::<32>(encoded))
        .decompress()
        .unwrap()
}

fn prefixed(out: &mut Vec<u8>, part: &[u8]) {
    out.extend_from_slice(&(part.len() as u64).to_le_bytes());
    out.extend_from_slice(part);
}

fn tag(key: &[u8; 16], transcript: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
    mac.update(transcript);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::super::vector::{self, hex, hex_array};
    use super::*;

    #[test]
    fn the_phone_role_reproduces_every_value_of_the_shared_vector() {
        let client = TestClient::with_secret(
            vector::DEVICE,
            vector::PIN,
            vector::SERVER_ID,
            Scalar::from_bytes_mod_order(vector::X),
        );
        assert_eq!(client.w.to_bytes(), hex_array::<32>(vector::W));
        assert_eq!(client.pa, hex_array::<32>(vector::PA));

        let derived = client.derive_from(&hex_array::<32>(vector::PB), &vector::SALT);

        assert_eq!(derived.shared, hex_array::<32>(vector::K));
        assert_eq!(derived.transcript, hex(vector::TT));
        assert_eq!(derived.ke, hex_array::<16>(vector::KE));
        assert_eq!(derived.ka, hex_array::<16>(vector::KA));
        assert_eq!(derived.kc_a, hex_array::<16>(vector::KC_A));
        assert_eq!(derived.kc_b, hex_array::<16>(vector::KC_B));
        assert_eq!(derived.confirm_a, hex_array::<32>(vector::CONFIRM_A));
        assert_eq!(derived.confirm_b, hex_array::<32>(vector::CONFIRM_B));
        assert_eq!(derived.k_wrap, hex_array::<32>(vector::K_WRAP));
    }
}
