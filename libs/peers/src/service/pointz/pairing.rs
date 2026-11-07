use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use curve25519_dalek::scalar::Scalar;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::spake2;

pub(crate) const WINDOW_SECS: u64 = 60;
const MAX_ATTEMPTS: u32 = 3;
const MAX_PENDING: usize = 16;
const PROTOCOL: u64 = 2;

const UNAVAILABLE: &str = "unavailable";
#[derive(Deserialize)]
#[serde(tag = "type")]
enum WireRequest {
    #[serde(rename = "PairHello")]
    Hello {
        #[serde(default)]
        protocol: u64,
        #[serde(default)]
        device_id: String,
        #[serde(default)]
        pa: String,
        #[serde(default)]
        name: Option<String>,
    },
    #[serde(rename = "PairConfirm")]
    Confirm {
        #[serde(default)]
        protocol: u64,
        #[serde(default)]
        device_id: String,
        #[serde(default)]
        client_confirm: String,
    },
}

impl WireRequest {
    fn protocol(&self) -> u64 {
        match self {
            Self::Hello { protocol, .. } | Self::Confirm { protocol, .. } => *protocol,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum WireResponse<'a> {
    #[serde(rename = "PairOffer")]
    Offer {
        protocol: u64,
        server_id: &'a str,
        pb: String,
        salt: String,
    },
    #[serde(rename = "PairResult")]
    Granted {
        protocol: u64,
        server_confirm: String,
        wrap_nonce: String,
        sealed: String,
    },
    #[serde(rename = "PairError")]
    Denied {
        reason: &'a str,
        attempts_left: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected: Option<u64>,
    },
}

struct Pending {
    confirm_a: [u8; 32],
    confirm_b: [u8; 32],
    k_wrap: Zeroizing<[u8; 32]>,
    name: String,
}

pub(crate) struct PairingSession {
    pin: String,
    y: Zeroizing<Scalar>,
    salt: [u8; 16],
    opened_at: Instant,
    attempts_left: u32,
    pending: HashMap<[u8; 16], Pending>,
}

pub(crate) enum HandleOutcome {
    Reply(Vec<u8>),
    Paired {
        device_id: [u8; 16],
        device_key: [u8; 32],
        name: String,
        reply: Vec<u8>,
    },
    Closed(Vec<u8>),
    Ignore,
}

impl PairingSession {
    pub(crate) fn open() -> Option<Self> {
        Some(Self {
            pin: generate_pin()?,
            y: Zeroizing::new(spake2::random_scalar()?),
            salt: random_bytes::<16>()?,
            opened_at: Instant::now(),
            attempts_left: MAX_ATTEMPTS,
            pending: HashMap::new(),
        })
    }

    pub(crate) fn pin(&self) -> &str {
        &self.pin
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.opened_at.elapsed() >= Duration::from_secs(WINDOW_SECS)
    }

    pub(crate) fn seconds_remaining(&self) -> u32 {
        Duration::from_secs(WINDOW_SECS)
            .saturating_sub(self.opened_at.elapsed())
            .as_secs()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    pub(crate) fn handle(&mut self, server_id: &str, datagram: &[u8]) -> HandleOutcome {
        let Ok(request) = serde_json::from_slice::<WireRequest>(datagram) else {
            return HandleOutcome::Ignore;
        };
        if self.is_expired() {
            return HandleOutcome::Closed(error(REASON_CLOSED, 0));
        }
        if request.protocol() != PROTOCOL {
            return HandleOutcome::Reply(protocol_error(self.attempts_left));
        }
        match request {
            WireRequest::Hello {
                device_id,
                pa,
                name,
                ..
            } => self.on_hello(server_id, device_id, pa, name),
            WireRequest::Confirm {
                device_id,
                client_confirm,
                ..
            } => self.on_confirm(server_id, device_id, client_confirm),
        }
    }

    fn on_hello(
        &mut self,
        server_id: &str,
        device_id: String,
        pa: String,
        name: Option<String>,
    ) -> HandleOutcome {
        let (Some(device_id), Some(pa)) = (decode16(&device_id), decode32(&pa)) else {
            return HandleOutcome::Reply(error("bad_request", self.attempts_left));
        };
        if self.pending.len() >= MAX_PENDING && !self.pending.contains_key(&device_id) {
            return HandleOutcome::Reply(error("busy", self.attempts_left));
        }
        let Some(responder) =
            spake2::respond(&self.y, &self.pin, &device_id, server_id, &pa, &self.salt)
        else {
            return HandleOutcome::Reply(error("bad_request", self.attempts_left));
        };
        self.pending.insert(
            device_id,
            Pending {
                confirm_a: responder.confirm_a,
                confirm_b: responder.confirm_b,
                k_wrap: responder.k_wrap,
                name: name.unwrap_or_default(),
            },
        );
        HandleOutcome::Reply(to_bytes(&WireResponse::Offer {
            protocol: PROTOCOL,
            server_id,
            pb: URL_SAFE_NO_PAD.encode(responder.pb),
            salt: URL_SAFE_NO_PAD.encode(self.salt),
        }))
    }

    fn on_confirm(
        &mut self,
        server_id: &str,
        device_id: String,
        client_confirm: String,
    ) -> HandleOutcome {
        let (Some(device_id), Some(client_confirm)) =
            (decode16(&device_id), decode32(&client_confirm))
        else {
            return HandleOutcome::Reply(error("bad_request", self.attempts_left));
        };
        let Some(pending) = self.pending.get(&device_id) else {
            return HandleOutcome::Reply(error("no_session", self.attempts_left));
        };
        if !bool::from(pending.confirm_a[..].ct_eq(&client_confirm[..])) {
            self.attempts_left = self.attempts_left.saturating_sub(1);
            let reply = error("bad_pin", self.attempts_left);
            return if self.attempts_left == 0 {
                HandleOutcome::Closed(reply)
            } else {
                HandleOutcome::Reply(reply)
            };
        }
        let Some((device_key, (wrap_nonce, sealed))) =
            random_bytes::<32>().and_then(|device_key| {
                Some((
                    device_key,
                    seal(&pending.k_wrap, &device_key, &device_id, server_id)?,
                ))
            })
        else {
            return HandleOutcome::Reply(error(UNAVAILABLE, self.attempts_left));
        };
        let reply = to_bytes(&WireResponse::Granted {
            protocol: PROTOCOL,
            server_confirm: URL_SAFE_NO_PAD.encode(pending.confirm_b),
            wrap_nonce: URL_SAFE_NO_PAD.encode(wrap_nonce),
            sealed: URL_SAFE_NO_PAD.encode(sealed),
        });
        HandleOutcome::Paired {
            device_id,
            device_key,
            name: pending.name.clone(),
            reply,
        }
    }
}

fn confirm_aad(device_id: &[u8; 16], server_id: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(16 + server_id.len());
    aad.extend_from_slice(device_id);
    aad.extend_from_slice(server_id.as_bytes());
    aad
}

fn seal(
    k_wrap: &[u8; 32],
    device_key: &[u8; 32],
    device_id: &[u8; 16],
    server_id: &str,
) -> Option<([u8; 12], Vec<u8>)> {
    let nonce_bytes = random_bytes::<12>()?;
    let cipher = ChaCha20Poly1305::new_from_slice(k_wrap).expect("ChaCha20Poly1305 32-byte key");
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: device_key,
                aad: &confirm_aad(device_id, server_id),
            },
        )
        .ok()?;
    Some((nonce_bytes, sealed))
}

fn generate_pin() -> Option<String> {
    loop {
        let value = u32::from_le_bytes(random_bytes::<4>()?);
        if value < 4_294_000_000 {
            return Some(format!("{:06}", value % 1_000_000));
        }
    }
}

fn random_bytes<const N: usize>() -> Option<[u8; N]> {
    crate::service::enrollment::random::<N>().ok()
}

fn decode16(encoded: &str) -> Option<[u8; 16]> {
    URL_SAFE_NO_PAD.decode(encoded).ok()?.try_into().ok()
}

fn decode32(encoded: &str) -> Option<[u8; 32]> {
    URL_SAFE_NO_PAD.decode(encoded).ok()?.try_into().ok()
}

fn to_bytes(response: &WireResponse) -> Vec<u8> {
    serde_json::to_vec(response).expect("pairing responses always serialize")
}

pub(crate) fn unavailable() -> Vec<u8> {
    error(UNAVAILABLE, 0)
}

fn error(reason: &str, attempts_left: u32) -> Vec<u8> {
    to_bytes(&WireResponse::Denied {
        reason,
        attempts_left,
        expected: None,
    })
}

fn protocol_error(attempts_left: u32) -> Vec<u8> {
    to_bytes(&WireResponse::Denied {
        reason: "protocol",
        attempts_left,
        expected: Some(PROTOCOL),
    })
}

const REASON_CLOSED: &str = "closed";

#[cfg(test)]
mod tests {
    use super::super::phone::TestClient;
    use super::*;

