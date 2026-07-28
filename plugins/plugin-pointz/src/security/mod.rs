mod replay;
mod secret;
mod wire;

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::command::Command;
use replay::ReplayWindow;
use secret::PairingSecret;

const PAIRING_WINDOW: Duration = Duration::from_secs(120);
const COMMAND_CLOCK_SKEW_MS: u64 = 30_000;

pub struct CommandGate {
    secret: PairingSecret,
    pairing_until: Mutex<Option<Instant>>,
    replay: Mutex<ReplayWindow>,
}

pub struct DiscoveryAuth {
    pub server_id: String,
    pub pairing_secret: Option<String>,
}

impl CommandGate {
    pub fn load() -> Result<Self> {
        Ok(Self::new(PairingSecret::load_or_create()?))
    }

    fn new(secret: PairingSecret) -> Self {
        Self {
            secret,
            pairing_until: Mutex::new(None),
            replay: Mutex::new(ReplayWindow::default()),
        }
    }

    pub fn begin_pairing(&self) {
        let Ok(mut pairing_until) = self.pairing_until.lock() else {
            return;
        };
        *pairing_until = Some(Instant::now() + PAIRING_WINDOW);
    }

    pub fn discovery_auth(&self) -> DiscoveryAuth {
        let pairing_secret = self.pairing_until.lock().ok().and_then(|mut until| {
            let is_active = until.is_some_and(|deadline| deadline > Instant::now());
            *until = None;
            is_active.then(|| self.secret.encoded())
        });
        DiscoveryAuth {
            server_id: self.secret.server_id(),
            pairing_secret,
        }
    }

    pub fn authenticate(&self, packet: &[u8]) -> Result<Command> {
        let verified = wire::verify(packet, self.secret.bytes())?;
        if verified.sent_at_ms.abs_diff(unix_time_ms()) > COMMAND_CLOCK_SKEW_MS {
            anyhow::bail!("command timestamp is outside the accepted window");
        }
        let mut replay = self
            .replay
            .lock()
            .map_err(|_| anyhow::anyhow!("command replay state is unavailable"))?;
        if !replay.insert(verified.nonce) {
            anyhow::bail!("command nonce was already used");
        }
        Ok(serde_json::from_slice(&verified.payload)?)
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
        CommandGate::new(PairingSecret::from_bytes([7; 32]))
    }

    #[test]
    fn pairing_secret_is_only_visible_during_pairing_window() {
        let gate = gate();
        assert!(gate.discovery_auth().pairing_secret.is_none());

        gate.begin_pairing();

        assert!(gate.discovery_auth().pairing_secret.is_some());
        assert!(gate.discovery_auth().pairing_secret.is_none());
    }

    #[test]
    fn authentic_packet_is_accepted_once() {
        let gate = gate();
        let payload = br#"{"type":"MouseClick","button":1}"#;
        let packet = wire::seal(payload, gate.secret.bytes(), unix_time_ms(), [9; 16]);

        let first = gate.authenticate(&packet);
        let second = gate.authenticate(&packet);

        assert!(matches!(first, Ok(Command::MouseClick { button: 1 })));
        assert!(second.is_err());
    }

    #[test]
    fn stale_and_tampered_packets_are_rejected() {
        let gate = gate();
        let payload = br#"{"type":"MouseClick","button":1}"#;
        let stale = wire::seal(
            payload,
            gate.secret.bytes(),
            unix_time_ms() - COMMAND_CLOCK_SKEW_MS - 1,
            [1; 16],
        );
        let mut tampered = wire::seal(payload, gate.secret.bytes(), unix_time_ms(), [2; 16]);
        let last = tampered.len() - 2;
        tampered[last] = if tampered[last] == b'a' { b'b' } else { b'a' };

        assert!(gate.authenticate(&stale).is_err());
        assert!(gate.authenticate(&tampered).is_err());
    }
}
