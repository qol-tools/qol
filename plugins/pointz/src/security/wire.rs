use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const VERSION: u8 = 1;

#[derive(Deserialize, Serialize)]
struct Envelope {
    version: u8,
    sent_at_ms: u64,
    nonce: String,
    payload: String,
    mac: String,
}

pub struct VerifiedPacket {
    pub sent_at_ms: u64,
    pub nonce: [u8; 16],
    pub payload: Vec<u8>,
}

pub fn verify(packet: &[u8], secret: &[u8]) -> Result<VerifiedPacket> {
    let envelope: Envelope = serde_json::from_slice(packet)?;
    if envelope.version != VERSION {
        anyhow::bail!("unsupported PointZ command version");
    }
    let nonce = decode_array::<16>(&envelope.nonce, "nonce")?;
    let payload = URL_SAFE_NO_PAD
        .decode(&envelope.payload)
        .context("command payload is not valid base64url")?;
    let mac = decode_array::<32>(&envelope.mac, "MAC")?;
    let mut verifier = HmacSha256::new_from_slice(secret)?;
    update_mac(
        &mut verifier,
        envelope.version,
        envelope.sent_at_ms,
        &nonce,
        &payload,
    );
    verifier
        .verify_slice(&mac)
        .map_err(|_| anyhow::anyhow!("command authentication failed"))?;
    Ok(VerifiedPacket {
        sent_at_ms: envelope.sent_at_ms,
        nonce,
        payload,
    })
}

fn decode_array<const N: usize>(encoded: &str, label: &str) -> Result<[u8; N]> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .with_context(|| format!("command {label} is not valid base64url"))?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("command {label} has the wrong length"))
}

fn update_mac(
    mac: &mut HmacSha256,
    version: u8,
    sent_at_ms: u64,
    nonce: &[u8; 16],
    payload: &[u8],
) {
    mac.update(&[version]);
    mac.update(&sent_at_ms.to_be_bytes());
    mac.update(nonce);
    mac.update(payload);
}

#[cfg(test)]
pub fn seal(payload: &[u8], secret: &[u8], sent_at_ms: u64, nonce: [u8; 16]) -> Vec<u8> {
    let mut signer = HmacSha256::new_from_slice(secret).unwrap();
    update_mac(&mut signer, VERSION, sent_at_ms, &nonce, payload);
    let envelope = Envelope {
        version: VERSION,
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

    #[test]
    fn flutter_protocol_fixture_verifies() {
        let packet = br#"{"version":1,"sent_at_ms":123456789,"nonce":"CQkJCQkJCQkJCQkJCQkJCQ","payload":"eyJ0eXBlIjoiTW91c2VDbGljayIsImJ1dHRvbiI6MX0","mac":"l3JZ067Bos8PQMsGBJKXsSgXFYY4hebSozcV6mi9UXU"}"#;
        let secret: Vec<u8> = (0..32).collect();

        let verified = verify(packet, &secret).unwrap();

        assert_eq!(verified.sent_at_ms, 123456789);
        assert_eq!(verified.nonce, [9; 16]);
        assert_eq!(verified.payload, br#"{"type":"MouseClick","button":1}"#);
    }
}
