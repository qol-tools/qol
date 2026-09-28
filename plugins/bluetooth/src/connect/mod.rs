#[cfg(test)]
mod tests;

use anyhow::{anyhow, Result};

use crate::bluetooth::DeviceInfo;

pub(crate) enum Attempt {
    Connected(DeviceInfo),
    Skipped(Option<String>),
    Failed(anyhow::Error),
}

pub(crate) trait Route {
    fn attempt(&self, address: &str) -> Attempt;
}

pub(crate) struct Direct<F>(pub F);

impl<F: Fn(&str) -> Result<DeviceInfo>> Route for Direct<F> {
    fn attempt(&self, address: &str) -> Attempt {
        match (self.0)(address) {
            Ok(device) => Attempt::Connected(device),
            Err(error) => Attempt::Failed(error),
        }
    }
}

pub(crate) fn connect(address: &str, routes: &[&dyn Route]) -> Result<DeviceInfo> {
    let mut failure = None;
    let mut notes = Vec::new();
    for route in routes {
        match route.attempt(address) {
            Attempt::Connected(device) => return Ok(device),
            Attempt::Skipped(note) => notes.extend(note),
            Attempt::Failed(error) => failure = Some(error),
        }
    }
    let error = failure.unwrap_or_else(|| anyhow!("nothing could connect {address}"));
    Err(notes
        .into_iter()
        .fold(error, |error, note| error.context(note)))
}

pub(crate) fn for_user(
    address: &str,
    power_on_adapter: bool,
    direct: impl Fn(&str) -> Result<DeviceInfo>,
) -> Result<DeviceInfo> {
    crate::handoff::release_for_user(address);
    let handoff = crate::handoff::Handoff::from_env(power_on_adapter);
    connect(address, &[&Direct(direct), &handoff])
}
