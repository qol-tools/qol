use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Local};
use qol_config::pages::{Go, Notice, Press, PressReply, Stack};
use qol_peers::admin::{PageCursor, PointzRequest, PointzStatus, Response};
use qol_peers::pointz::{PointzSocket, PointzTransport};
use qol_plugin_daemon::daemon::ReadResult;
use serde::Serialize;

use crate::config::ServerConfig;
use crate::network;
use crate::qol::pages::{self, Act, Memory, Pairing, Phone, Serving, Snapshot, PAIR};

static MEMORY: Mutex<Memory> = Mutex::new(Memory::new());

pub(super) trait Tray {
    fn snapshot(&self) -> Snapshot;
    fn serve(&self, on: bool) -> bool;
    fn begin(&self) -> bool;
    fn cancel(&self);
    fn remove(&self, device: &str) -> bool;
}

pub(super) fn pages<C>(input: &serde_json::Value) -> ReadResult<C> {
    answer_pages(&Live, &mut memory(), input, Local::now())
}

pub(super) fn press<C>(input: &serde_json::Value) -> ReadResult<C> {
    answer_press(&Live, &mut memory(), input, Local::now())
}

fn memory() -> MutexGuard<'static, Memory> {
    MEMORY.lock().unwrap_or_else(|error| error.into_inner())
}

fn answer_pages<C>(
    tray: &impl Tray,
    memory: &mut Memory,
    input: &serde_json::Value,
    now: DateTime<Local>,
) -> ReadResult<C> {
    let open = if input.is_null() {
        Vec::new()
    } else {
        match serde_json::from_value::<Stack>(input.clone()) {
            Ok(stack) => stack.open,
            Err(error) => return ReadResult::Error(format!("pages needs the open pages: {error}")),
        }
    };
    let snapshot = current(tray, memory, &open);
    payload(&pages::build(&snapshot, memory, now).only(&open))
}

fn answer_press<C>(
    tray: &impl Tray,
    memory: &mut Memory,
    input: &serde_json::Value,
    now: DateTime<Local>,
) -> ReadResult<C> {
    let mut press: Press = match serde_json::from_value(input.clone()) {
        Ok(press) => press,
        Err(error) => return ReadResult::Error(format!("press needs a page and a row: {error}")),
    };
    let before = current(tray, memory, &press.open);
    let mut reply = PressReply::default();
    match pages::act(&press.page, &press.row, press.set) {
        Act::Serve(on) => {
            if !tray.serve(on) {
                let failure = if on {
                    "PointZerver could not start"
                } else {
                    "PointZerver could not stop"
                };
                reply.notice = Some(Notice::Failure(failure.into()));
            }
        }
        Act::Pair => {
            if before.serving == Serving::Off {
                tray.serve(true);
            }
            if tray.begin() {
                memory.began(&before.phones);
                reply.go = Some(Go::Open(PAIR.into()));
                press.open.push(PAIR.into());
            } else {
                reply.notice = Some(Notice::Failure(
                    "PointZerver could not start pairing".into(),
                ));
            }
        }
        Act::Stop => {
            tray.cancel();
            memory.stopped();
        }
        Act::Remove(id) => {
            if tray.remove(&id) {
                memory.removed(&before.phones, &id);
                reply.go = Some(Go::Back);
            } else {
                let name = pages::name_of(&before.phones, &id).unwrap_or("the phone");
                reply.notice = Some(Notice::Failure(format!(
                    "PointZerver could not remove {name}"
                )));
            }
        }
        Act::Nothing => {}
    }
    let after = current(tray, memory, &press.open);
    reply.pages = Some(pages::build(&after, memory, now).only(&press.open));
    payload(&reply)
}

fn current(tray: &impl Tray, memory: &mut Memory, open: &[String]) -> Snapshot {
    let mut snapshot = tray.snapshot();
    if memory.observe(&snapshot, open) {
        tray.cancel();
        snapshot.pairing = None;
    }
    snapshot
}

fn payload<C, T: Serialize>(value: &T) -> ReadResult<C> {
    match serde_json::to_value(value) {
        Ok(data) => ReadResult::HandledWithData(data),
        Err(error) => ReadResult::Error(format!("failed to encode the answer: {error}")),
    }
}

struct Live;

impl Tray for Live {
    fn snapshot(&self) -> Snapshot {
        let status = super::pairing::status();
        let phones = status.as_ref().map(phones).unwrap_or_default();
        Snapshot {
            serving: status.as_ref().map_or(Serving::Unknown, serving),
            pairing: status.as_ref().and_then(|status| {
                let code = status
                    .pairing
                    .code
                    .clone()
                    .filter(|_| status.pairing.open)?;
                Some(Pairing {
                    code,
                    seconds: status.pairing.seconds_remaining,
                })
            }),
            phones,
            hostname: network::get_hostname(),
            ip: network::get_local_ip().map(|ip| ip.to_string()),
        }
    }

    fn serve(&self, on: bool) -> bool {
        matches!(
            super::pairing::request(PointzRequest::Serve { on }),
            Some(Response::PointzStatus { .. })
        )
    }

    fn begin(&self) -> bool {
        matches!(
            super::pairing::request(PointzRequest::BeginPairing {}),
            Some(Response::PointzPairing { pairing }) if pairing.open
        )
    }

    fn cancel(&self) {
        super::pairing::request(PointzRequest::CancelPairing {});
    }

    fn remove(&self, device: &str) -> bool {
        let Ok(device_id) = device.parse() else {
            return false;
        };
        let Some(expected) = super::pairing::status().and_then(|status| status.authority) else {
            return false;
        };
        matches!(
            super::pairing::request(PointzRequest::Remove {
                expected,
                device_id,
            }),
            Some(Response::Changed { .. })
        )
    }
}

fn serving(status: &PointzStatus) -> Serving {
    let port = |socket| match socket {
        PointzSocket::Discovery => ServerConfig::DISCOVERY_PORT,
        PointzSocket::Command => ServerConfig::COMMAND_PORT,
    };
    match status.transport {
        PointzTransport::Running { .. } => Serving::Running,
        PointzTransport::Stopped {} => Serving::Off,
        PointzTransport::PortBusy { socket } => Serving::PortBusy(port(socket)),
        PointzTransport::Failed { socket } => Serving::Failed(port(socket)),
    }
}

fn phones(status: &PointzStatus) -> Vec<Phone> {
    let Some(expected) = status.authority else {
        return Vec::new();
    };
    let mut cursor = PageCursor {
        authority_id: expected.authority_id,
        activation_id: expected.activation_id,
        revision: expected.revision,
        offset: 0,
    };
    let mut phones = Vec::new();
    loop {
        let Some(Response::PointzDevices { page }) =
            super::pairing::request(PointzRequest::Devices { cursor })
        else {
            return phones;
        };
        phones.extend(page.items.into_iter().map(|device| Phone {
            id: device.device_id.to_string(),
            name: device.name,
            paired_at_ms: device.paired_at_ms,
        }));
        match page.next {
            Some(next) if next.offset > cursor.offset => cursor = next,
            _ => return phones,
        }
    }
}

#[cfg(test)]
mod tests;