    const SERVER_ID: &str = "test-server-id01";
    const DEVICE: [u8; 16] = [4; 16];

    fn reply_json(outcome: &HandleOutcome) -> serde_json::Value {
        let bytes = match outcome {
            HandleOutcome::Reply(bytes)
            | HandleOutcome::Closed(bytes)
            | HandleOutcome::Paired { reply: bytes, .. } => bytes,
            HandleOutcome::Ignore => panic!("expected a reply"),
        };
        serde_json::from_slice(bytes).unwrap()
    }

    fn wrong_pin(real: &str) -> String {
        format!("{:06}", (real.parse::<u32>().unwrap() + 1) % 1_000_000)
    }

    #[test]
    fn a_correct_pin_pairs_and_delivers_a_decryptable_device_key() {
        let mut session = PairingSession::open().unwrap();
        let client = TestClient::new(DEVICE, session.pin(), SERVER_ID);

        let offer = reply_json(&session.handle(SERVER_ID, &client.hello()));
        let derived = client.derive(&offer);
        let outcome = session.handle(SERVER_ID, &client.confirm(&derived));

        let HandleOutcome::Paired {
            device_id,
            device_key,
            reply,
            name,
        } = outcome
        else {
            panic!("expected Paired");
        };
        assert_eq!(device_id, DEVICE);
        assert_eq!(name, "TestPhone");
        let result: serde_json::Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(result["type"], "PairResult");
        assert_eq!(result["protocol"], 2);
        assert_eq!(client.open_key(&result, &derived), Some(device_key));
    }

