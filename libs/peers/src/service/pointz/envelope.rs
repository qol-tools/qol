use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const HMAC_VERSION: u8 = 2;
const SEALED_VERSION: u8 = 3;
const HMAC_NONCE_BYTES: usize = 16;
const SEALED_NONCE_BYTES: usize = 12;

#[derive(Deserialize, Serialize)]
struct Envelope {
    version: u8,
    device_id: String,
    sent_at_ms: u64,
    nonce: String,
    payload: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mac: Option<String>,
}

enum Protection {
    Hmac([u8; 32]),
    Sealed,
}

pub(crate) struct ParsedCommand {
    pub(crate) device_id: [u8; 16],
    pub(crate) sent_at_ms: u64,
    pub(crate) nonce: Vec<u8>,
    payload: Vec<u8>,
    protection: Protection,
}

pub(crate) fn parse(packet: &[u8]) -> Option<ParsedCommand> {
    let envelope: Envelope = serde_json::from_slice(packet).ok()?;
    let (nonce_bytes, protection) = match (envelope.version, envelope.mac.as_deref()) {
        (HMAC_VERSION, Some(mac)) => (HMAC_NONCE_BYTES, Protection::Hmac(decode_array(mac)?)),
        (SEALED_VERSION, None) => (SEALED_NONCE_BYTES, Protection::Sealed),
        _ => return None,
    };
    let nonce = URL_SAFE_NO_PAD.decode(&envelope.nonce).ok()?;
    if nonce.len() != nonce_bytes {
        return None;
    }
    Some(ParsedCommand {
        device_id: decode_array::<16>(&envelope.device_id)?,
        sent_at_ms: envelope.sent_at_ms,
        nonce,
        payload: URL_SAFE_NO_PAD.decode(&envelope.payload).ok()?,
        protection,
    })
}

impl ParsedCommand {
    pub(crate) fn is_sealed(&self) -> bool {
        matches!(self.protection, Protection::Sealed)
    }

    pub(crate) fn open(&self, key: &[u8; 32]) -> Option<Vec<u8>> {
        match &self.protection {
            Protection::Hmac(mac) => self.verify_hmac(key, mac),
            Protection::Sealed => self.decrypt(key),
        }
    }

    fn verify_hmac(&self, key: &[u8; 32], mac: &[u8; 32]) -> Option<Vec<u8>> {
        let mut verifier =
            HmacSha256::new_from_slice(key).expect("HMAC accepts a 32-byte device key");
        update_mac(
            &mut verifier,
            self.sent_at_ms,
            &self.device_id,
            &self.nonce,
            &self.payload,
        );
        verifier.verify_slice(mac).ok()?;
        Some(self.payload.clone())
    }

    fn decrypt(&self, key: &[u8; 32]) -> Option<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new_from_slice(key).expect("ChaCha20Poly1305 32-byte key");
        cipher
            .decrypt(
                Nonce::from_slice(&self.nonce),
                Payload {
                    msg: &self.payload,
                    aad: &sealed_aad(self.sent_at_ms, &self.device_id),
                },
            )
            .ok()
    }
}

fn decode_array<const N: usize>(encoded: &str) -> Option<[u8; N]> {
    URL_SAFE_NO_PAD.decode(encoded).ok()?.try_into().ok()
}

fn update_mac(
    mac: &mut HmacSha256,
    sent_at_ms: u64,
    device_id: &[u8; 16],
    nonce: &[u8],
    payload: &[u8],
) {
    mac.update(&[HMAC_VERSION]);
    mac.update(&sent_at_ms.to_be_bytes());
    mac.update(device_id);
    mac.update(nonce);
    mac.update(payload);
}

fn sealed_aad(sent_at_ms: u64, device_id: &[u8; 16]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(1 + 8 + 16);
    aad.push(SEALED_VERSION);
    aad.extend_from_slice(&sent_at_ms.to_be_bytes());
    aad.extend_from_slice(device_id);
    aad
}

#[cfg(test)]
pub(crate) fn seal_v2(
    payload: &[u8],
    key: &[u8; 32],
    device_id: [u8; 16],
    sent_at_ms: u64,
    nonce: [u8; 16],
) -> Vec<u8> {
    let mut signer = HmacSha256::new_from_slice(key).unwrap();
    update_mac(&mut signer, sent_at_ms, &device_id, &nonce, payload);
    let envelope = Envelope {
        version: HMAC_VERSION,
        device_id: URL_SAFE_NO_PAD.encode(device_id),
        sent_at_ms,
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        payload: URL_SAFE_NO_PAD.encode(payload),
        mac: Some(URL_SAFE_NO_PAD.encode(signer.finalize().into_bytes())),
    };
    serde_json::to_vec(&envelope).unwrap()
}

