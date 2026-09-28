mod core;

#[cfg(test)]
mod tests;

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use qol_bluetooth_control::{now_ms, HoldOwner, Holds};
use serde_json::{json, Value};

use crate::bluetooth::{connection_ready, normalize_address, DeviceInfo};
use crate::config::ReconnectConfig;

use self::core::CoreRemote;

const HANDOFF_HOLD: Duration = Duration::from_secs(12 * 60 * 60);
const RELEASE_SETTLE: Duration = Duration::from_secs(5);
const RELEASE_POLL: Duration = Duration::from_millis(250);
const LOCAL_ATTEMPTS: u32 = 3;
const LOCAL_RETRY_DELAY: Duration = Duration::from_millis(1500);
const MAX_PEERS: usize = 16;

pub(crate) struct Peer {
    pub id: String,
    pub name: String,
}

#[derive(Debug)]
pub(crate) enum RemoteError {
    Refused(String),
    Unknown,
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(reason) => formatter.write_str(reason),
            Self::Unknown => formatter.write_str("the result is unknown"),
        }
    }
}

pub(crate) trait Remote {
    fn peers(&self) -> Result<Vec<Peer>>;
    fn call(&self, peer: &Peer, operation: Operation, address: &str) -> Result<Value, RemoteError>;
}

pub(crate) trait Local {
    fn device(&self, address: &str) -> Result<Option<DeviceInfo>>;
    fn connect(&self, address: &str) -> Result<DeviceInfo>;
    fn pause(&self, delay: Duration);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Operation {
    State,
    Release,
    Resume,
}

impl Operation {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::State => "handoff_state",
            Self::Release => "release_for_handoff",
            Self::Resume => "resume_reconnect",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Moved {
    AlreadyHere,
    Connected { from: Option<String> },
}

pub(crate) fn take_over(address: &str, remote: &impl Remote, local: &impl Local) -> Result<Moved> {
    let address = normalize_address(address)?;
    let device = local
        .device(&address)?
        .ok_or_else(|| anyhow!("{address} is not known to this computer"))?;
    if !device.paired {
        bail!("{} is not paired with this computer", device.alias);
    }
    if connection_ready(&device) {
        return Ok(Moved::AlreadyHere);
    }
    let peers = remote
        .peers()
        .map_err(|error| anyhow!("linked computers are unavailable: {error:#}"))?;
    let mut unreachable = Vec::new();
    let mut holder = None;
    for peer in peers.iter().take(MAX_PEERS) {
        match remote.call(peer, Operation::State, &address) {
            Ok(state) if state["connected"] == true => {
                holder = Some(peer);
                break;
            }
            Ok(_) => {}
            Err(error) => unreachable.push(format!("{} ({error})", peer.name)),
        }
    }
    let Some(peer) = holder else {
        return connect_here(&address, local)
            .map(|_| Moved::Connected { from: None })
            .map_err(|error| match unreachable.is_empty() {
                true => error,
                false => error.context(format!(
                    "no reachable linked computer has {}; not asked: {}",
                    device.alias,
                    unreachable.join(", ")
                )),
            });
    };
    match remote.call(peer, Operation::Release, &address) {
        Ok(result) if result["released"] == true => {}
        Ok(_) => bail!(
            "{} did not confirm that it let go of {}",
            peer.name,
            device.alias
        ),
        Err(RemoteError::Unknown) => bail!(
            "{} may have let go of {}, but the result is unknown; check it before trying again",
            peer.name,
            device.alias
        ),
        Err(error) => bail!(
            "{} refused to let go of {}: {error}",
            peer.name,
            device.alias
        ),
    }
    match connect_here(&address, local) {
        Ok(_) => Ok(Moved::Connected {
            from: Some(peer.name.clone()),
        }),
        Err(error) => {
            let resumed = remote.call(peer, Operation::Resume, &address).is_ok();
            let next = if resumed {
                format!("{} can reconnect them again", peer.name)
            } else {
                format!("{} could not be told to reconnect them", peer.name)
            };
            Err(error.context(format!(
                "{} let go of {}, but this computer could not connect; {next}",
                peer.name, device.alias
            )))
        }
    }
}

fn connect_here(address: &str, local: &impl Local) -> Result<DeviceInfo> {
    let mut failure = None;
    for attempt in 0..LOCAL_ATTEMPTS {
        if attempt > 0 {
            local.pause(LOCAL_RETRY_DELAY);
        }
        match local.connect(address) {
            Ok(device) if connection_ready(&device) => return Ok(device),
            Ok(device) => {
                failure = Some(anyhow!(
                    "{} connected but its audio is not ready",
                    device.alias
                ))
            }
            Err(error) => failure = Some(error),
        }
    }
    Err(failure.unwrap_or_else(|| anyhow!("the connection was never attempted")))
}

pub(crate) struct PlatformLocal {
    pub power_on_adapter: bool,
}

impl Local for PlatformLocal {
    fn device(&self, address: &str) -> Result<Option<DeviceInfo>> {
        Ok(crate::platform::list_devices()?
            .into_iter()
            .find(|device| device.address == address))
    }