    #[test]
    fn a_phone_that_bound_the_password_to_another_server_identity_fails_confirmation() {
        let mut session = PairingSession::open().unwrap();
        let client = TestClient::new(DEVICE, session.pin(), "another-server-id");
        let offer = reply_json(&session.handle(SERVER_ID, &client.hello()));
        let derived = client.derive_from(
            &decode32(offer["pb"].as_str().unwrap()).unwrap(),
            &session.salt,
        );

        let outcome = session.handle(SERVER_ID, &client.confirm(&derived));

        assert_eq!(reply_json(&outcome)["reason"], "bad_pin");
    }

    #[test]
    fn a_wrong_pin_is_rejected_and_closes_after_three_attempts() {
        let mut session = PairingSession::open().unwrap();
        let client = TestClient::new(DEVICE, &wrong_pin(session.pin()), SERVER_ID);
        let offer = reply_json(&session.handle(SERVER_ID, &client.hello()));
        let derived = client.derive(&offer);

        let first = reply_json(&session.handle(SERVER_ID, &client.confirm(&derived)));
        let second = reply_json(&session.handle(SERVER_ID, &client.confirm(&derived)));
        let third = session.handle(SERVER_ID, &client.confirm(&derived));

        assert_eq!(first["reason"], "bad_pin");
        assert_eq!(first["attempts_left"], 2);
        assert_eq!(second["attempts_left"], 1);
        assert!(matches!(third, HandleOutcome::Closed(_)));
        assert_eq!(reply_json(&third)["attempts_left"], 0);
        assert!(first.get("sealed").is_none());
        assert!(first.get("server_confirm").is_none());
    }

    #[test]
    fn a_wrong_pin_never_receives_the_desktop_confirmation() {
        let mut session = PairingSession::open().unwrap();
        let client = TestClient::new(DEVICE, "000000", SERVER_ID);
        let offer = reply_json(&session.handle(SERVER_ID, &client.hello()));
        let derived = client.derive(&offer);

        let outcome = session.handle(SERVER_ID, &client.confirm(&derived));

        assert!(matches!(
            outcome,
            HandleOutcome::Reply(_) | HandleOutcome::Closed(_)
        ));
        assert_eq!(reply_json(&outcome)["reason"], "bad_pin");
    }

