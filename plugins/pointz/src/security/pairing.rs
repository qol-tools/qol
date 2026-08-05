use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use x25519_dalek::{PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

pub const WINDOW_SECS: u64 = 60;
const MAX_ATTEMPTS: u32 = 3;
const MAX_PENDING: usize = 16;

const AUTH_INFO: &[u8] = b"pointz-pair-auth-v1";
const WRAP_INFO: &[u8] = b"pointz-pair-wrap-v1";
const CLIENT_CONFIRM: &[u8] = b"pointz-confirm-client";
const SERVER_CONFIRM: &[u8] = b"pointz-confirm-server";

#[derive(Deserialize)]
#[serde(tag = "type")]
enum WireRequest {
    #[serde(rename = "PairHello")]
    Hello {
        device_id: String,
        client_pub: String,
        #[serde(default)]
        name: Option<String>,
    },
    #[serde(rename = "PairConfirm")]
    Confirm {
        device_id: String,
        client_confirm: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum WireResponse<'a> {
    #[serde(rename = "PairOffer")]
    Offer {
        server_id: &'a str,
        server_pub: String,
        salt: String,
    },
    #[serde(rename = "PairResult")]
    Granted {
        server_confirm: String,
        wrap_nonce: String,
        sealed: String,
    },
    #[serde(rename = "PairError")]
    Denied { reason: &'a str, attempts_left: u32 },
}

struct Pending {
    k_auth: [u8; 32],
    k_wrap: [u8; 32],
    name: String,
}

pub struct PairingSession {
    pin: String,
    secret: StaticSecret,
    server_pub: [u8; 32],
    salt: [u8; 16],
    opened_at: Instant,
    attempts_left: u32,
    pending: HashMap<[u8; 16], Pending>,
}

pub enum HandleOutcome {
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
    pub fn open() -> Self {
        let secret = StaticSecret::from(random_bytes::<32>());
        let server_pub = PublicKey::from(&secret).to_bytes();
        Self {
            pin: generate_pin(),
            secret,
            server_pub,
            salt: random_bytes::<16>(),
            opened_at: Instant::now(),
            attempts_left: MAX_ATTEMPTS,
            pending: HashMap::new(),
        }
    }

    pub fn pin(&self) -> &str {
        &self.pin
    }

    pub fn is_expired(&self) -> bool {
        self.opened_at.elapsed() >= Duration::from_secs(WINDOW_SECS)
    }

    pub fn handle(&mut self, server_id: &str, datagram: &[u8]) -> HandleOutcome {
        let Ok(request) = serde_json::from_slice::<WireRequest>(datagram) else {
            return HandleOutcome::Ignore;
        };
        if self.is_expired() {
            return HandleOutcome::Closed(error(reason_closed(), 0));
        }
        match request {
            WireRequest::Hello {
                device_id,
                client_pub,
                name,
            } => self.on_hello(server_id, device_id, client_pub, name),
            WireRequest::Confirm {
                device_id,
                client_confirm,
            } => self.on_confirm(server_id, device_id, client_confirm),
        }
    }

    fn on_hello(
        &mut self,
        server_id: &str,
        device_id: String,
        client_pub: String,
        name: Option<String>,
    ) -> HandleOutcome {
        let (Some(device_id), Some(client_pub)) = (decode16(&device_id), decode32(&client_pub))
        else {
            return HandleOutcome::Reply(error("bad_request", self.attempts_left));
        };
        if self.pending.len() >= MAX_PENDING && !self.pending.contains_key(&device_id) {
            return HandleOutcome::Reply(error("busy", self.attempts_left));
        }
        let (k_auth, k_wrap) = derive_keys(
            &self.secret,
            &client_pub,
            &client_pub,
            &self.server_pub,
            &self.salt,
            &self.pin,
            &device_id,
        );
        self.pending.insert(
            device_id,
            Pending {
                k_auth,
                k_wrap,
                name: name.unwrap_or_default(),
            },
        );
        HandleOutcome::Reply(to_bytes(&WireResponse::Offer {
            server_id,
            server_pub: URL_SAFE_NO_PAD.encode(self.server_pub),
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
        if !confirm_matches(&pending.k_auth, CLIENT_CONFIRM, &client_confirm) {
            self.attempts_left = self.attempts_left.saturating_sub(1);
            let reply = error("bad_pin", self.attempts_left);
            return if self.attempts_left == 0 {
                HandleOutcome::Closed(reply)
            } else {
                HandleOutcome::Reply(reply)
            };
        }
        let device_key = random_bytes::<32>();
        let aad = confirm_aad(&device_id, server_id);
        let (wrap_nonce, sealed) = seal(&pending.k_wrap, &device_key, &aad);
        let reply = to_bytes(&WireResponse::Granted {
            server_confirm: URL_SAFE_NO_PAD.encode(confirm_tag(&pending.k_auth, SERVER_CONFIRM)),
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

fn derive_keys(
    own_secret: &StaticSecret,
    peer_pub: &[u8; 32],
    client_pub: &[u8; 32],
    server_pub: &[u8; 32],
    salt: &[u8; 16],
    pin: &str,
    device_id: &[u8; 16],
) -> ([u8; 32], [u8; 32]) {
    let shared = own_secret.diffie_hellman(&PublicKey::from(*peer_pub));
    let hk = Hkdf::<Sha256>::new(Some(salt), shared.as_bytes());
    let mut transcript = Vec::with_capacity(16 + 32 + 32 + pin.len());
    transcript.extend_from_slice(device_id);
    transcript.extend_from_slice(client_pub);
    transcript.extend_from_slice(server_pub);
    transcript.extend_from_slice(pin.as_bytes());
    (
        expand(&hk, AUTH_INFO, &transcript),
        expand(&hk, WRAP_INFO, &transcript),
    )
}

fn expand(hk: &Hkdf<Sha256>, label: &[u8], transcript: &[u8]) -> [u8; 32] {
    let mut info = Vec::with_capacity(label.len() + transcript.len());
    info.extend_from_slice(label);
    info.extend_from_slice(transcript);
    let mut okm = [0u8; 32];
    hk.expand(&info, &mut okm)
        .expect("32-byte OKM is valid for HKDF-SHA256");
    okm
}

fn confirm_tag(k_auth: &[u8; 32], label: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(k_auth).expect("HMAC accepts a 32-byte key");
    mac.update(label);
    mac.finalize().into_bytes().into()
}

fn confirm_matches(k_auth: &[u8; 32], label: &[u8], candidate: &[u8; 32]) -> bool {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(k_auth).expect("HMAC accepts a 32-byte key");
    mac.update(label);
    mac.verify_slice(candidate).is_ok()
}

fn confirm_aad(device_id: &[u8; 16], server_id: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(16 + server_id.len());
    aad.extend_from_slice(device_id);
    aad.extend_from_slice(server_id.as_bytes());
    aad
}

fn seal(k_wrap: &[u8; 32], device_key: &[u8; 32], aad: &[u8]) -> ([u8; 12], Vec<u8>) {
    let nonce_bytes = random_bytes::<12>();
    let cipher = ChaCha20Poly1305::new_from_slice(k_wrap).expect("ChaCha20Poly1305 32-byte key");
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: device_key,
                aad,
            },
        )
        .expect("AEAD sealing never fails for a bounded plaintext");
    (nonce_bytes, sealed)
}

fn generate_pin() -> String {
    loop {
        let value = u32::from_le_bytes(random_bytes::<4>());
        if value < 4_294_000_000 {
            return format!("{:06}", value % 1_000_000);
        }
    }
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the OS RNG is available");
    bytes
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

fn error(reason: &str, attempts_left: u32) -> Vec<u8> {
    to_bytes(&WireResponse::Denied {
        reason,
        attempts_left,
    })
}

fn reason_closed() -> &'static str {
    "pairing_closed"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER_ID: &str = "test-server-id01";
    const DEVICE: [u8; 16] = [4; 16];

    struct TestClient {
        secret: StaticSecret,
        public: [u8; 32],
    }

    impl TestClient {
        fn new() -> Self {
            let secret = StaticSecret::from(random_bytes::<32>());
            let public = PublicKey::from(&secret).to_bytes();
            Self { secret, public }
        }

        fn hello(&self) -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({
                "type": "PairHello",
                "device_id": URL_SAFE_NO_PAD.encode(DEVICE),
                "client_pub": URL_SAFE_NO_PAD.encode(self.public),
                "name": "TestPhone",
            }))
            .unwrap()
        }

        fn keys(&self, offer: &serde_json::Value, pin: &str) -> ([u8; 32], [u8; 32]) {
            let server_pub = decode32(offer["server_pub"].as_str().unwrap()).unwrap();
            let salt: [u8; 16] = URL_SAFE_NO_PAD
                .decode(offer["salt"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            derive_keys(
                &self.secret,
                &server_pub,
                &self.public,
                &server_pub,
                &salt,
                pin,
                &DEVICE,
            )
        }

        fn confirm(&self, k_auth: &[u8; 32]) -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({
                "type": "PairConfirm",
                "device_id": URL_SAFE_NO_PAD.encode(DEVICE),
                "client_confirm": URL_SAFE_NO_PAD.encode(confirm_tag(k_auth, CLIENT_CONFIRM)),
            }))
            .unwrap()
        }
    }

    fn reply_json(outcome: &HandleOutcome) -> serde_json::Value {
        let bytes = match outcome {
            HandleOutcome::Reply(bytes)
            | HandleOutcome::Closed(bytes)
            | HandleOutcome::Paired { reply: bytes, .. } => bytes,
            HandleOutcome::Ignore => panic!("expected a reply"),
        };
        serde_json::from_slice(bytes).unwrap()
    }

    #[test]
    fn a_correct_pin_pairs_and_delivers_a_decryptable_device_key() {
        let mut session = PairingSession::open();
        let pin = session.pin().to_string();
        let client = TestClient::new();

        let offer = reply_json(&session.handle(SERVER_ID, &client.hello()));
        let (k_auth, k_wrap) = client.keys(&offer, &pin);
        let outcome = session.handle(SERVER_ID, &client.confirm(&k_auth));

        let HandleOutcome::Paired {
            device_id,
            device_key,
            reply,
            ..
        } = outcome
        else {
            panic!("expected Paired");
        };
        assert_eq!(device_id, DEVICE);
        let result: serde_json::Value = serde_json::from_slice(&reply).unwrap();
        assert!(confirm_matches(
            &k_auth,
            SERVER_CONFIRM,
            &decode32(result["server_confirm"].as_str().unwrap()).unwrap(),
        ));
        let nonce: [u8; 12] = URL_SAFE_NO_PAD
            .decode(result["wrap_nonce"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let sealed = URL_SAFE_NO_PAD
            .decode(result["sealed"].as_str().unwrap())
            .unwrap();
        let cipher = ChaCha20Poly1305::new_from_slice(&k_wrap).unwrap();
        let opened = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &sealed,
                    aad: &confirm_aad(&DEVICE, SERVER_ID),
                },
            )
            .unwrap();
        assert_eq!(opened, device_key);
    }

    #[test]
    fn a_wrong_pin_is_rejected_and_closes_after_three_attempts() {
        let mut session = PairingSession::open();
        let real_pin = session.pin().to_string();
        let wrong_pin = format!("{:06}", (real_pin.parse::<u32>().unwrap() + 1) % 1_000_000);
        let client = TestClient::new();
        let offer = reply_json(&session.handle(SERVER_ID, &client.hello()));
        let (bad_auth, _) = client.keys(&offer, &wrong_pin);

        let first = reply_json(&session.handle(SERVER_ID, &client.confirm(&bad_auth)));
        let second = reply_json(&session.handle(SERVER_ID, &client.confirm(&bad_auth)));
        let third = session.handle(SERVER_ID, &client.confirm(&bad_auth));

        assert_eq!(first["reason"], "bad_pin");
        assert_eq!(first["attempts_left"], 2);
        assert_eq!(second["attempts_left"], 1);
        assert!(matches!(third, HandleOutcome::Closed(_)));
        assert_eq!(reply_json(&third)["attempts_left"], 0);
    }

    #[test]
    fn a_man_in_the_middle_without_the_pin_cannot_complete_confirmation() {
        let mut session = PairingSession::open();
        let client = TestClient::new();
        let offer = reply_json(&session.handle(SERVER_ID, &client.hello()));
        let (guessed_auth, _) = client.keys(&offer, "000000");

        let outcome = session.handle(SERVER_ID, &client.confirm(&guessed_auth));

        assert!(matches!(
            outcome,
            HandleOutcome::Reply(_) | HandleOutcome::Closed(_)
        ));
        assert_eq!(reply_json(&outcome)["reason"], "bad_pin");
    }

    #[test]
    fn a_confirm_without_a_prior_hello_is_refused_without_spending_an_attempt() {
        let mut session = PairingSession::open();
        let client = TestClient::new();

        let outcome = session.handle(SERVER_ID, &client.confirm(&[0; 32]));

        assert_eq!(reply_json(&outcome)["reason"], "no_session");
        assert_eq!(reply_json(&outcome)["attempts_left"], MAX_ATTEMPTS);
    }

    #[test]
    fn a_non_pairing_datagram_is_ignored() {
        let mut session = PairingSession::open();

        assert!(matches!(
            session.handle(SERVER_ID, b"DISCOVER"),
            HandleOutcome::Ignore
        ));
    }
}
