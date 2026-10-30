use std::path::Path;

use chrono::NaiveDateTime;

use crate::updates::{AutoKey, Failure, Holding, Host, Hours, Reason, Refusal, Update, Updates};

const UNSUPPORTED: &str = "Update Manager reads Linux Mint only so far";

pub(crate) struct Helper;

impl Helper {
    pub(crate) fn start() -> Result<Self, Refusal> {
        Err(refusal())
    }

    pub(crate) fn hold(&mut self) -> Result<Holding, Refusal> {
        Err(refusal())
    }

    pub(crate) fn release(&mut self) -> Result<(), Refusal> {
        Err(refusal())
    }
}

impl Drop for Helper {
    fn drop(&mut self) {}
}

fn refusal() -> Refusal {
    Refusal::Failed(UNSUPPORTED.into())
}

pub(crate) fn read_updates() -> Updates {
    Updates {
        list: Vec::new(),
        problems: vec![UNSUPPORTED.into()],
    }
}

pub(crate) fn read_host() -> Host {
    Host {
        problems: vec![UNSUPPORTED.into()],
        ..Host::default()
    }
}

pub(crate) fn install(
    _updates: &[Update],
    _log: &Path,
    _progress: &mut dyn FnMut(usize),
) -> Result<(), Failure> {
    Err(Failure {
        done: 0,
        reason: Reason::Stopped(UNSUPPORTED.into()),
    })
}

pub(crate) fn set_auto(_key: AutoKey, _on: bool) -> Result<(), Refusal> {
    Err(refusal())
}

pub(crate) fn set_hours(
    _hours: Option<Hours>,
    _startup_delay: u32,
    _restart: Option<NaiveDateTime>,
) -> Result<(), Refusal> {
    Err(refusal())
}

pub(crate) fn set_skips(_add: &[String], _remove: &[String], _system: bool) -> Result<(), Refusal> {
    Err(refusal())
}

pub(crate) fn schedule_restart(_at: Option<NaiveDateTime>) -> Result<(), Refusal> {
    Err(refusal())
}

pub(crate) fn restart_now() -> Result<(), Refusal> {
    Err(refusal())
}

pub(crate) fn supported() -> Result<(), &'static str> {
    Err(UNSUPPORTED)
}

pub(crate) fn is_mint() -> bool {
    false
}

pub(crate) fn missing(programs: &[&'static str]) -> Vec<&'static str> {
    programs.to_vec()
}

pub(crate) fn boot_id() -> Option<String> {
    None
}