    #[test]
    fn a_confirm_without_a_prior_hello_is_refused_without_spending_an_attempt() {
        let mut session = PairingSession::open().unwrap();
        let client = TestClient::new(DEVICE, session.pin(), SERVER_ID);

        let outcome = session.handle(SERVER_ID, &client.confirm_bytes([0; 32]));

        assert_eq!(reply_json(&outcome)["reason"], "no_session");
        assert_eq!(reply_json(&outcome)["attempts_left"], MAX_ATTEMPTS);
    }

    #[test]
    fn a_non_pairing_datagram_is_ignored() {
        let mut session = PairingSession::open().unwrap();

        assert!(matches!(
            session.handle(SERVER_ID, b"DISCOVER"),
            HandleOutcome::Ignore
        ));
    }

    #[test]
    fn any_pairing_message_with_another_protocol_gets_a_protocol_error_that_costs_nothing() {
        let device = URL_SAFE_NO_PAD.encode(DEVICE);
        let cases = [
            serde_json::json!({
                "type": "PairHello",
                "device_id": device,
                "client_pub": URL_SAFE_NO_PAD.encode([5u8; 32]),
                "name": "OldPhone",
            }),
            serde_json::json!({"type": "PairHello", "protocol": 1, "device_id": device}),
            serde_json::json!({"type": "PairHello", "protocol": 3, "device_id": device}),
            serde_json::json!({
                "type": "PairConfirm",
                "protocol": 1,
                "device_id": device,
                "client_confirm": URL_SAFE_NO_PAD.encode([0u8; 32]),
            }),
            serde_json::json!({"type": "PairConfirm", "device_id": device}),
        ];
        let mut session = PairingSession::open().unwrap();
        for datagram in cases {
            let outcome = session.handle(SERVER_ID, &serde_json::to_vec(&datagram).unwrap());

            let reply = reply_json(&outcome);
            assert!(matches!(outcome, HandleOutcome::Reply(_)), "{datagram}");
            assert_eq!(reply["type"], "PairError", "{datagram}");
            assert_eq!(reply["reason"], "protocol", "{datagram}");
            assert_eq!(reply["expected"], 2, "{datagram}");
            assert_eq!(reply["attempts_left"], MAX_ATTEMPTS, "{datagram}");
        }
        assert!(session.pending.is_empty());
    }

    #[test]
    fn other_errors_do_not_carry_an_expected_protocol() {
        let mut session = PairingSession::open().unwrap();
        let client = TestClient::new(DEVICE, session.pin(), SERVER_ID);

        let reply = reply_json(&session.handle(SERVER_ID, &client.confirm_bytes([0; 32])));

        assert_eq!(reply["reason"], "no_session");
        assert!(reply.get("expected").is_none());
    }

    #[test]
    fn a_hello_with_a_malformed_or_small_order_share_is_a_bad_request() {
        let identity = {
            let mut bytes = [0u8; 32];
            bytes[0] = 1;
            bytes
        };
        let cases = [
            ("not base64", "!!!".to_string()),
            ("short", URL_SAFE_NO_PAD.encode([1u8; 31])),
            ("identity", URL_SAFE_NO_PAD.encode(identity)),
            ("order four", URL_SAFE_NO_PAD.encode([0u8; 32])),
        ];
        for (name, pa) in cases {
            let mut session = PairingSession::open().unwrap();
            let hello = serde_json::to_vec(&serde_json::json!({
                "type": "PairHello",
                "protocol": 2,
                "device_id": URL_SAFE_NO_PAD.encode(DEVICE),
                "pa": pa,
            }))
            .unwrap();

            let reply = reply_json(&session.handle(SERVER_ID, &hello));

            assert_eq!(reply["reason"], "bad_request", "{name}");
            assert!(session.pending.is_empty(), "{name}");
        }
    }

    #[test]
    fn the_offer_names_the_protocol_the_server_and_a_fresh_share() {
        let mut session = PairingSession::open().unwrap();
        let client = TestClient::new(DEVICE, session.pin(), SERVER_ID);

        let offer = reply_json(&session.handle(SERVER_ID, &client.hello()));

        assert_eq!(offer["type"], "PairOffer");
        assert_eq!(offer["protocol"], 2);
        assert_eq!(offer["server_id"], SERVER_ID);
        assert_eq!(decode32(offer["pb"].as_str().unwrap()).unwrap().len(), 32);
        assert_eq!(
            decode16(offer["salt"].as_str().unwrap()).unwrap(),
            session.salt
        );
    }
}