#[cfg(test)]
pub(crate) fn seal_v3(
    payload: &[u8],
    key: &[u8; 32],
    device_id: [u8; 16],
    sent_at_ms: u64,
    nonce: [u8; 12],
) -> Vec<u8> {
    let sealed = ChaCha20Poly1305::new_from_slice(key)
        .unwrap()
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: payload,
                aad: &sealed_aad(sent_at_ms, &device_id),
            },
        )
        .unwrap();
    let envelope = Envelope {
        version: SEALED_VERSION,
        device_id: URL_SAFE_NO_PAD.encode(device_id),
        sent_at_ms,
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        payload: URL_SAFE_NO_PAD.encode(sealed),
        mac: None,
    };
    serde_json::to_vec(&envelope).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];
    const DEVICE: [u8; 16] = [3; 16];
    const PAYLOAD: &[u8] = br#"{"type":"MouseClick","button":1}"#;

    #[test]
    fn a_v2_command_round_trips_and_authenticates() {
        let packet = seal_v2(PAYLOAD, &KEY, DEVICE, 123456789, [9; 16]);

        let parsed = parse(&packet).unwrap();

        assert_eq!(parsed.device_id, DEVICE);
        assert_eq!(parsed.sent_at_ms, 123456789);
        assert_eq!(parsed.nonce, [9; 16]);
        assert!(!parsed.is_sealed());
        assert_eq!(parsed.open(&KEY).unwrap(), PAYLOAD);
    }

    #[test]
    fn a_v3_command_round_trips_and_decrypts() {
        let packet = seal_v3(PAYLOAD, &KEY, DEVICE, 123456789, [9; 12]);

        let parsed = parse(&packet).unwrap();

        assert_eq!(parsed.device_id, DEVICE);
        assert_eq!(parsed.sent_at_ms, 123456789);
        assert_eq!(parsed.nonce, [9; 12]);
        assert!(parsed.is_sealed());
        assert_eq!(parsed.open(&KEY).unwrap(), PAYLOAD);
    }

    #[test]
    fn a_v3_payload_is_not_readable_on_the_wire() {
        let packet = seal_v3(PAYLOAD, &KEY, DEVICE, 1, [1; 12]);
        let text = String::from_utf8(packet).unwrap();

        assert!(!text.contains("MouseClick"));
        assert!(!text.contains(&URL_SAFE_NO_PAD.encode(PAYLOAD)));
    }

    #[test]
    fn a_wrong_key_fails_for_both_versions() {
        let v2 = seal_v2(PAYLOAD, &KEY, DEVICE, 1, [1; 16]);
        let v3 = seal_v3(PAYLOAD, &KEY, DEVICE, 1, [1; 12]);

        assert!(parse(&v2).unwrap().open(&[8; 32]).is_none());
        assert!(parse(&v3).unwrap().open(&[8; 32]).is_none());
    }

    #[test]
    fn a_tampered_payload_fails_for_both_versions() {
        let cases = [
            seal_v2(PAYLOAD, &KEY, DEVICE, 1, [1; 16]),
            seal_v3(PAYLOAD, &KEY, DEVICE, 1, [1; 12]),
        ];
        for packet in cases {
            let mut envelope: serde_json::Value = serde_json::from_slice(&packet).unwrap();
            let mut payload = URL_SAFE_NO_PAD
                .decode(envelope["payload"].as_str().unwrap())
                .unwrap();
            payload[0] ^= 1;
            envelope["payload"] = serde_json::json!(URL_SAFE_NO_PAD.encode(payload));
            let tampered = serde_json::to_vec(&envelope).unwrap();

            assert!(parse(&tampered).unwrap().open(&KEY).is_none());
        }
    }

    #[test]
    fn a_v3_envelope_bound_to_other_metadata_fails_to_decrypt() {
        let packet = seal_v3(PAYLOAD, &KEY, DEVICE, 1_000, [1; 12]);
        let mut envelope: serde_json::Value = serde_json::from_slice(&packet).unwrap();
        let retimed = {
            envelope["sent_at_ms"] = serde_json::json!(1_001);
            serde_json::to_vec(&envelope).unwrap()
        };
        let mut envelope: serde_json::Value = serde_json::from_slice(&packet).unwrap();
        envelope["device_id"] = serde_json::json!(URL_SAFE_NO_PAD.encode([4u8; 16]));
        let redirected = serde_json::to_vec(&envelope).unwrap();

        assert!(parse(&retimed).unwrap().open(&KEY).is_none());
        assert!(parse(&redirected).unwrap().open(&KEY).is_none());
    }

    #[test]
    fn a_v3_envelope_downgraded_to_v2_or_with_a_mac_is_rejected() {
        let packet = seal_v3(PAYLOAD, &KEY, DEVICE, 1, [1; 12]);
        let mut envelope: serde_json::Value = serde_json::from_slice(&packet).unwrap();
        envelope["mac"] = serde_json::json!(URL_SAFE_NO_PAD.encode([0u8; 32]));
        let with_mac = serde_json::to_vec(&envelope).unwrap();
        envelope.as_object_mut().unwrap().remove("mac");
        envelope["version"] = serde_json::json!(2);
        let relabelled = serde_json::to_vec(&envelope).unwrap();

        assert!(parse(&with_mac).is_none());
        assert!(parse(&relabelled).is_none());
    }

    #[test]
    fn nonce_lengths_must_match_the_version() {
        let v3 = seal_v3(PAYLOAD, &KEY, DEVICE, 1, [1; 12]);
        let mut envelope: serde_json::Value = serde_json::from_slice(&v3).unwrap();
        envelope["nonce"] = serde_json::json!(URL_SAFE_NO_PAD.encode([1u8; 16]));
        let long_v3_nonce = serde_json::to_vec(&envelope).unwrap();
        let v2 = seal_v2(PAYLOAD, &KEY, DEVICE, 1, [1; 16]);
        let mut envelope: serde_json::Value = serde_json::from_slice(&v2).unwrap();
        envelope["nonce"] = serde_json::json!(URL_SAFE_NO_PAD.encode([1u8; 12]));
        let short_v2_nonce = serde_json::to_vec(&envelope).unwrap();

        assert!(parse(&long_v3_nonce).is_none());
        assert!(parse(&short_v2_nonce).is_none());
    }

    #[test]
    fn a_v1_envelope_is_rejected() {
        let packet = br#"{"version":1,"sent_at_ms":1,"nonce":"AAAAAAAAAAAAAAAAAAAAAA","payload":"e30","mac":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;

        assert!(parse(packet).is_none());
    }
}