    fn connect(&self, address: &str) -> Result<DeviceInfo> {
        connect_for_user(address, self.power_on_adapter)
    }

    fn pause(&self, delay: Duration) {
        std::thread::sleep(delay);
    }
}

pub(crate) fn run_take_over(address: &str, power_on_adapter: bool) -> Result<Moved> {
    let result = take_over(
        address,
        &CoreRemote::from_env(),
        &PlatformLocal { power_on_adapter },
    );
    let outcome = match &result {
        Ok(Moved::AlreadyHere) => "already_here",
        Ok(Moved::Connected { from: Some(_) }) => "moved",
        Ok(Moved::Connected { from: None }) => "connected",
        Err(_) => "failed",
    };
    qol_runtime::probe!("BLUETOOTH_HANDOFF", "event=take_over outcome={outcome}");
    result
}

pub(crate) fn message(result: &Result<Moved>) -> String {
    match result {
        Ok(Moved::AlreadyHere) => "Already connected here with audio ready".into(),
        Ok(Moved::Connected { from: Some(from) }) => {
            format!("Moved here from {from}; audio is ready")
        }
        Ok(Moved::Connected { from: None }) => {
            "No linked computer had it; connected here with audio ready".into()
        }
        Err(error) => format!("Could not move it here: {error:#}"),
    }
}

pub(crate) fn held(address: &str) -> bool {
    Holds::shared()
        .and_then(|holds| holds.active(address, now_ms()).ok().flatten())
        .is_some()
}

pub(crate) fn release_for_user(address: &str) {
    let Some(holds) = Holds::shared() else {
        return;
    };
    if let Err(error) = holds.release_address(address, now_ms()) {
        qol_runtime::probe!(
            "BLUETOOTH_HANDOFF",
            "event=user_release outcome=failed kind={:?}",
            error.kind()
        );
    }
}

pub(crate) fn connect_for_user(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    release_for_user(address);
    crate::platform::connect_device(address, power_on_adapter)
}

pub(crate) fn release_managed_for_user(config: &ReconnectConfig) {
    for address in &config.managed_devices {
        release_for_user(address);
    }
}

pub(crate) fn state(address: &str, devices: &[DeviceInfo]) -> Value {
    let device = devices.iter().find(|device| device.address == address);
    json!({
        "known": device.is_some(),
        "connected": device.is_some_and(|device| device.connected),
        "held": held(address),
    })
}

pub(crate) fn release(
    address: &str,
    list: impl Fn() -> Result<Vec<DeviceInfo>>,
    disconnect: impl FnOnce(&str) -> Result<DeviceInfo>,
    pause: impl Fn(Duration),
) -> Result<Value> {
    let address = normalize_address(address)?;
    let holds =
        Holds::shared().ok_or_else(|| anyhow!("the QoL runtime directory is unavailable"))?;
    let devices = list()?;
    let device = devices
        .iter()
        .find(|device| device.address == address)
        .ok_or_else(|| anyhow!("{address} is not known to this computer"))?;
    let was_connected = device.connected;
    let now = now_ms();
    holds.place(
        &address,
        HoldOwner::Handoff,
        now.saturating_add(HANDOFF_HOLD.as_millis() as u64),
        now,
    )?;
    let released = (|| {
        if was_connected {
            disconnect(&address)?;
        }
        let deadline = std::time::Instant::now() + RELEASE_SETTLE;
        loop {
            let connected = list()?
                .iter()
                .any(|device| device.address == address && device.connected);
            if !connected {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                bail!("{address} is still connected here");
            }
            pause(RELEASE_POLL);
        }
    })();
    if let Err(error) = released {
        let _ = holds.release(&address, HoldOwner::Handoff, now_ms());
        qol_runtime::probe!("BLUETOOTH_HANDOFF", "event=release outcome=failed");
        return Err(error);
    }
    qol_runtime::probe!(
        "BLUETOOTH_HANDOFF",
        "event=release outcome=released was_connected={was_connected}"
    );
    Ok(json!({ "released": true, "was_connected": was_connected }))
}

pub(crate) fn resume(address: &str) -> Result<Value> {
    let address = normalize_address(address)?;
    let holds =
        Holds::shared().ok_or_else(|| anyhow!("the QoL runtime directory is unavailable"))?;
    let resumed = holds.release(&address, HoldOwner::Handoff, now_ms())?;
    qol_runtime::probe!("BLUETOOTH_HANDOFF", "event=resume resumed={resumed}");
    Ok(json!({ "resumed": resumed }))
}
