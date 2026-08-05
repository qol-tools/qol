mod pairing;
mod pairing_status;
mod registry;
mod replay;
mod secret;
mod wire;

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::command::Command;
use pairing::{HandleOutcome, PairingSession};
use registry::DeviceRegistry;
use replay::ReplayWindow;
use secret::ServerIdentity;
pub(crate) use secret::{ExistingSecretInspection, ExistingSecretState};

const COMMAND_CLOCK_SKEW_MS: u64 = 30_000;

pub struct CommandGate {
    identity: ServerIdentity,
    registry: Mutex<DeviceRegistry>,
    pairing: Mutex<Option<PairingSession>>,
    replay: Mutex<ReplayWindow>,
}

pub struct DiscoveryAuth {
    pub server_id: String,
    pub pairing_open: bool,
}

pub(crate) fn inspect_existing_secret() -> ExistingSecretInspection {
    ServerIdentity::inspect_existing()
}

pub fn pairing_status_json() -> serde_json::Value {
    let snapshot = pairing_status::current();
    serde_json::json!({
        "pairing_open": snapshot.pairing_open,
        "pin": snapshot.pin,
        "seconds_remaining": snapshot.seconds_remaining,
    })
}

impl CommandGate {
    pub fn load() -> Result<Self> {
        Ok(Self {
            identity: ServerIdentity::load_or_create()?,
            registry: Mutex::new(DeviceRegistry::load()?),
            pairing: Mutex::new(None),
            replay: Mutex::new(ReplayWindow::default()),
        })
    }

    pub fn begin_pairing(&self) {
        let Ok(mut pairing) = self.pairing.lock() else {
            return;
        };
        let session = PairingSession::open();
        let expires_at_ms = pairing_status::now_ms() + pairing::WINDOW_SECS * 1000;
        pairing_status::open(session.pin().to_string(), expires_at_ms);
        log::info!("PointZ pairing open for {} seconds", pairing::WINDOW_SECS);
        *pairing = Some(session);
    }

    pub fn discovery_auth(&self) -> DiscoveryAuth {
        DiscoveryAuth {
            server_id: self.identity.server_id(),
            pairing_open: pairing_status::current().pairing_open,
        }
    }

    pub fn handle_pairing(&self, datagram: &[u8]) -> Option<Vec<u8>> {
        let mut guard = self.pairing.lock().ok()?;
        let session = guard.as_mut()?;
        let server_id = self.identity.server_id();
        match session.handle(&server_id, datagram) {
            HandleOutcome::Ignore => None,
            HandleOutcome::Reply(reply) => Some(reply),
            HandleOutcome::Closed(reply) => {
                *guard = None;
                pairing_status::close();
                Some(reply)
            }
            HandleOutcome::Paired {
                device_id,
                device_key,
                name,
                reply,
            } => {
                if let Ok(mut registry) = self.registry.lock() {
                    if let Err(error) = registry.upsert(device_id, device_key, name) {
                        log::error!("PointZ failed to store the paired device: {error}");
                    }
                }
                *guard = None;
                pairing_status::close();
                Some(reply)
            }
        }
    }

    pub fn authenticate(&self, packet: &[u8]) -> Result<Command> {
        let parsed = wire::parse(packet)?;
        if parsed.sent_at_ms.abs_diff(unix_time_ms()) > COMMAND_CLOCK_SKEW_MS {
            anyhow::bail!("command timestamp is outside the accepted window");
        }
        let key = self
            .registry
            .lock()
            .map_err(|_| anyhow::anyhow!("device registry is unavailable"))?
            .key_for(&parsed.device_id)
            .ok_or_else(|| anyhow::anyhow!("command from an unpaired device"))?;
        let payload = parsed.verify(&key)?;
        let payload = payload.to_vec();
        let mut replay = self
            .replay
            .lock()
            .map_err(|_| anyhow::anyhow!("command replay state is unavailable"))?;
        if !replay.insert(&parsed.device_id, &parsed.nonce) {
            anyhow::bail!("command nonce was already used");
        }
        Ok(serde_json::from_slice(&payload)?)
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> CommandGate {
        let dir = tempfile::tempdir().unwrap().keep();
        CommandGate {
            identity: ServerIdentity::from_bytes([7; 32]),
            registry: Mutex::new(DeviceRegistry::load_at(dir.join("devices.json")).unwrap()),
            pairing: Mutex::new(None),
            replay: Mutex::new(ReplayWindow::default()),
        }
    }

    fn pair(gate: &CommandGate, device_id: [u8; 16], key: [u8; 32]) {
        gate.registry
            .lock()
            .unwrap()
            .upsert(device_id, key, "Phone".to_string())
            .unwrap();
    }

    #[test]
    fn discovery_auth_never_exposes_key_material() {
        let gate = gate();

        let auth = gate.discovery_auth();

        assert_eq!(
            auth.server_id,
            ServerIdentity::from_bytes([7; 32]).server_id()
        );
        assert!(!auth.pairing_open);
    }

    #[test]
    fn a_command_from_an_unpaired_device_is_rejected() {
        let gate = gate();
        let packet = wire::seal(
            br#"{"type":"MouseClick","button":1}"#,
            &[9; 32],
            [1; 16],
            unix_time_ms(),
            [2; 16],
        );

        assert!(gate.authenticate(&packet).is_err());
    }

    #[test]
    fn a_paired_device_command_is_accepted_once_then_replays_are_rejected() {
        let gate = gate();
        let device_id = [5; 16];
        let key = [6; 32];
        pair(&gate, device_id, key);
        let packet = wire::seal(
            br#"{"type":"MouseClick","button":1}"#,
            &key,
            device_id,
            unix_time_ms(),
            [3; 16],
        );

        let first = gate.authenticate(&packet);
        let second = gate.authenticate(&packet);

        assert!(matches!(first, Ok(Command::MouseClick { button: 1 })));
        assert!(second.is_err());
    }

    #[test]
    fn a_stale_command_is_rejected() {
        let gate = gate();
        let device_id = [5; 16];
        let key = [6; 32];
        pair(&gate, device_id, key);
        let stale = wire::seal(
            br#"{"type":"MouseClick","button":1}"#,
            &key,
            device_id,
            unix_time_ms() - COMMAND_CLOCK_SKEW_MS - 1,
            [7; 16],
        );

        assert!(gate.authenticate(&stale).is_err());
    }
}
