use anyhow::Context;

use qol_audio::devices::Identity;

use crate::device::{self, INPUT};

pub use crate::device::{DeviceRow as InputRow, DeviceStatus as InputStatus};

pub fn list() -> anyhow::Result<Vec<InputRow>> {
    device::list(INPUT)
}

pub fn switch(input: &str) -> anyhow::Result<()> {
    device::switch(INPUT, input).map(|_| ())
}

pub fn status(saved: String) -> anyhow::Result<InputStatus> {
    device::status(INPUT, saved)
}

pub fn companion(output: &Identity) -> anyhow::Result<Option<Identity>> {
    qol_audio::devices::companion_input(output)
        .context("cannot look up the microphone on the same device as the sound output")
}

pub fn follow(output: &Identity) -> anyhow::Result<Option<Identity>> {
    let Some(companion) = companion(output)? else {
        return Ok(None);
    };
    let applied = device::switch(INPUT, companion.as_str())?;
    Ok(Some(applied))
}
