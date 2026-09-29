use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const COMMAND_VERSION: u8 = 2;

#[derive(Deserialize, Serialize)]
struct Envelope {
    version: u8,
    device_id: String,
    sent_at_ms: u64,
    nonce: String,
    payload: String,
    mac: String,
}

pub(crate) struct ParsedCommand {
    pub(crate) device_id: [u8; 16],
    pub(crate) sent_at_ms: u64,
    pub(crate) nonce: [u8; 16],
    payload: Vec<u8>,
    mac: [u8; 32],
}

pub(crate) fn parse(packet: &[u8]) -> Option<ParsedCommand> {
    let envelope: Envelope = serde_json::from_slice(packet).ok()?;
    if envelope.version != COMMAND_VERSION {
        return None;
    }
    Some(ParsedCommand {
        device_id: decode_array::<16>(&envelope.device_id)?,
        sent_at_ms: envelope.sent_at_ms,
        nonce: decode_array::<16>(&envelope.nonce)?,
        payload: URL_SAFE_NO_PAD.decode(&envelope.payload).ok()?,
        mac: decode_array::<32>(&envelope.mac)?,
    })
}

impl ParsedCommand {
    pub(crate) fn verify(&self, key: &[u8; 32]) -> Option<&[u8]> {
        let mut verifier =
            HmacSha256::new_from_slice(key).expect("HMAC accepts a 32-byte device key");
        update_mac(
            &mut verifier,
            self.sent_at_ms,
            &self.device_id,
            &self.nonce,
            &self.payload,
        );
        verifier.verify_slice(&self.mac).ok()?;
        Some(&self.payload)
    }
}

fn decode_array<const N: usize>(encoded: &str) -> Option<[u8; N]> {
    URL_SAFE_NO_PAD.decode(encoded).ok()?.try_into().ok()
}

fn update_mac(
    mac: &mut HmacSha256,
    sent_at_ms: u64,
    device_id: &[u8; 16],
    nonce: &[u8; 16],
    payload: &[u8],
) {
    mac.update(&[COMMAND_VERSION]);
    mac.update(&sent_at_ms.to_be_bytes());
    mac.update(device_id);
    mac.update(nonce);
    mac.update(payload);
}

#[cfg(test)]
pub(crate) fn seal(
    payload: &[u8],
    key: &[u8; 32],
    device_id: [u8; 16],
    sent_at_ms: u64,
    nonce: [u8; 16],
) -> Vec<u8> {
    let mut signer = HmacSha256::new_from_slice(key).unwrap();
    update_mac(&mut signer, sent_at_ms, &device_id, &nonce, payload);
    let envelope = Envelope {
        version: COMMAND_VERSION,
        device_id: URL_SAFE_NO_PAD.encode(device_id),
        sent_at_ms,
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        payload: URL_SAFE_NO_PAD.encode(payload),
        mac: URL_SAFE_NO_PAD.encode(signer.finalize().into_bytes()),
    };
    serde_json::to_vec(&envelope).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];
    const DEVICE: [u8; 16] = [3; 16];

    #[test]
    fn sealed_command_round_trips_and_authenticates() {
        let payload = br#"{"type":"MouseClick","button":1}"#;
        let packet = seal(payload, &KEY, DEVICE, 123456789, [9; 16]);

        let parsed = parse(&packet).unwrap();

        assert_eq!(parsed.device_id, DEVICE);
        assert_eq!(parsed.sent_at_ms, 123456789);
        assert_eq!(parsed.nonce, [9; 16]);
        assert_eq!(parsed.verify(&KEY).unwrap(), payload);
    }

    #[test]
    fn a_wrong_key_fails_authentication() {
        let packet = seal(
            br#"{"type":"MouseClick","button":1}"#,
            &KEY,
            DEVICE,
            1,
            [1; 16],
        );

        let parsed = parse(&packet).unwrap();

        assert!(parsed.verify(&[8; 32]).is_none());
    }

    #[test]
    fn a_tampered_payload_fails_authentication() {
        let mut packet = seal(
            br#"{"type":"MouseClick","button":1}"#,
            &KEY,
            DEVICE,
            1,
            [1; 16],
        );
        let last = packet.len() - 2;
        packet[last] = if packet[last] == b'a' { b'b' } else { b'a' };

        if let Some(parsed) = parse(&packet) {
            assert!(parsed.verify(&KEY).is_none());
        }
    }

    #[test]
    fn a_v1_envelope_is_rejected() {
        let packet = br#"{"version":1,"sent_at_ms":1,"nonce":"AAAAAAAAAAAAAAAAAAAAAA","payload":"e30","mac":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;

        assert!(parse(packet).is_none());
    }
}
