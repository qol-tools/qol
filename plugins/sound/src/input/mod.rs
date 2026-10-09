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

pub fn status() -> anyhow::Result<InputStatus> {
    let inspection = crate::config::inspect().context("cannot read the saved microphone")?;
    device::status(INPUT, inspection.config.input.device)
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
    if let Err(error) = crate::config::save_input_device(applied.as_str()) {
        anyhow::bail!(
            "switched to the microphone `{applied}`, but the choice was not saved: {error:#}"
        );
    }
    Ok(Some(applied))
}
